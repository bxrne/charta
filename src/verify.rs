//! Formal verification of SCXML state charts via Z3.
//!
//! ## What this module does
//!
//! Given an SCXML document and a chosen tool ([`VerificationTool::Smt`] or
//! [`VerificationTool::KInduction`]), this module:
//!
//! 1. Extracts user-declared invariants from XML comment pragmas of the form
//!    `<!-- @invariant id="NAME" expr="PROP" -->`.
//! 2. Lowers the state chart into a flat Kripke transition system.
//! 3. Encodes the transition system as Z3 constraints over `BMC_BOUND` (or
//!    `KIND_MAX_K`) symbolic time steps.
//! 4. For each invariant, asks Z3 either "is there a counterexample within
//!    the bound" (BMC) or "is the invariant inductive" (k-induction).
//! 5. Returns a [`Verdict`] per invariant with a human-readable rendering.
//!
//! ## v1 limitations (intentional, documented in the tool description)
//!
//! * Flat charts only — `<state>` nesting, `<parallel>`, and `<history>` are
//!   rejected with an `Unsupported` error.
//! * Transition guards (`cond=`) and data model expressions are *not*
//!   interpreted. Guards are treated as nondeterministic, which gives a
//!   sound over-approximation for safety: any property that holds in this
//!   model also holds in the real chart, but counterexamples may be spurious
//!   if they rely on a guard-pattern that the real datamodel forbids.
//! * Events are modelled as an opaque alphabet; `<send delay=…>` timing is
//!   ignored.
//! * Property language is `in('STATE')`, `not`, `and`, `or`, `=>`, `<=>`,
//!   `true`, `false`, parens.
//!
//! ## Property pragma syntax
//!
//! Invariants are declared as XML comments so they survive any conformant
//! SCXML parser without breaking validation:
//!
//! ```xml
//! <!-- @invariant id="floor_nonneg" expr="not in('crashed')" -->
//! <!-- @invariant id="exclusive_doors" expr="not (in('open') and in('closing'))" -->
//! ```

use std::collections::BTreeMap;

use scxml::{Statechart, StateKind, parse_xml};
use z3::{
    SatResult, Solver,
    ast::{Bool, Int},
};

use crate::tools::{ToolError, VerificationTool};

/// BMC unrolling depth. Larger = more thorough, slower.
const BMC_BOUND: usize = 12;
/// Maximum `k` to try when searching for an inductive invariant.
const KIND_MAX_K: usize = 8;

/// Public entry point invoked by the MCP `verify_state_chart` tool.
///
/// Returns one [`Verdict`] per declared invariant. An empty pragma set
/// produces a single advisory verdict explaining how to declare one — that
/// is more useful to an LLM caller than a silent empty success.
pub fn verify(xml: &str, tool: VerificationTool) -> Result<Vec<Verdict>, ToolError> {
    let pragmas = extract_pragmas(xml);

    // We re-parse here even though the router already validated the XML; the
    // module is self-contained and `parse_xml` is cheap relative to Z3.
    let chart = parse_xml(xml).map_err(ToolError::Parse)?;
    let ts = TransitionSystem::from_chart(&chart)?;

    if pragmas.is_empty() {
        return Ok(vec![Verdict {
            property: "<none>".into(),
            method: tool,
            outcome: Outcome::Unknown {
                reason: "no `<!-- @invariant id=\"...\" expr=\"...\" -->` pragmas found in document".into(),
            },
        }]);
    }

    let mut verdicts = Vec::with_capacity(pragmas.len());
    for (id, src) in pragmas {
        // Per-property: parse the expression, then dispatch to the chosen
        // backend. Property-parse failures surface as typed errors (the
        // entire batch fails fast) because they indicate a malformed input.
        let prop = parse_property(&src).map_err(|e| {
            ToolError::PropertyParse(format!("invariant `{id}`: {e} (in `{src}`)"))
        })?;
        let outcome = match tool {
            VerificationTool::Smt => bmc(&ts, &prop, BMC_BOUND),
            VerificationTool::KInduction => k_induction(&ts, &prop, KIND_MAX_K),
        };
        verdicts.push(Verdict {
            property: id,
            method: tool,
            outcome,
        });
    }
    Ok(verdicts)
}

// ───────────────────────── Verdicts ─────────────────────────

