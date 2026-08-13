# The symbol registry

The compiler knows nothing about aircraft. Everything a procedure may read or
touch is declared by the host in a `SymbolRegistry`, and anything not in it is
a compile error.

```rust
let mut r = SymbolRegistry::new();

r.define_state("hydraulic.2.pressure", ValueType::F32, tag::HYD2_PRESSURE)?;
r.define_state("engine.2.running", ValueType::Bool, tag::ENG2_RUNNING)?;

r.define_control("HYD_2_ELECTRIC_PUMP", ControlSpec::switch(), tag::HYD2_PUMP)?;
r.define_control("HYD_2_ISOLATION_VALVE", ControlSpec::valve(), tag::HYD2_VALVE)?;
r.define_control(
    "FUEL_XFEED_SELECTOR",
    ControlSpec::selector(["OFF", "TANK_1_TO_3", "TANK_3_TO_1"]),
    tag::XFEED,
)?;
r.define_control("FUEL_PUMP_PRESSURE_TARGET", ControlSpec::analog(0.0, 50.0), tag::PUMP_PRESS)?;
r.define_control("HYD_1_ENGINE_PUMP", ControlSpec::checklist(), tag::HYD1_PUMP)?;
```

## State versus control

The split is the language's central safety property.

**State** is readable and never writable. `set hydraulic.2.pressure = 3000` is
an error (E0202), because the aircraft owns its own state: a procedure moves a
pump, and the simulation decides what that does to pressure. Allowing a
procedure to assign pressure directly would let a mistake in a checklist
silently rewrite the aircraft's physics.

**Controls** are actuable and never readable. `wait HYD_2_ELECTRIC_PUMP` is an
error (E0203); if a procedure needs to know whether the pump is actually
running, the aircraft should expose that as state
(`hydraulic.2.electric_pump_running`), because "the switch is on" and "the pump
is running" are different facts and conflating them is exactly the kind of
thing that bites at 3am.

## Control kinds

| Kind | Positions | Accepts |
| --- | --- | --- |
| `switch()` | `OFF`, `ON` | `set = OFF/ON`, `start`, `stop`, `check` |
| `valve()` | `CLOSED`, `OPEN` | `set = CLOSED/OPEN`, `open`, `close`, `check` |
| `selector([...])` | host-defined | `set = <name>`, `check` |
| `analog(min, max)` | — | `set = <number in range>`, `check` |
| `checklist()` | — | `check` only |

Position names are matched case-insensitively, so `set X = on` and
`set X = ON` are the same. A `selector` with positions named `OFF` and `ON`
behaves exactly like a `switch`; the distinct kinds exist so the host can
render the right widget and so the verbs mean something.

`checklist()` is for items with no actuator — a gauge to read, a placard to
confirm. Making it a control kind rather than a special case means `check`
works uniformly and the host still gets a tag to highlight.

## Tags

The third argument is an arbitrary `u32` that travels through the compiler
untouched and arrives on `Symbol::tag` / `Control::tag` at runtime. It is the
intended dispatch key: the aircraft matches on an integer, never on a string.

Nothing stops two symbols sharing a tag if that is what you want; the registry
does not interpret them.

## What the compiler does with it

* **Resolution.** An unknown name is E0201, with a suggestion when something
  close exists — the near-miss case is the most common authoring mistake and
  the one where a compiler earns its keep.
* **Type checking.** `hydraulic.2.pressure > 2500` type-checks against the
  registered `ValueType`; comparing a bool to a number is E0204.
* **Action validity.** `open` on a switch is E0206, with the control's actual
  positions listed. An unlisted selector position is E0205, likewise with the
  valid set. An analog value outside the registered range is E0207 — a compile
  error, not a runtime clamp, because a clamped value is a procedure that
  silently does something other than what it says.
* **Pruning.** Only symbols and controls a procedure actually references reach
  the output. A registry with two hundred entries and a database that uses
  eleven produces eleven records.

## Naming

The registry accepts any name matching the language's path syntax. The examples
use lowercase dotted paths for state (`hydraulic.2.pressure`) and upper snake
case for controls (`HYD_2_ELECTRIC_PUMP`), which makes the read/write split
visible at a glance in the source. Nothing enforces it.

`RegistryError` covers the two things that can go wrong at registration: a
duplicate name, and a name that is not a legal path.
