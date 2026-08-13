# The instruction set

A stack machine with 24 instructions, two value types, and no way to name
anything outside the database. Every operand is either an immediate scalar or
an index into a table that was bounds-checked at load.

There is no instruction that calls a host function, computes an address,
allocates, or introduces a symbol. That is the point: a `.febin` is *data*, and
the worst a malicious one can do is ask the aircraft to move a control the host
already registered — which the host is free to refuse.

## Encoding

One opcode byte, then fixed-width little-endian operands. Instruction lengths
come from `format::instruction_len`, which the interpreter, verifier and
disassembler all share so they cannot disagree.

### Expression instructions

| Op | Byte | Operands | Stack |
| --- | --- | --- | --- |
| `NOP` | `0x00` | — | — |
| `PUSH_F32` | `0x01` | `f32` | → f32 |
| `PUSH_TRUE` | `0x02` | — | → bool |
| `PUSH_FALSE` | `0x03` | — | → bool |
| `LOAD_F32` | `0x04` | `u16` symbol | → f32 |
| `LOAD_BOOL` | `0x05` | `u16` symbol | → bool |
| `NOT` | `0x10` | — | bool → bool |
| `AND` | `0x11` | — | bool bool → bool |
| `OR` | `0x12` | — | bool bool → bool |
| `LT` `LE` `GT` `GE` | `0x18`–`0x1B` | — | f32 f32 → bool |
| `EQ_F32` `NE_F32` | `0x1C`–`0x1D` | — | f32 f32 → bool |
| `EQ_BOOL` `NE_BOOL` | `0x1E`–`0x1F` | — | bool bool → bool |

Expressions are emitted in post-order and are branch-free. `AND` and `OR` do
**not** short-circuit: both sides are evaluated. Operands are reads of aircraft
state with no side effects, so this costs nothing observable, and it keeps a
condition a straight run of bytes — which is what allows a `wait` body to be
re-evaluated by simply re-running a byte range.

### Control flow

| Op | Byte | Operands |
| --- | --- | --- |
| `JUMP` | `0x20` | `u32` target |
| `JUMP_IF_FALSE` | `0x21` | `u32` target (pops a bool) |

Targets are offsets from the start of the procedure's own code, so a procedure
is relocatable within the code section. **Both jumps are forward-only**: the
verifier rejects any target that is not strictly greater than the jump's own
offset. See [`verification.md`](verification.md).

### Actions

| Op | Byte | Operands |
| --- | --- | --- |
| `SET_POSITION` | `0x30` | `u16` control, `u8` position |
| `SET_ANALOG` | `0x31` | `u16` control, `f32` value |
| `CHECK` | `0x32` | `u16` control |
| `NOTIFY` | `0x33` | `u32` string |
| `CALL` | `0x34` | `u16` procedure |

### Waiting

| Op | Byte | Operands |
| --- | --- | --- |
| `AWAIT` | `0x40` | `u16` body length, `u32` timeout ms, `u8` on-timeout |
| `AWAIT_TEST` | `0x41` | — (pops a bool) |

`AWAIT` is followed by exactly `body_len` bytes of pure expression, then
`AWAIT_TEST`. On-timeout is 0 (continue) or 1 (fail); a timeout of 0 means
wait indefinitely.

`AWAIT_TEST` popping `true` falls through. Popping `false` parks: the executor
sets the instruction pointer back to the `AWAIT` and *returns*, ending the
tick. The backward move lives in the executor, not in the instruction stream,
which is how `wait` exists in a language whose bytecode has no backward edges.

### Termination

| Op | Byte | Operands |
| --- | --- | --- |
| `REQUIRE` | `0x50` | `u32` string or `0xFFFFFFFF` (pops a bool) |
| `COMPLETE` | `0x60` | — |
| `FAIL` | `0x61` | `u32` string or `0xFFFFFFFF` |
| `END` | `0x62` | — |

Every body ends with `END`. Reaching it at depth 0 completes the procedure;
inside a `call` it returns to the caller.

## Lowering

| Source | Bytecode |
| --- | --- |
| `check X` | `CHECK #x` |
| `set X = ON` / `start X` | `SET_POSITION #x, 1` |
| `set X = 22.5` | `SET_ANALOG #x, 22.5` |
| `notify "m"` | `NOTIFY $m` |
| `call P` | `CALL #p` |
| `require C "m"` | *C*; `REQUIRE $m` |
| `wait C timeout 30s else fail` | `AWAIT len,30000,fail`; *C*; `AWAIT_TEST` |
| `complete` | `COMPLETE` |
| `complete when C timeout 10s` | `AWAIT len,10000,fail`; *C*; `AWAIT_TEST`; `COMPLETE` |
| `fail "m"` | `FAIL $m` |

`if C { A } else { B }`:

```
      <C>
      JUMP_IF_FALSE @else
      <A>
      JUMP @end          ; omitted when A ends the procedure
else: <B>
end:
```

Omitting the skip jump when the `then` branch terminates is not just a
five-byte saving: it avoids emitting an instruction that is unreachable, which
keeps the verifier's reachability analysis honest.

## Reading it

`fe_runtime::disassemble` prints a procedure with symbol and control names
resolved:

```
procedure HYD_ALL_SYSTEMS_CHECK ; "Hydraulic Systems Check" [normal priority=0 rev=0]
.body
0000  CHECK          #9 ; HYD_1_ENGINE_PUMP
0003  CHECK          #7 ; HYD_2_ENGINE_PUMP
0006  CHECK          #10 ; HYD_3_ENGINE_PUMP
0009  AWAIT          body=19 timeout=15000ms on_timeout=fail
0017  LOAD_F32       #10 ; hydraulic.1.pressure
0020  PUSH_F32       2500
0025  GT
0026  LOAD_F32       #9 ; hydraulic.2.pressure
0029  PUSH_F32       2500
0034  GT
0035  AND
0036  AWAIT_TEST
0037  COMPLETE
0038  END
```

It writes through `core::fmt::Write`, so it works in `no_std` — into a fixed
buffer, a serial port, or the MSFS console — as well as into a `String`.
