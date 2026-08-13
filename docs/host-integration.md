# Integrating with an aircraft

The runtime knows how to read a symbol and how to ask for an action. Everything
else — what a pump does, whether it is powered, how pressure builds — lives on
your side of two traits.

## The two traits

```rust
impl AircraftState for MyAircraft {
    fn read(&self, symbol: Symbol<'_>) -> Value {
        // symbol.tag is the integer you registered. Dispatch on it.
        match symbol.tag {
            tag::HYD2_PRESSURE => Value::F32(self.hydraulics[1].pressure),
            tag::ENG2_RUNNING  => Value::Bool(self.engines[1].running),
            _ => Value::Bool(false),
        }
    }
}

impl AircraftControls for MyAircraft {
    fn execute(&mut self, action: Action<'_>) -> ActionResult {
        match action {
            Action::Set { control, value } => self.actuate(control.tag, value),
            Action::Check { control } => { self.highlight(control.tag); ActionResult::Accepted }
        }
    }

    fn on_event(&mut self, event: ProcedureEvent<'_>) {
        if let ProcedureEvent::Notification { message } = event {
            self.eicas.push(message);
        }
    }
}
```

`read` is called once per symbol reference per evaluation, including every tick
a `wait` is parked, so make it a lookup rather than a computation.

## Tags, not strings

The `tag` on a symbol or control is an arbitrary `u32` you supplied when
registering it. It exists so the aircraft never has to match on a string in a
hot path — and so renaming `hydraulic.2.pressure` to `hyd.2.press` in the
registry and the procedures is a one-line change that does not touch the
systems code.

In a real addon these are best generated as a shared enum so the registry and
the systems code cannot disagree.

## Build integration

The compiler is a library with no filesystem access, so the build step is
yours. In a `build.rs`:

```rust
let units: Vec<SourceUnit> = /* read your .fe files */;
match fe_compiler::compile(&units, &aircraft_registry()) {
    Ok(compiled) => std::fs::write(out_dir.join("procedures.febin"), compiled.as_bytes())?,
    Err(error) => panic!("{}", error.render(&units)),
}
```

and in the aircraft:

```rust
static PROCEDURES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/procedures.febin"));
```

`fe-compiler/examples/build_database.rs` is a complete working version of this,
including diagnostics rendering and a disassembly dump, and
`fe-compiler/examples/run_aircraft.rs` is the client half — a small simulation,
both trait implementations and a frame loop, in one file.

`compile` returns warnings on success; a build that treats them as errors is a
one-line check on `compiled.warnings()`.

## Notes for MSFS WASM

* `fe-runtime` is `no_std`, allocation-free and dependency-free, so it links
  into a gauge module without pulling in an allocator or a panic machinery you
  would then have to justify.
* Embed the database with `include_bytes!`. `ProcedureDatabase` borrows it, so
  there is no load-time copy and no heap traffic — validation walks the bytes
  in place.
* Validate once, at gauge init, and keep the `ProcedureDatabase` (it is `Copy`
  and about six words). Do not re-parse per frame.
* `tick` returning promptly is a hard requirement in a gauge callback; see
  [`verification.md`](verification.md) for why it does.
* `disassemble` writes through `core::fmt::Write`, so it can go to the MSFS
  console for debugging without a `String`.

## Keeping the read and write sides apart

`tick` takes `&impl AircraftState` and `&mut impl AircraftControls`, so the two
cannot be the same object. That is worth leaning into rather than working
around: have `execute` *queue* commands and apply them when the aircraft next
integrates.

```rust
panel.refresh(&aircraft);              // what the panel needs to refuse actions
aircraft.integrate(dt_ms);             // the aircraft simulates itself
let tick = executor.tick(&aircraft, &mut panel, dt_ms);
aircraft.apply(&mut panel.commands);   // then apply what the procedure asked for
```

A procedure then cannot mutate the aircraft halfway through evaluating a
condition, and every symbol read within one tick sees one consistent frame.
`run_aircraft.rs` is built this way.

## Running several procedures

One executor per running procedure. They share the database by copying it,
which is a slice and a few offsets:

```rust
struct Panel<'db> {
    checklist: ProcedureExecutor<'db>,
    abnormal: Option<ProcedureExecutor<'db>>,
}
```

Nothing is shared between them, so a checklist and an abnormal procedure can
run simultaneously without interfering — a case the test suite covers directly.

## Reporting to the crew

`ProcedureEvent` is deliberately vague about presentation. `Notification`
carries a string; whether that becomes an aural warning, an EICAS line, a
tooltip on the flight engineer's panel, or a line in a log is the host's
decision, and the language has no way to express a preference.