/// Per-property result returned to the caller.
#[derive(Debug)]
pub struct Verdict {
    /// Pragma `id` (or `"<none>"` for the no-invariants advisory).
    pub property: String,
    /// Which backend produced this verdict.
    pub method: VerificationTool,
    /// What the backend concluded.
    pub outcome: Outcome,
}

/// The four answers Z3 can give us, projected onto something a human / LLM
/// can act on without parsing solver output.
#[derive(Debug)]
pub enum Outcome {
    /// k-induction proved the property is inductive at depth `k`.
    Holds { k: usize },
    /// BMC unrolled to `bound` steps without finding a counterexample.
    /// **Not** a proof — a violation may exist at greater depth.
    BoundedSafe { bound: usize },
    /// A concrete trace that violates the property.
    Violated { trace: Vec<Step> },
    /// Z3 returned `unknown`, or k-induction hit its cap without converging.
    Unknown { reason: String },
}

/// One slice of a counterexample trace.
#[derive(Debug)]
pub struct Step {
    /// The leaf state that is active at this step.
    pub state: String,
    /// The event that fires *out of* this step (None for the final step,
    /// where no further transition is taken).
    pub event: Option<String>,
}

impl Verdict {
    /// Render the verdict as a human-readable text block, suitable for
    /// embedding in an MCP `Content::text` payload.
    pub fn render(&self) -> String {
        let method = match self.method {
            VerificationTool::Smt => "bmc",
            VerificationTool::KInduction => "k-induction",
        };
        let mut out = format!("invariant `{}` [{}]\n", self.property, method);
        match &self.outcome {
            Outcome::Holds { k } => {
                out.push_str(&format!("HOLDS — proved inductive at k={k}\n"));
            }
            Outcome::BoundedSafe { bound } => {
                out.push_str(&format!(
                    "BOUNDED-SAFE — no counterexample within {bound} steps (not a proof)\n"
                ));
            }
            Outcome::Violated { trace } => {
                out.push_str("VIOLATED — counterexample trace:\n");
                for (i, step) in trace.iter().enumerate() {
                    let evt = step.event.as_deref().unwrap_or("·");
                    out.push_str(&format!("  step {i:>2}: state={} event={}\n", step.state, evt));
                }
            }
            Outcome::Unknown { reason } => {
                out.push_str(&format!("UNKNOWN — {reason}\n"));
            }
        }
        out
    }
}

// ───────────────────────── Pragma extraction ─────────────────────────

/// Walk the raw XML for `<!-- @invariant id="..." expr="..." -->` comments
/// and return `(id, expr)` pairs in document order.
///
/// Implemented as a hand-rolled scanner rather than pulling in a regex crate
/// — the pattern is shallow enough that the extra dependency isn't worth it.
fn extract_pragmas(xml: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<!--") {
        // Slice from <!-- onward; bail if the comment is unterminated.
        let after_open = &rest[start + 4..];
        let Some(end) = after_open.find("-->") else {
            break;
        };
        let body = after_open[..end].trim();
        // Advance the cursor past this comment regardless of whether we use it.
        rest = &after_open[end + 3..];
        if let Some(after_tag) = body.strip_prefix("@invariant") {
            // Need a space (or other whitespace) right after the tag so that
            // e.g. `@invariantfoo` doesn't match.
            if !after_tag.starts_with(|c: char| c.is_whitespace()) {
                continue;
            }
            let attrs = after_tag.trim();
            if let (Some(id), Some(expr)) = (extract_attr(attrs, "id"), extract_attr(attrs, "expr"))
            {
                out.push((id, expr));
            }
        }
    }
    out
}

/// Pull `name="value"` out of a key=value attribute string. Naive (no
/// escape handling), which is acceptable for hand-written pragmas.
fn extract_attr(s: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let i = s.find(&key)?;
    let rest = &s[i + key.len()..];
    let j = rest.find('"')?;
    Some(rest[..j].to_string())
}

// ───────────────────────── Property AST + parser ─────────────────────────

/// Boolean property AST. Atoms are state-membership predicates; the rest is
/// standard propositional logic.
#[derive(Debug, Clone)]
enum Prop {
    True,
    False,
    /// `in('STATE')` — true iff the named leaf state is active.
    In(String),
    Not(Box<Prop>),
    And(Box<Prop>, Box<Prop>),
    Or(Box<Prop>, Box<Prop>),
    Implies(Box<Prop>, Box<Prop>),
    Iff(Box<Prop>, Box<Prop>),
}

