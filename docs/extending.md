# Extending it

## Adding a statement

Worked example: a hypothetical `announce "..." priority 2`.

1. **Token** — add a `Keyword` variant in `fe-lang/src/token.rs` and its string
   in `from_str` / `as_str`. Add it to `starts_step()` if it begins a
   statement, which also teaches error recovery about it.
2. **AST** — add a `Step` variant in `fe-lang/src/ast.rs` with a span, and
   handle it in `Step::span()`.
3. **Parser** — add a case in `Parser::step`.
4. **IR** — add an `IrStep` variant; update `terminates()` if it can end a
   procedure.
5. **Analysis** — lower it in `Analyzer::step`, validating whatever needs
   validating and interning any strings.
6. **Opcode** — see below.
7. **Code generation** — add a case in `Assembler::step`.
8. **Verifier** — add a case in `verify_body` stating its stack effect and
   which operands it references.
9. **Interpreter** — add a case in `run`.
10. **Disassembler** — add a case in `write_instruction`.

Steps 6–10 are all in `fe-runtime`, and the compiler will fail to build until
7 matches 6, which is the point of the dependency direction.

Tests to add: a parser test for the syntax, a sema test for each rejection, a
codegen snapshot for the emitted shape, an execution test for the behaviour.

## Adding an opcode

In `fe-runtime/src/format.rs`:

* pick an unused byte in the right block (`0x0x`/`0x1x` expression,
  `0x2x` control flow, `0x3x` actions, `0x4x` waiting, `0x5x`/`0x6x`
  termination);
* add it to `op`;
* add its length to `instruction_len` — **this is the one that matters**, since
  every walker derives instruction boundaries from it;
* add an `Instr` variant and a `decode` case;
* add it to `is_expression_op` if it is one.

Then handle it in the verifier, the interpreter and the disassembler. All three
go through `decode`, so a missing case is a compile error rather than a
divergence.

Adding an opcode is a **format-breaking change** for old readers: a v1 runtime
will reject a file containing it with `UnknownOpcode`, which is the correct
behaviour but means the version must be bumped if such files will ship.

## Adding a control kind

1. Add a `ControlKind` variant with an explicit discriminant and update
   `from_u8` / `as_str`.
2. Add a `ControlSpec` constructor and teach `positions()`, `position_index()`,
   `kind()` and `validate()` about it.
3. Decide what the verbs mean for it, if anything.

Old runtimes read an unknown kind as `ControlKind::Unknown` and pass it to the
host rather than guessing, so this is *not* format-breaking for `check`-only
use — but a `SET_POSITION` on an unknown kind will still be rejected by an old
verifier, because it cannot check the position index.

## Changing the binary format

Additive changes:

1. bump `FORMAT_VERSION`;
2. append fields — never reorder or resize existing ones;
3. if you extend the header, grow `HEADER_SIZE` and rely on the stored
   `header_size`, which readers already use to locate the payload;
4. update `docs/binary-format.md` in the same commit. `fe-runtime`'s
   `tests/standalone.rs` builds a database by hand from the documented layout,
   so the doc and the code are tested against each other.

Old readers reject a newer version by name (`UnsupportedVersion { found,
supported }`) rather than misreading it. If backward compatibility is wanted,
`from_bytes` is where a v1-vs-v2 branch would go, and the reserved word at
header offset 76 is available for a feature bitmap.

## Adding a diagnostic

Append to `fe-lang/src/diagnostics::codes` in the right block. Never reuse a
retired code: someone's CI is filtering on it.

Give it a note or a help line. "Unknown symbol `X`" is a fact; "did you mean
`Y`?" is a fix, and the difference matters at 2am.

## Relaxing a limit

`MAX_NESTING`, `MAX_CALL_DEPTH`, `STACK_CAPACITY` and the verifier's
`MAX_PENDING` are all coupled to fixed-size arrays in the runtime. Raising one
means raising the array, which costs stack in a WASM gauge. Raising
`MAX_PENDING` without raising `MAX_NESTING` is safe; the reverse is not, and
the verifier will start rejecting deeply nested procedures with `TooComplex` if
you get it wrong.

## Testing

The suites and what they are for:

| Suite | Covers |
| --- | --- |
| `fe-lang/tests/lexer.rs` | tokens, paths versus decimals, durations, malformed input |
| `fe-lang/tests/parser.rs` | structure, precedence, recovery, rendering |
| `fe-compiler/tests/sema.rs` | resolution, types, control rules, call graph, warnings |
| `fe-compiler/tests/codegen.rs` | emitted shape, via disassembly snapshots |
| `fe-compiler/tests/binary.rs` | layout, determinism, the corruption matrix |
| `fe-compiler/tests/execution.rs` | end-to-end behaviour against a fake aircraft |
| `fe-compiler/tests/fuzz.rs` | seeded mutation; verified implies executable |
| `fe-runtime/tests/standalone.rs` | the runtime with no compiler, from hand-built bytes |

A change that touches the format should fail `binary.rs` and `standalone.rs`.
A change that touches lowering should fail `codegen.rs`. If a change touches
neither and everything still passes, be suspicious.
