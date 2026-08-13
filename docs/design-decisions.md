# Design decisions

The arguments behind the shape of the thing, including the ones where the
answer was "no".

## Forward-only jumps, and `wait` as an executor concern

The obvious way to compile `wait C` is: evaluate `C`, jump backwards if false.
It is two instructions and everyone understands it. It also makes every
termination argument about a tick into a hand-wave.

Instead the bytecode has **no backward edges at all**, and the verifier
enforces it. `wait` compiles to straight-line code (`AWAIT`, condition,
`AWAIT_TEST`), and the re-entry is performed by the executor, which rewinds its
own instruction pointer and *returns*. The only backward transition in the
design is one that always ends the tick.

The payoff: "a tick terminates" is a structural property provable from the
verifier's rules, not a claim about the compiler's good behaviour. In an MSFS
gauge, where not returning means a frozen aircraft, that is worth designing
around. Full argument in [`verification.md`](verification.md).

## No arithmetic

`hydraulic.1.pressure - hydraulic.2.pressure > 500` is not expressible. This
looks like a gap until you ask where such a quantity should be defined. If a
differential matters, the aircraft should expose it as a symbol: then it has
one definition, the systems code can test it, the flight engineer's manual can
name it, and every procedure that uses it agrees about what it means.

Arithmetic in the procedure language would let five procedures each invent
their own slightly different derived quantity, and there would be no place to
go and check which one is right.

It also keeps the type checker and the interpreter trivial: two types, no
promotion, no overflow, no division by zero, no NaN propagation to reason
about.

## No short-circuit evaluation

`AND` and `OR` evaluate both sides. Operands are pure reads of aircraft state,
so there is nothing to short-circuit *for* — no side effects, no expensive
calls, no null to guard against.

What it buys is that a condition is a contiguous, branch-free run of bytes.
That is what makes a `wait` body something the executor can re-run by pointing
at a byte range, and what makes an expression region trivially verifiable.

## Two types

Bool and f32. No integers (a float represents every quantity on the panel
exactly enough, and mixing the two would mean promotion rules), no strings as
values (there is nothing to do with one), no enums (positions are already
indices).

## `isolate` and `transfer` were dropped

The original sketch had `isolate hydraulic.2` and `transfer hydraulic.3 to
hydraulic.2` as statements. Both were cut.

They are not primitives; they are names for a *configuration*. Isolating a
hydraulic system means closing a specific shutoff valve on a specific aircraft,
and transferring means putting a selector in a specific detent. On the DC-10
that selector genuinely exists, so the honest spelling is:

```
set HYD_TRANSFER_SELECTOR = THREE_TO_TWO
```

which says what the crew member's hand does. `transfer hydraulic.3 to
hydraulic.2` reads better and says less: it hides which control moves, it would
need the compiler to know which systems can donate to which, and on an aircraft
where the transfer works differently it would quietly mean something else.

A language for procedures should describe actions on controls. Naming
configurations is the registry's job, and the sugar that survived — `start`,
`stop`, `open`, `close` — survived because each maps to exactly one position on
one control with no aircraft knowledge required.

## A flat procedure namespace

No modules, no imports, no qualified names. Every procedure compiled together
shares one namespace, and a duplicate identifier is an error.

A procedure database is a catalogue that a human reviews as a whole, the way a
QRH is reviewed as a whole. Identifiers like `HYD_2_LOW_PRESSURE` are already
namespaced by convention, and a module system would add a second, weaker
namespacing mechanism plus a resolution algorithm to argue about — for a corpus
that is a few hundred entries at most.

The flat namespace also means `call` reaches across files with no ceremony,
which is what you want when `ELEC_BUS_2_RESTORE` lives in `electrical.fe` and
the hydraulic procedure needs it.

## State is read-only, controls are write-only

Enforced, not encouraged. See [`symbols.md`](symbols.md). A procedure that
could assign `hydraulic.2.pressure` would be able to lie to every other system
in the aircraft, and the mistake would look exactly like a working procedure.

The mirror rule — controls are not readable — is less obvious but the same
argument. "The switch is on" and "the pump is running" are different facts. If
a procedure needs the second one, the aircraft must expose it as state, which
forces someone to decide what it actually means.

## The compiler depends on the runtime

Not the other way round, and not a shared `-types` crate. The runtime owns the
format because the runtime is what has to survive reading a hostile file; the
compiler writes what the runtime declares, and then reads its own output back
through the runtime's verifier before returning.

A shared crate would have worked, but it would have created a place where
format constants live without anything that *uses* them, and the self-check
would have needed a fourth crate to live in.

## The compiler is a library

No CLI, no filesystem access, no environment variables. Discussed in
[`architecture.md`](architecture.md); the short version is that every one of
those would require the compiler to invent an answer to a question that belongs
to a specific aircraft's build.

## Fixed limits everywhere

Nesting 16, call depth 8, stack 32, 65535 procedures. Each is a compile error
with a real span rather than a runtime failure, because each corresponds to a
fixed-size array in an allocation-free runtime.

Making them compile errors is the whole trick: the runtime can then be written
without a single "what if this is too deep" branch in its hot loop, and the
aircraft cannot be surprised by a procedure that was fine on the ground.

## The content hash is not a security feature

FNV-1a 32 catches a corrupted download. It does not stop a determined editor,
and it is not meant to — the runtime never trusts it *in place of* validation,
and a database with the hash flag cleared is validated exactly as strictly.

Signing a database is a host concern, and a host that needs it can sign the
bytes it embeds.
