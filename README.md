# charta

> Formal verification for agent-generated state machines. Because *probably correct* isn't good enough.

Agents write [SCXML](https://www.w3.org/TR/scxml/) specs, `charta` verifies them formally and emits provably-correct Rust. Structured diagnostics close the feedback loop — no human review of logic required.

```
spec.scxml → charta validate → charta generate → rustc → your tests
```
