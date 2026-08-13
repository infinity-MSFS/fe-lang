# flight engineer lang

A domain-specific language, compiler and runtime for authoring the flight
engineer's procedures on a study-level aircraft.

Procedures are written as text, compiled ahead of time into a compact verified
binary, and executed inside the aircraft by a `no_std`, allocation-free
interpreter. There is no scripting at runtime: by the time the aircraft sees a
procedure it is bytecode whose every branch target, symbol reference and
control action has been checked.

```
  hydraulic.fe ─────┐
  electrical.fe ────┤
  fuel.fe ──────────┼─► fe-lang ─► fe-compiler ─► dc10.febin ─► include_bytes! ─► fe-runtime
  pressurization.fe ┘   (parse)    (check+emit)   (3.4 KB)      (in the addon)    (execute)
                                        ▲
                                 SymbolRegistry
                           (what this aircraft exposes)
```

## What a procedure looks like

```
procedure HYD_2_LOW_PRESSURE {
    name        "Hydraulic System 2 Low Pressure"
    description "Restore system 2 pressure using the electric standby pump."
    category    abnormal
    priority    80
    revision    3

    trigger hydraulic.2.pressure < 1800 && engine.2.running
    require engine.2.running "engine 2 must be running"

    check HYD_2_ENGINE_PUMP

    if electrical.ac_bus.2.powered {
        call HYD_2_ELECTRIC_PUMP_START
    } else {
        notify "AC bus 2 unpowered - electric pump unavailable"
        call ELEC_BUS_2_RESTORE
    }

    wait hydraulic.2.pressure > 2500 timeout 30s

    if hydraulic.2.pressure > 2500 {
        notify "Hydraulic system 2 pressure restored"
        complete
    }

    notify "Hydraulic system 2 pressure remains low"
    close HYD_2_ISOLATION_VALVE
    fail "system 2 pressure not restored"
}
```

## The three crates

| Crate         | Role                                           | Constraints                                           |
| ------------- | ---------------------------------------------- | ----------------------------------------------------- |
| `fe-lang`     | lexer, parser, AST, diagnostics                | knows nothing about aircraft                          |
| `fe-compiler` | analysis, IR, code generation, binary emission | a **library**: no CLI, no filesystem, no environment  |
| `fe-runtime`  | verification and execution                     | `no_std`, no allocation, no dependencies, no `unsafe` |

`fe-compiler` depends on `fe-runtime` so the two cannot disagree about the
format — and so the compiler can load its own output through the aircraft's
verifier before returning it.

## Building a database

There is no compiler executable, on purpose: reading files, globbing
directories and knowing what the aircraft's symbols are called all belong to a
specific aircraft's build, not to the language.

```rust
use fe_compiler::{compile, ControlSpec, SourceUnit, SymbolRegistry, ValueType};

let mut registry = SymbolRegistry::new();
registry.define_state("hydraulic.2.pressure", ValueType::F32, tag::HYD2_PRESSURE)?;
registry.define_control("HYD_2_ELECTRIC_PUMP", ControlSpec::switch(), tag::HYD2_PUMP)?;

let units = vec![SourceUnit::new("hydraulic.fe", std::fs::read_to_string("hydraulic.fe")?)];

match compile(&units, &registry) {
    Ok(compiled) => std::fs::write("dc10.febin", compiled.as_bytes())?,
    Err(error) => eprint!("{}", error.render(&units)),
}
```

The whole build tool for the example aircraft is
[`fe-compiler/examples/build_database.rs`](fe-compiler/examples/build_database.rs),
in about a hundred lines:

```
cargo run -p fe-compiler --example build_database -- examples/dc10 dc10.febin
```

## Running it in a client

[`fe-compiler/examples/run_aircraft.rs`](fe-compiler/examples/run_aircraft.rs)
is the integration end to end in one file: a small hydraulic and electrical
simulation, the two trait implementations, and a 20 Hz frame loop. It runs the
same procedure against two aircraft states and shows it succeeding once and
isolating the system the other time.

```
cargo run -p fe-compiler --example run_aircraft
```

## Running it

```rust
static PROCEDURES: &[u8] = include_bytes!("dc10.febin");

let db = ProcedureDatabase::from_bytes(PROCEDURES)?;   // validates everything, once
let mut exec = ProcedureExecutor::new(db.get_procedure("HYD_2_LOW_PRESSURE").unwrap());

// every simulation frame:
match exec.tick(&aircraft, &mut controls, dt_ms) {
    Tick::Waiting { .. } => { /* parked on a `wait` */ }
    Tick::Completed => { /* done */ }
    _ => {}
}
```

The host implements two traits — `AircraftState::read` and
`AircraftControls::execute` — and nothing else.

## Properties worth knowing about

- **A tick always terminates.** Bytecode branches are forward-only and the
  verifier enforces it; `wait` re-entry is performed by the executor and always
  ends the tick. In an MSFS gauge, a callback that does not return has frozen
  the aircraft, so this is a structural property rather than a promise.
  ([verification.md](docs/verification.md))
- **Malformed data cannot panic.** `from_bytes` validates structure _and_
  bytecode before anything can run. The fuzz suite flips every single bit in a
  real database and asserts that whatever still verifies also executes safely.
- **Builds are reproducible.** The same sources and registry produce
  byte-identical output, on any machine, in any file order.
- **A procedure can only touch what the host registered.** No opcode calls a
  host function or names anything outside the database, and the host can refuse
  any action.

## Documentation

|                                                 |                                              |
| ----------------------------------------------- | -------------------------------------------- |
| [architecture.md](docs/architecture.md)         | the crates and why they are shaped that way  |
| [language.md](docs/language.md)                 | the language reference and grammar           |
| [semantics.md](docs/semantics.md)               | what analysis checks                         |
| [ir.md](docs/ir.md)                             | the intermediate representation              |
| [bytecode.md](docs/bytecode.md)                 | the instruction set                          |
| [binary-format.md](docs/binary-format.md)       | the `.febin` layout                          |
| [verification.md](docs/verification.md)         | why a tick terminates                        |
| [runtime.md](docs/runtime.md)                   | the execution model                          |
| [host-integration.md](docs/host-integration.md) | embedding it in an aircraft                  |
| [symbols.md](docs/symbols.md)                   | the symbol registry                          |
| [diagnostics.md](docs/diagnostics.md)           | every code                                   |
| [design-decisions.md](docs/design-decisions.md) | the arguments, including the noes            |
| [extending.md](docs/extending.md)               | adding a statement, opcode or format version |

## Tests

```
cargo test
```

162 tests: lexer, parser, semantics, code generation, binary format,
end-to-end execution against a fake aircraft, a seeded mutation fuzzer, and a
runtime suite that builds databases by hand with no compiler involved.

## The examples are not real procedures

`examples/dc10/*.fe` are illustrative. They exercise the language; they are not
authentic DC-10 procedures and must not be used for training or for operating
an aircraft.
