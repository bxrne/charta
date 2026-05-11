# charta

> Formal verification for agent-generated state machines. Because *probably correct* isn't good enough.

`charta` is an [MCP](https://modelcontextprotocol.io/) server that lets an agent author [SCXML](https://www.w3.org/TR/scxml/) state charts, validate them, visualise them, and generate provably-correct source code in Rust, Go, C++, Kotlin, or C11.

```
spec.scxml → validate_state_chart → verify_state_chart → codegen_state_chart → rustc / go build / clang / kotlinc → your tests
                                          ↑
                                    Z3 (BMC / k-induction)
```


## Tools

| Tool | Description |
| --- | --- |
| `validate_state_chart` | Parses and structurally validates an SCXML XML string. Returns `OK` on success, structured `invalid_params` error otherwise. |
| `visualise_state_chart` | Renders an SCXML state chart as a [Mermaid](https://mermaid.js.org/) diagram. |
| `verify_state_chart` | Formally verifies user-declared invariants via [Z3](https://github.com/Z3Prover/z3). Choose `Smt` (bounded model checking, finds bugs) or `KInduction` (real proofs). |
| `codegen_state_chart` | Generates source code for the chosen `backend` (`rust`, `go`, `cpp`, `kotlin`, or `c11`). |

All tools that consume SCXML fail fast with typed `invalid_params` errors when the input cannot be parsed or validated — no panics, no string-in-success-payload error reporting.

## Verification

Declare invariants directly in your SCXML using XML comment pragmas — they survive any conformant parser without breaking validation:

```xml
<!-- @invariant id="exclusive_motion" expr="not (in('open') and in('moving'))" -->
<!-- @invariant id="reachable_idle"   expr="in('idle') or in('moving') or in('opening') or in('open') or in('closing')" -->
```

The expression language is propositional logic over state-membership atoms:

| Construct | Meaning |
| --- | --- |
| `in('S')` | The leaf state `S` is currently active. |
| `not P`, `P and Q`, `P or Q` | Standard Boolean operators. |
| `P => Q`, `P <=> Q` | Implication and biconditional. |
| `true`, `false`, `(…)` | Constants and grouping. |

`verify_state_chart` returns one text block per invariant. Each verdict is one of:

| Verdict | Meaning |
| --- | --- |
| `HOLDS` | k-induction proved the property is inductive at depth `k`. A real proof. |
| `BOUNDED-SAFE` | BMC unrolled to N steps without finding a counterexample. **Not a proof** — a violation may exist deeper. |
| `VIOLATED` | A concrete counterexample trace, step-by-step, with the active state and event chosen at each step. |
| `UNKNOWN` | Z3 returned `unknown`, k-induction did not converge, or no invariants were declared. |

### Backends

| Backend | What it does | When to use it |
| --- | --- | --- |
| `Smt` | Bounded model checking — unrolls the transition relation `BMC_BOUND` steps and asks Z3 for a counterexample. | Bug-finding. Cheap, always terminates, but absence of a CEX is only "no bug within the bound". |
| `KInduction` | Base case (no shallow CEX) + inductive step (property closed under one transition for any state). | Real safety proofs. Returns `HOLDS` only when both queries are UNSAT. |

### Limitations (v1)

* **Flat charts only.** Nested `<state>`, `<parallel>`, and `<history>` are rejected with a typed `Unsupported` error.
* **Guards and datamodel are nondeterministic.** Transition `cond=` and `<assign>` expressions are *not* interpreted; the verifier conservatively assumes any guard can evaluate either way. Sound for safety; counterexamples may be spurious if they rely on a guard pattern your real datamodel forbids.
* **Events are an opaque alphabet.** `<send delay="…">` timing and event ordering are ignored.
* **Single-target transitions only.** Multi-target (parallel-entry) transitions are rejected.

## Install

```bash
cargo build --release
```

The `codegen_state_chart` tool shells out to `sce-codegen` from [scxml-core-engine](https://github.com/newmassrael/scxml-core-engine). Install it once:

```bash
cargo install --git https://github.com/newmassrael/scxml-core-engine sce-build --features cli
```

`sce-build` links against `libxml2` at build time. On Ubuntu / Debian:

```bash
sudo apt-get install -y libxml2-dev pkg-config
```

On Fedora / RHEL: `sudo dnf install libxml2-devel pkgconfig`. On macOS: `brew install libxml2 pkg-config`.

If the binary lives somewhere other than your `PATH`, point at it with `SCE_CODEGEN_BIN=/abs/path/to/sce-codegen`.

## Run

`charta` speaks MCP over stdio:

```bash
./target/release/charta
```

Wire it into your MCP-aware client (Claude Desktop, Amp, etc.) the same way as any other stdio MCP server.

## Backends

`codegen_state_chart` accepts `backend` as one of:

| Backend | Output | Notes |
| --- | --- | --- |
| `rust` | `chart_sm.rs` | `StatePolicy` trait impl |
| `go` | `chart_sm.go` | requires Go 1.22+ (generics) |
| `cpp` | `chart_sm.h` + `chart_sm.inl` | CRTP, clang-format applied |
| `kotlin` | `chartSm.kt` | sealed interfaces, coroutine-based |
| `c11` | `chart_sm.h` + `chart_sm.c` | MCU / embedded target |

Each generated file is returned as a separate text content block prefixed with `// file: <filename>`.