/// Tokeniser for the tiny property language. A `Vec<Token>` keeps the parser
/// readable without any third-party dep.
#[derive(Debug, Clone, PartialEq)]
enum Tok {
    LParen,
    RParen,
    Comma,
    Not,
    And,
    Or,
    Implies,
    Iff,
    True,
    False,
    In,
    Ident(String),
    Str(String),
}

fn tokenize(s: &str) -> Result<Vec<Tok>, String> {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            c if c.is_whitespace() => i += 1,
            '(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            ',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            // `=>` for implication; otherwise we have no use for `=` so flag it.
            '=' if i + 1 < bytes.len() && bytes[i + 1] as char == '>' => {
                out.push(Tok::Implies);
                i += 2;
            }
            // `<=>` for biconditional.
            '<' if i + 2 < bytes.len()
                && bytes[i + 1] as char == '='
                && bytes[i + 2] as char == '>' =>
            {
                out.push(Tok::Iff);
                i += 3;
            }
            // String literal: `'...'` or `"..."`. No escapes — state ids
            // can't contain them in practice.
            '\'' | '"' => {
                let quote = c;
                let start = i + 1;
                i += 1;
                while i < bytes.len() && bytes[i] as char != quote {
                    i += 1;
                }
                if i >= bytes.len() {
                    return Err(format!("unterminated string literal starting at offset {start}"));
                }
                let s = std::str::from_utf8(&bytes[start..i])
                    .map_err(|e| format!("invalid utf-8 in string: {e}"))?;
                out.push(Tok::Str(s.to_string()));
                i += 1; // skip closing quote
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < bytes.len()
                    && (bytes[i] as char).is_ascii_alphanumeric()
                    || (i < bytes.len() && bytes[i] as char == '_')
                {
                    i += 1;
                }
                let word = &s[start..i];
                let tok = match word {
                    "not" => Tok::Not,
                    "and" => Tok::And,
                    "or" => Tok::Or,
                    "true" => Tok::True,
                    "false" => Tok::False,
                    "in" => Tok::In,
                    other => Tok::Ident(other.to_string()),
                };
                out.push(tok);
            }
            other => return Err(format!("unexpected character `{other}` at offset {i}")),
        }
    }
    Ok(out)
}

