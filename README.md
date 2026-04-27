# charta

> Formal verification for agent-generated state machines. Because *probably correct* isn't good enough.

`charta` is an [MCP](https://modelcontextprotocol.io/) server that lets an agent author [SCXML](https://www.w3.org/TR/scxml/) state charts, validate them, visualise them, and generate provably-correct source code in Rust, Go, C++, Kotlin, or C11.

```
spec.scxml → validate_state_chart → codegen_state_chart → rustc / go build / clang / kotlinc → your tests
```

## Tools

| Tool | Description |
| --- | --- |
| `validate_state_chart` | Parses and structurally validates an SCXML XML string. Returns `OK` on success, structured `invalid_params` error otherwise. |
| `visualise_state_chart` | Renders an SCXML state chart as a [Mermaid](https://mermaid.js.org/) diagram. |
| `codegen_state_chart` | Generates source code for the chosen `backend` (`rust`, `go`, `cpp`, `kotlin`, or `c11`). |

All tools that consume SCXML fail fast with typed `invalid_params` errors when the input cannot be parsed or validated — no panics, no string-in-success-payload error reporting.

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

Each generated file is returned as a separate text content block prefixed with `// === <filename> ===`.
