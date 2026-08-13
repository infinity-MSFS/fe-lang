# The intermediate representation

IR is what survives semantic analysis. Every name is an index, every expression
has a known type, every literal has been range-checked. Code generation is then
a mechanical walk with no lookups and no way to fail, which is what makes the
output deterministic.

IR is `pub(crate)`. It is not part of the public API, because pinning callers
to it would freeze the compiler's internals.

## Shape

```rust
struct IrModule {
    procedures: Vec<IrProcedure>,  // sorted by identifier
    symbols:    Vec<IrSymbol>,     // interned in first-use order
    controls:   Vec<IrControl>,
    strings:    Vec<String>,
}
```

`IrProcedure` carries the metadata, an optional trigger expression, and a
`Vec<IrStep>`. It also carries the span of its declaration — the one piece of
source information the IR keeps, so that the two late capacity errors
(procedure too large, stack too deep) can still point at real source rather
than at nothing.

`IrExpr` is a small tree — `Bool`, `Number`, `Load { symbol, ty }`, `Not`,
`And`, `Or`, `Compare { op, lhs, rhs }` — with a `stack_depth()` method the
compiler uses to check against the runtime's fixed stack before emitting
anything.

`IrStep` is the resolved statement set: `SetPosition`, `SetAnalog`, `Check`,
`Notify`, `Call`, `Require`, `Wait`, `If`, `Complete`, `Fail`. Note what is
*not* there: no verbs (lowered to `SetPosition`), no `complete when` (lowered
to `Wait` + `Complete`), no `else if` (nested `If` in the else arm). Sugar is
gone by this point, so code generation has one case per construct.

## Why a separate IR at all

The AST is a faithful record of what was written, spans and all, and it has to
stay that way for diagnostics. The bytecode is a flat byte array. Neither is a
good place to answer "what does this procedure actually do".

The IR is where the two useful invariants live:

* **Everything is resolved.** `IrStep::Call { procedure: u16 }` cannot name a
  procedure that does not exist, because turning a name into that index was
  what checked it.
* **Everything is ordered.** The traversal order over the IR *is* the id
  assignment order, so determinism is a property of the data structure rather
  than a discipline code generation has to maintain.

## Interning

`Interner` assigns string ids in first-use order over the fixed traversal, and
its lookup map is a `BTreeMap`. Symbols and controls intern the same way, in
`Analyzer::intern_state` / `intern_control`.

The result: a database contains exactly the strings, symbols and controls its
procedures reference, each once, in an order that depends only on the sources.

## `terminates()`

`IrStep::terminates()` answers "does this step unconditionally end the
procedure". `Complete` and `Fail` do; an `If` does when both arms exist and
both end in a terminating step.

Two things use it. Semantic analysis drops steps after a terminator with W0001.
Code generation omits the skip-the-else jump when the `then` arm terminates,
which avoids emitting an unreachable instruction — see
[`bytecode.md`](bytecode.md).
