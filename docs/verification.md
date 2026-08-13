# Verification

Every procedure is verified once, at load, inside `from_bytes`. Nothing is
verified lazily and nothing is re-verified per tick.

Verification establishes four properties the interpreter then relies on:

1. **Decodability** — every byte reachable as an instruction start decodes to a
   known opcode with in-range operands.
2. **Termination** — a tick cannot loop.
3. **Stack discipline** — operands are typed and balanced, and the stack is
   empty at every statement boundary, so 32 slots always suffice.
4. **Reference validity** — every symbol, control, position, string and
   procedure operand names an entry that exists.

## Why a tick terminates

This is the property that matters most, because the alternative is a hung
simulation thread: in MSFS a WASM gauge that does not return has frozen the
aircraft, and possibly the sim.

The argument has three parts.

**Jumps are forward-only.** The verifier rejects any `JUMP` or
`JUMP_IF_FALSE` whose target is not strictly greater than the jump's own
offset. The instruction pointer within one procedure body therefore increases
monotonically, and a body is finite, so a straight-line run through it is
finite.

**`wait` is not a backward jump.** It would have been the obvious way to
implement it — test the condition, jump back if false — and it would have
destroyed the property above. Instead a `wait` compiles to straight-line code
(`AWAIT`, condition, `AWAIT_TEST`), and re-entry is the *executor's* business:
when `AWAIT_TEST` pops false, the executor rewinds its own instruction pointer
to the `AWAIT` and returns from `tick`. The only backward transition in the
whole design is one that always ends the tick, so it cannot contribute to a
loop within a tick.

**`call` cannot recurse.** Cycles in the call graph are rejected by the
compiler (E0210), as are chains deeper than the runtime's fixed frame array
(E0215), and the executor re-checks depth anyway. The call graph is therefore a
DAG of bounded depth over finite bodies.

Those three together make a tick provably finite. The executor *also* carries
an independent per-tick instruction budget (`DEFAULT_STEP_LIMIT`, 4096) so that
even a hypothetical verifier bug degrades to a failed procedure rather than a
frozen aircraft. Belt and braces, because the cost of the braces is one integer
decrement per instruction.

## Stack discipline

The verifier walks each body with an abstract stack of *types*.

* Expression instructions push and pop as documented in
  [`bytecode.md`](bytecode.md); a type mismatch is rejected.
* At every statement boundary the stack must be empty. `SET_POSITION` with a
  leftover value on the stack is a malformed file, not a curiosity.
* At every branch target the stack must be empty, both on the path that falls
  into it and on the path that jumps to it. This is what makes the walk a
  single linear pass rather than a dataflow fixpoint.
* Depth may not exceed `STACK_CAPACITY` (32), which is the physical size of the
  executor's stack array.

Unreachable regions — code after a `COMPLETE`, `FAIL` or unconditional `JUMP`
that nothing jumps to — are still structurally validated, but analysed with a
fresh stack so they cannot corrupt the reachable state.

## Branch targets

Pending forward targets are held in a fixed 64-entry array, matching the
compiler's nesting limit. A file with more outstanding branches than that is
rejected (`TooComplex`) rather than allocated for — the runtime never
allocates.

A target must land exactly on an instruction start. Because the walk visits
instruction starts in increasing order and removes targets as it reaches them,
any target still pending at the end of the body was either past the end or in
the middle of an instruction; either way the file is rejected.

## `AWAIT` pairing

An `AWAIT` records where its `AWAIT_TEST` must appear (`next + body_len`).
Reaching an `AWAIT_TEST` anywhere else, reaching the end with one outstanding,
or nesting one `AWAIT` inside another's body all fail with `BadWait`.

## What this buys

The fuzz suite mutates a real database two thousand ways, plus every single-bit
flip in the file, and asserts two things: `from_bytes` never panics, and
anything it *accepts* can be executed without panicking and without a tick
failing to make progress. Roughly two thirds of single-bit flips still verify —
they land in strings, priorities, or the hash — and every one of them then runs
safely.

That is the real contract. "The verifier rejects bad files" is easy and not
very useful; "anything the verifier accepts is safe to execute" is what lets
the interpreter be written without defensive checks in its hot loop.
