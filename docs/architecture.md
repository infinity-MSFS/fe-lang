# Architecture

Three crates, one direction of dependency:

```
fe-lang  ──────►  fe-compiler  ──────►  .febin bytes  ──────►  fe-runtime
(text → AST)      (AST → bytes)                                (bytes → behaviour)
                        │                                            ▲
                        └──────────── depends on ────────────────────┘
```

`fe-compiler` depends on `fe-runtime`. That looks backwards until you ask
where the format constants should live. If the compiler owned them, the
runtime would need its own copy, and the two would drift the first time
someone added an opcode. Instead the runtime owns the format — opcodes, record
sizes, header offsets, the `Category` enum — and the compiler writes what the
runtime declares.

The compiler also *runs* the runtime: before `compile` returns, it loads its
own output through `ProcedureDatabase::from_bytes`, which verifies every
procedure with the same verifier the aircraft will use. A compiler bug that
would produce an unloadable database fails the build instead of the flight.

## What each crate may not do

| Crate | Must not |
| --- | --- |
| `fe-lang` | know what a symbol *is*, type-check, or generate code |
| `fe-compiler` | touch the filesystem, read environment variables, or expose a CLI |
| `fe-runtime` | parse text, allocate, use `std`, or depend on the compiler |

These are enforced by construction rather than by convention:

* `fe-lang` has no dependency on `fe-runtime`, so it *cannot* mention a
  `ValueType` or an opcode.
* `fe-compiler`'s public API takes `&[SourceUnit]` — text the caller already
  loaded — and returns `Vec<u8>`. There is no path type anywhere in it.
* `fe-runtime` is `#![no_std]` and `#![forbid(unsafe_code)]` with no
  dependencies, so `cargo build` fails if anything sneaks in.

## Why no compiler binary

Reading files, globbing directories, deciding where output goes, and knowing
what the aircraft's symbols are called are all properties of a specific
aircraft's build, not of the language. An executable would have to invent
answers to all four and would then be in everyone's way.

`fe-compiler/examples/build_database.rs` is the whole build tool for the
example aircraft, in about a hundred lines, and it is the shape a `build.rs`
would take.

## The pipeline

| Stage | Module | In | Out |
| --- | --- | --- | --- |
| Lexing | `fe-lang::lexer` | `&str` | `Vec<Token>` |
| Parsing | `fe-lang::parser` | tokens | `Ast` |
| Analysis | `fe-compiler::sema` | `Ast` + registry | `IrModule` |
| Code generation | `fe-compiler::codegen` | `IrModule` | bytecode |
| Emission | `fe-compiler::emit` | `IrModule` + code | `.febin` bytes |
| Verification | `fe-runtime::verify` | bytes | `ProcedureDatabase` |
| Execution | `fe-runtime::interpreter` | database + host | actions |

Every stage before emission reports problems as diagnostics with source spans
and keeps going; the compiler returns errors only once the whole pipeline has
had a chance to speak. An author fixing a procedure at 2am should see every
problem in the file, not the first one.

## Further reading

* [`language.md`](language.md) — the language itself
* [`semantics.md`](semantics.md) — what analysis checks
* [`ir.md`](ir.md) — the intermediate representation
* [`bytecode.md`](bytecode.md) — the instruction set
* [`binary-format.md`](binary-format.md) — the file layout
* [`verification.md`](verification.md) — why a tick always terminates
* [`runtime.md`](runtime.md) — the execution model
* [`host-integration.md`](host-integration.md) — embedding it in an aircraft
* [`symbols.md`](symbols.md) — the symbol registry
* [`diagnostics.md`](diagnostics.md) — every error and warning code
* [`design-decisions.md`](design-decisions.md) — the arguments behind the shape
* [`extending.md`](extending.md) — adding to the language or the format
