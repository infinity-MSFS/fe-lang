# Semantic analysis

Between the AST and code generation sits the one pass that has opinions.
Everything decidable without generating code is decided here, and everything
decided here is reported as a diagnostic with a span rather than as a bail-out:
the analyser reports as many real problems as it can find before giving up.

The output is either a fully resolved `IrModule` or a bag of errors — never a
half-lowered module.

## Order of work

1. **Collect** every procedure from every unit into one flat namespace.
   Duplicates are E0209, reported with both locations.
2. **Sort** the procedures by identifier. This happens *before* lowering, and
   it is what makes the output independent of source file order — every string
   id, symbol id and call index assigned afterwards depends only on this fixed
   traversal.
3. **Lower** each procedure: validate metadata, resolve names, type-check
   expressions, check control actions, lower steps to IR.
4. **Check the call graph** for cycles and depth.

## Resolution

Every path is looked up in the `SymbolRegistry`:

* not found → E0201, with a suggestion if an entry is within a small edit
  distance;
* found as state, used as a control → E0202;
* found as a control, used in an expression → E0203.

Resolved symbols and controls are interned in first-use order, so the tables in
the output contain only what the procedures actually reference.

## Types

Two types, no conversions. `ValueType::Bool` and `ValueType::F32`.

* Conditions (`trigger`, `require`, `wait`, `if`, `complete when`) must be
  boolean — E0204 otherwise.
* `&&`, `||`, `!` take booleans.
* `<`, `<=`, `>`, `>=` take numbers.
* `==`, `!=` take two of the same type and lower to type-specific opcodes.
* Numeric literals are checked to be representable as `f32`.

Two warnings come out of type checking: comparing floats with `==` (W0002) and
a condition that reads no aircraft state at all (W0005) — `wait true` is
almost always a mistake, and `if false { ... }` is dead weight in a document
someone will have to review.

## Controls

Each action is checked against the control's registered kind:

| Problem | Code |
| --- | --- |
| verb has no such position (`open` a switch) | E0206 |
| named position not in a selector's list | E0205 |
| a number on a non-analog control, or a name on an analog one | E0205 |
| analog value outside the registered range | E0207 |
| actuating a `checklist` item | E0206 |

Messages list what *is* accepted, so the author does not have to go and read
the registry source.

## Metadata

`name` and `category` are required (E0211). `category` must be one of the four
(E0212). `priority` and `revision` must be non-negative integers within their
field widths (E0212). A `timeout` of zero is E0213 — a note points out that
omitting `timeout` is how you wait indefinitely — and `complete timeout 10s`
without a `when` is E0213 too.

## Limits

These exist because the runtime is allocation-free: every one of them
corresponds to a fixed-size array in the executor or the verifier.

| Limit | Value | Code |
| --- | --- | --- |
| `if` nesting | 16 | E0214 |
| `call` depth | `MAX_CALL_DEPTH` (8) | E0215 |
| expression stack depth | `STACK_CAPACITY` (32) | E0216 |
| procedures / symbols / controls | 65535 each | E0217 |

Call depth is computed with a memoised longest-path walk, so the diagnostic
names the chain that is too deep rather than the procedure that happened to be
analysed last.

## The call graph

Cycles are rejected outright (E0210), found with an iterative depth-first walk —
iterative rather than recursive so that a pathological input cannot blow the
*compiler's* stack while it is busy proving the runtime's cannot overflow.

Recursion is not a feature this language is missing. A procedure is a document
a crew follows; one that could call itself would have no bounded reading, and
the executor would need a heap-allocated frame stack to run it.

## Lowering

Steps become `IrStep`s with all names resolved to indices:

* verbs become `SetPosition` with the position's index;
* `complete when C timeout D` becomes `Wait { fail_on_timeout: true }`
  followed by `Complete` — the `true` is the safety-relevant part, and it is
  set here rather than in code generation so it is visible in the IR;
* `require` clauses are lowered ahead of the first body step, so preconditions
  are checked before anything moves;
* steps after a terminator are dropped with W0001, and an `if` with no steps in
  either arm is dropped with W0003.

## Warnings

| Code | Meaning |
| --- | --- |
| W0001 | unreachable step after `complete` or `fail` |
| W0002 | float compared with `==` |
| W0003 | `if` with empty branches |
| W0005 | condition reads no aircraft state |

Warnings never prevent compilation. A build that wants them fatal checks
`compiled.warnings()` itself.