/// Recursive-descent parser. Precedence (loosest → tightest):
/// `<=>`, `=>`, `or`, `and`, `not`, primary.
fn parse_property(src: &str) -> Result<Prop, String> {
    let toks = tokenize(src)?;
    let mut p = Parser { toks, pos: 0 };
    let prop = p.parse_iff()?;
    if p.pos != p.toks.len() {
        return Err(format!("unexpected trailing tokens at position {}", p.pos));
    }
    Ok(prop)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == Some(t) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse_iff(&mut self) -> Result<Prop, String> {
        let lhs = self.parse_implies()?;
        if self.eat(&Tok::Iff) {
            let rhs = self.parse_iff()?;
            Ok(Prop::Iff(Box::new(lhs), Box::new(rhs)))
        } else {
            Ok(lhs)
        }
    }

    fn parse_implies(&mut self) -> Result<Prop, String> {
        let lhs = self.parse_or()?;
        if self.eat(&Tok::Implies) {
            // Right-associative implication.
            let rhs = self.parse_implies()?;
            Ok(Prop::Implies(Box::new(lhs), Box::new(rhs)))
        } else {
            Ok(lhs)
        }
    }

    fn parse_or(&mut self) -> Result<Prop, String> {
        let mut lhs = self.parse_and()?;
        while self.eat(&Tok::Or) {
            let rhs = self.parse_and()?;
            lhs = Prop::Or(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<Prop, String> {
        let mut lhs = self.parse_unary()?;
        while self.eat(&Tok::And) {
            let rhs = self.parse_unary()?;
            lhs = Prop::And(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Prop, String> {
        if self.eat(&Tok::Not) {
            let inner = self.parse_unary()?;
            Ok(Prop::Not(Box::new(inner)))
        } else {
            self.parse_primary()
        }
    }

    fn parse_primary(&mut self) -> Result<Prop, String> {
        match self.peek().cloned() {
            Some(Tok::True) => {
                self.pos += 1;
                Ok(Prop::True)
            }
            Some(Tok::False) => {
                self.pos += 1;
                Ok(Prop::False)
            }
            Some(Tok::LParen) => {
                self.pos += 1;
                let inner = self.parse_iff()?;
                if !self.eat(&Tok::RParen) {
                    return Err("expected `)`".into());
                }
                Ok(inner)
            }
            Some(Tok::In) => {
                self.pos += 1;
                if !self.eat(&Tok::LParen) {
                    return Err("expected `(` after `in`".into());
                }
                let id = match self.peek().cloned() {
                    Some(Tok::Str(s)) => {
                        self.pos += 1;
                        s
                    }
                    Some(Tok::Ident(s)) => {
                        self.pos += 1;
                        s
                    }
                    other => return Err(format!("expected state id in `in(...)`, got {other:?}")),
                };
                if !self.eat(&Tok::RParen) {
                    return Err("expected `)` to close `in(...)`".into());
                }
                Ok(Prop::In(id))
            }
            other => Err(format!("unexpected token {other:?} in primary position")),
        }
    }
}

// ───────────────────────── Transition system ─────────────────────────

/// Flattened Kripke structure derived from a chart.
///
/// Hierarchy is rejected up front so that "state" can mean exactly one of
/// `states`, with no need to track active configurations or LCAs.
#[derive(Debug)]
struct TransitionSystem {
    /// Leaf state ids in deterministic (parse) order.
    states: Vec<String>,
    /// Initial state id (one of `states`).
    initial: String,
    /// Distinct event names; eventless transitions get the synthetic name
    /// `"ε"` at index 0.
    events: Vec<String>,
    /// `(source_idx, event_idx, target_idx)` triples — one per transition.
    edges: Vec<(usize, usize, usize)>,
}

impl TransitionSystem {
    fn from_chart(chart: &Statechart) -> Result<Self, ToolError> {
        let mut states = Vec::new();
        for s in chart.iter_all_states() {
            match s.kind {
                StateKind::Atomic | StateKind::Final => states.push(s.id.to_string()),
                _ => {
                    return Err(ToolError::Unsupported(format!(
                        "state `{}` has kind {:?}; v1 supports only flat charts (Atomic/Final)",
                        s.id, s.kind
                    )));
                }
            }
        }
        if states.is_empty() {
            return Err(ToolError::Unsupported("chart has no leaf states".into()));
        }

        // O(1) id → index lookup for transition resolution.
        let idx_of: BTreeMap<&str, usize> = states
            .iter()
            .enumerate()
            .map(|(i, s)| (s.as_str(), i))
            .collect();

        let initial = chart.initial.to_string();
        if !idx_of.contains_key(initial.as_str()) {
            return Err(ToolError::Unsupported(format!(
                "initial state `{initial}` is not a leaf state"
            )));
        }

        // Collect distinct event names. Index 0 is reserved for "ε" (the
        // implicit event of an eventless transition); placing it first means
        // every encoding can use `0` as a sentinel without a special case.
        let mut events: Vec<String> = vec!["ε".to_string()];
        for s in chart.iter_all_states() {
            for t in &s.transitions {
                if let Some(e) = &t.event {
                    let name = e.to_string();
                    if !events.iter().any(|x| x == &name) {
                        events.push(name);
                    }
                }
            }
        }

        // Flatten transitions to (source, event, target). Multi-target
        // transitions (parallel state entry) would be ambiguous in a flat
        // model; reject them.
        let mut edges = Vec::new();
        for s in chart.iter_all_states() {
            // Skip non-leaf or unknown source ids defensively (we already
            // rejected hierarchy above, so this is just future-proofing).
            let Some(&src) = idx_of.get(s.id.as_str()) else {
                continue;
            };
            for t in &s.transitions {
                let evt_idx = match &t.event {
                    None => 0,
                    Some(e) => events.iter().position(|x| x == e.as_str()).unwrap(),
                };
                if t.targets.len() > 1 {
                    return Err(ToolError::Unsupported(format!(
                        "transition from `{}` has {} targets; v1 only supports single-target transitions",
                        s.id,
                        t.targets.len()
                    )));
                }
                // Empty targets => self-transition (re-enter source). Model
                // it as src → src so traces still advance the step counter.
                let tgt_id = t.targets.first().map(|c| c.as_str()).unwrap_or(s.id.as_str());
                let Some(&tgt) = idx_of.get(tgt_id) else {
                    return Err(ToolError::Unsupported(format!(
                        "transition target `{tgt_id}` is not a known leaf state"
                    )));
                };
                edges.push((src, evt_idx, tgt));
            }
        }

        Ok(TransitionSystem {
            states,
            initial,
            events,
            edges,
        })
    }

    fn initial_idx(&self) -> usize {
        self.states.iter().position(|s| s == &self.initial).unwrap()
    }
}

// ───────────────────────── Z3 encoding ─────────────────────────

/// Symbolic snapshot of the system at one time step.
struct Frame {
    /// `in_S_k` Bool var, one per state in `TransitionSystem::states` order.
    in_state: Vec<Bool>,
    /// `evt_k` Int var: which event fires *out of* this frame to produce
    /// the next frame. The last frame's `evt` is unconstrained (unused).
    evt: Int,
}

impl Frame {
    /// Allocate a fresh Frame at step `k`. We use `fresh_const` so repeated
    /// `verify()` calls don't collide on names in the thread-local Z3 ctx.
    fn new(ts: &TransitionSystem, k: usize) -> Self {
        let in_state = ts
            .states
            .iter()
            .map(|s| Bool::fresh_const(&format!("in_{s}_{k}")))
            .collect();
        let evt = Int::fresh_const(&format!("evt_{k}"));
        Frame { in_state, evt }
    }

    /// "Exactly one state is active." Pseudo-Boolean encoding is more
    /// compact than the naive O(n²) pairwise mutex.
    fn mutex(&self) -> Bool {
        let pairs: Vec<(&Bool, i32)> = self.in_state.iter().map(|b| (b, 1)).collect();
        Bool::pb_eq(&pairs, 1)
    }
}

/// Build the initial-state predicate.
fn init_formula(ts: &TransitionSystem, f: &Frame) -> Bool {
    let init = ts.initial_idx();
    let mut clauses: Vec<Bool> = Vec::with_capacity(ts.states.len() + 1);
    for (i, b) in f.in_state.iter().enumerate() {
        if i == init {
            clauses.push(b.clone());
        } else {
            clauses.push(b.not());
        }
    }
    clauses.push(f.mutex());
    Bool::and(&clauses)
}

/// Build the transition relation T(f_k, f_{k+1}).
///
/// Disjunction over edges: pick exactly one to fire, asserting source
/// active in `f_k`, target active in `f_{k+1}`, and `evt_k` matching the
/// edge's label.
fn trans_formula(ts: &TransitionSystem, f_k: &Frame, f_next: &Frame) -> Bool {
    let mut disjuncts: Vec<Bool> = Vec::with_capacity(ts.edges.len());
    for &(src, evt, tgt) in &ts.edges {
        let conj = Bool::and(&[
            f_k.in_state[src].clone(),
            f_next.in_state[tgt].clone(),
            f_k.evt.eq(Int::from_i64(evt as i64)),
        ]);
        disjuncts.push(conj);
    }
    let step = if disjuncts.is_empty() {
        Bool::from_bool(false)
    } else {
        Bool::or(&disjuncts)
    };
    Bool::and(&[step, f_next.mutex()])
}

/// Compile a [`Prop`] AST into a Z3 Bool over the given frame.
fn encode_prop(p: &Prop, ts: &TransitionSystem, f: &Frame) -> Result<Bool, String> {
    Ok(match p {
        Prop::True => Bool::from_bool(true),
        Prop::False => Bool::from_bool(false),
        Prop::In(s) => match ts.states.iter().position(|x| x == s) {
            Some(i) => f.in_state[i].clone(),
            None => return Err(format!("unknown state `{s}` in `in(...)`")),
        },
        Prop::Not(a) => encode_prop(a, ts, f)?.not(),
        Prop::And(a, b) => Bool::and(&[encode_prop(a, ts, f)?, encode_prop(b, ts, f)?]),
        Prop::Or(a, b) => Bool::or(&[encode_prop(a, ts, f)?, encode_prop(b, ts, f)?]),
        Prop::Implies(a, b) => encode_prop(a, ts, f)?.implies(&encode_prop(b, ts, f)?),
        Prop::Iff(a, b) => encode_prop(a, ts, f)?.iff(&encode_prop(b, ts, f)?),
    })
}

/// Build `bound + 1` frames and assert init ∧ T(0,1) ∧ … ∧ T(b-1,b) into
/// `solver`. Returns the frames so the caller can add property assertions.
fn unroll(solver: &Solver, ts: &TransitionSystem, bound: usize) -> Vec<Frame> {
    let frames: Vec<Frame> = (0..=bound).map(|k| Frame::new(ts, k)).collect();
    solver.assert(init_formula(ts, &frames[0]));
    for k in 0..bound {
        solver.assert(trans_formula(ts, &frames[k], &frames[k + 1]));
    }
    frames
}

/// Reconstruct the active state name + chosen event at step `k` from a Z3
/// model. Used to print counterexample traces.
fn step_from_model(
    model: &z3::Model,
    ts: &TransitionSystem,
    f: &Frame,
    is_last: bool,
) -> Step {
    // Find the unique state Bool that the model evaluated to true.
    let state = ts
        .states
        .iter()
        .zip(f.in_state.iter())
        .find_map(|(name, b)| {
            model
                .get_const_interp(b)
                .and_then(|v| v.as_bool())
                .filter(|v| *v)
                .map(|_| name.clone())
        })
        .unwrap_or_else(|| "<unmodeled>".to_string());
    // The last frame doesn't have an outgoing event — the trace ends there.
    let event = if is_last {
        None
    } else {
        model
            .get_const_interp(&f.evt)
            .and_then(|v| v.as_i64())
            .and_then(|i| ts.events.get(i as usize).cloned())
    };
    Step { state, event }
}

// ───────────────────────── BMC ─────────────────────────

/// Bounded model checking. SAT means a counterexample exists within `bound`
/// steps; UNSAT means none does (but says nothing about deeper traces).
fn bmc(ts: &TransitionSystem, prop: &Prop, bound: usize) -> Outcome {
    let solver = Solver::new();
    let frames = unroll(&solver, ts, bound);

    // Property must hold at every step → its negation at *some* step is the
    // bug witness we ask Z3 to find.
    let bad_at_step: Result<Vec<Bool>, String> = frames
        .iter()
        .map(|f| Ok(encode_prop(prop, ts, f)?.not()))
        .collect();
    let bad_at_step = match bad_at_step {
        Ok(v) => v,
        Err(e) => {
            return Outcome::Unknown {
                reason: format!("property encoding failed: {e}"),
            };
        }
    };
    solver.assert(Bool::or(&bad_at_step));

    match solver.check() {
        SatResult::Unsat => Outcome::BoundedSafe { bound },
        SatResult::Sat => {
            let model = match solver.get_model() {
                Some(m) => m,
                None => {
                    return Outcome::Unknown {
                        reason: "solver returned SAT but no model".into(),
                    };
                }
            };
            // Walk every step; an honest trace contains the entire prefix
            // up to (and including) the violating frame, so callers can see
            // *how* we got there, not just where we ended up.
            let trace: Vec<Step> = frames
                .iter()
                .enumerate()
                .map(|(k, f)| step_from_model(&model, ts, f, k == frames.len() - 1))
                .collect();
            Outcome::Violated { trace }
        }
        SatResult::Unknown => Outcome::Unknown {
            reason: solver
                .get_reason_unknown()
                .unwrap_or_else(|| "z3 returned unknown".into()),
        },
    }
}

// ───────────────────────── k-induction ─────────────────────────

/// Try to prove `prop` is inductive at depths 1..=max_k.
///
/// At each `k` we run two queries:
///
/// * **Base**: starting from `init`, no violation in any of the first `k+1`
///   frames. UNSAT ⇒ no shallow counterexample.
/// * **Step**: from any state where the property held for `k` consecutive
///   steps, show it must hold at step `k+1`. UNSAT ⇒ truly inductive.
///
/// Both UNSAT for the same `k` → real proof. SAT in base → real
/// counterexample. SAT in step only → not inductive at this `k`, retry at
/// `k+1`.
fn k_induction(ts: &TransitionSystem, prop: &Prop, max_k: usize) -> Outcome {
    for k in 1..=max_k {
        // ── base case ──
        let solver = Solver::new();
        let frames = unroll(&solver, ts, k);
        let bad_at_step: Result<Vec<Bool>, String> = frames
            .iter()
            .map(|f| Ok(encode_prop(prop, ts, f)?.not()))
            .collect();
        let bad_at_step = match bad_at_step {
            Ok(v) => v,
            Err(e) => {
                return Outcome::Unknown {
                    reason: format!("property encoding failed: {e}"),
                };
            }
        };
        solver.assert(Bool::or(&bad_at_step));
        match solver.check() {
            SatResult::Sat => {
                // Real counterexample of length ≤ k+1 — same trace
                // extraction as BMC.
                let model = solver.get_model();
                let Some(model) = model else {
                    return Outcome::Unknown {
                        reason: "base SAT without model".into(),
                    };
                };
                let trace = frames
                    .iter()
                    .enumerate()
                    .map(|(i, f)| step_from_model(&model, ts, f, i == frames.len() - 1))
                    .collect();
                return Outcome::Violated { trace };
            }
            SatResult::Unknown => {
                return Outcome::Unknown {
                    reason: format!("base case at k={k} returned unknown"),
                };
            }
            SatResult::Unsat => {} // good — try inductive step
        }

        // ── inductive step ──
        // Free initial state (no `init_formula` here): we want to show the
        // property is closed under one transition for *any* legal state, not
        // only states reachable from init.
        let solver = Solver::new();
        let frames: Vec<Frame> = (0..=k).map(|i| Frame::new(ts, i)).collect();
        // Each frame must still satisfy mutex (exactly-one), otherwise the
        // solver could pick a state combination that isn't even meaningful.
        for f in &frames {
            solver.assert(f.mutex());
        }
        for i in 0..k {
            solver.assert(trans_formula(ts, &frames[i], &frames[i + 1]));
        }
        // Assume P at frames 0..k-1, then ask: can ¬P hold at frame k?
        for f in &frames[..k] {
            match encode_prop(prop, ts, f) {
                Ok(b) => solver.assert(b),
                Err(e) => {
                    return Outcome::Unknown {
                        reason: format!("property encoding failed: {e}"),
                    };
                }
            }
        }
        match encode_prop(prop, ts, &frames[k]) {
            Ok(b) => solver.assert(b.not()),
            Err(e) => {
                return Outcome::Unknown {
                    reason: format!("property encoding failed: {e}"),
                };
            }
        }
        match solver.check() {
            SatResult::Unsat => return Outcome::Holds { k },
            SatResult::Sat => continue, // not inductive at this k, try larger
            SatResult::Unknown => {
                return Outcome::Unknown {
                    reason: format!("inductive step at k={k} returned unknown"),
                };
            }
        }
    }
    Outcome::Unknown {
        reason: format!("k-induction did not converge within k={KIND_MAX_K}"),
    }
}

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// A trivial 2-state ping-pong chart — useful as a known-good sanity check.
    const PINGPONG: &str = r#"
        <scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
            <state id="a">
                <transition event="go" target="b"/>
            </state>
            <state id="b">
                <transition event="go" target="a"/>
            </state>
        </scxml>
    "#;

    #[test]
    fn pragma_extraction_finds_invariants_in_order() {
        let xml = r#"
            <scxml>
              <!-- @invariant id="p1" expr="in('a')" -->
              <state id="a"/>
              <!-- @invariant id="p2" expr="not in('b')" -->
            </scxml>
        "#;
        let p = extract_pragmas(xml);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].0, "p1");
        assert_eq!(p[1].1, "not in('b')");
    }

    #[test]
    fn property_parser_handles_full_grammar() {
        // Compose every operator and a state-id atom into one expression.
        let src = "not in('a') or (in('b') and true) => false <=> in('c')";
        let p = parse_property(src).expect("parse");
        // Just asserting it parses without panicking is enough for grammar
        // coverage; semantics are exercised by the encoding tests below.
        let _ = format!("{p:?}");
    }

    #[test]
    fn bmc_rejects_obviously_false_invariant() {
        let xml = format!(
            "{}{}",
            r#"<!-- @invariant id="bad" expr="in('a') and in('b')" -->"#, PINGPONG
        );
        let v = verify(&xml, VerificationTool::Smt).unwrap();
        assert_eq!(v.len(), 1);
        // Mutex makes this inconsistent in *every* frame, including frame 0,
        // so BMC must find a counterexample.
        match &v[0].outcome {
            Outcome::Violated { trace } => assert!(!trace.is_empty()),
            other => panic!("expected Violated, got {other:?}"),
        }
    }

    #[test]
    fn bmc_passes_safe_invariant() {
        // Mutex implies "not (in('a') and in('b'))" is invariant.
        let xml = format!(
            "{}{}",
            r#"<!-- @invariant id="ok" expr="not (in('a') and in('b'))" -->"#, PINGPONG
        );
        let v = verify(&xml, VerificationTool::Smt).unwrap();
        match &v[0].outcome {
            Outcome::BoundedSafe { bound } => assert_eq!(*bound, BMC_BOUND),
            other => panic!("expected BoundedSafe, got {other:?}"),
        }
    }

    #[test]
    fn k_induction_proves_state_disjunction() {
        // `in('a') or in('b')` is true initially and preserved by every
        // transition (only targets are `a` and `b`). Should be inductive at
        // small k — k=1 in particular.
        let xml = format!(
            "{}{}",
            r#"<!-- @invariant id="cover" expr="in('a') or in('b')" -->"#, PINGPONG
        );
        let v = verify(&xml, VerificationTool::KInduction).unwrap();
        match &v[0].outcome {
            Outcome::Holds { k } => assert!(*k >= 1, "expected Holds with k>=1, got k={k}"),
            other => panic!("expected Holds, got {other:?}"),
        }
    }

    #[test]
    fn k_induction_finds_base_counterexample() {
        // Initial state is `a`; the invariant `not in('a')` is false at
        // step 0. Base case must produce a counterexample.
        let xml = format!(
            "{}{}",
            r#"<!-- @invariant id="not_a" expr="not in('a')" -->"#, PINGPONG
        );
        let v = verify(&xml, VerificationTool::KInduction).unwrap();
        match &v[0].outcome {
            Outcome::Violated { trace } => {
                assert_eq!(trace[0].state, "a");
            }
            other => panic!("expected Violated, got {other:?}"),
        }
    }

    #[test]
    fn empty_invariants_returns_advisory_verdict() {
        let v = verify(PINGPONG, VerificationTool::Smt).unwrap();
        assert_eq!(v.len(), 1);
        assert!(matches!(v[0].outcome, Outcome::Unknown { .. }));
    }

    #[test]
    fn hierarchical_chart_is_rejected() {
        let xml = r#"
            <scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="parent">
                <state id="parent" initial="child">
                    <state id="child"/>
                </state>
            </scxml>
        "#;
        let err = verify(xml, VerificationTool::Smt).unwrap_err();
        assert!(matches!(err, ToolError::Unsupported(_)), "got {err:?}");
    }

    #[test]
    fn microwave_example_chart_produces_expected_verdicts() {
        // End-to-end smoke test against the bundled microwave.scxml.
        // We don't pin every verdict — just enough to confirm the rich
        // example actually exercises HOLDS *and* VIOLATED outcomes, so
        // the README example stays in sync with the verifier.
        let xml = include_str!("../microwave.scxml");
        let verdicts = verify(xml, VerificationTool::KInduction).unwrap();

        let by_id: std::collections::HashMap<&str, &Outcome> =
            verdicts.iter().map(|v| (v.property.as_str(), &v.outcome)).collect();

        assert!(matches!(
            by_id["no_cook_with_door_open"],
            Outcome::Holds { .. }
        ));
        assert!(matches!(by_id["state_coverage"], Outcome::Holds { .. }));
        assert!(matches!(
            by_id["never_door_closed_BUG"],
            Outcome::Violated { .. }
        ));
        assert!(matches!(
            by_id["never_cooks_BUG"],
            Outcome::Violated { .. }
        ));

        // Print the rendered verdicts when running with `--nocapture` so
        // operators can eyeball the output format.
        for v in &verdicts {
            print!("{}", v.render());
        }
    }

    #[test]
    fn elevator_no_doors_open_while_moving() {
        // The packaged elevator chart shouldn't permit `open` and `moving`
        // to be active simultaneously (they're mutex by virtue of being
        // distinct atomic states, so this should hold via k-induction).
        let xml = format!(
            "{}\n{}",
            r#"<!-- @invariant id="exclusive_motion" expr="not (in('open') and in('moving'))" -->"#,
            include_str!("../elevator.scxml")
        );
        let v = verify(&xml, VerificationTool::KInduction).unwrap();
        match &v[0].outcome {
            Outcome::Holds { .. } => {}
            other => panic!("expected Holds, got {other:?}"),
        }
    }
}
