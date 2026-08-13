# The execution model

```rust
static PROCEDURES: &[u8] = include_bytes!("../procedures/dc10.febin");

let db = ProcedureDatabase::from_bytes(PROCEDURES)?;
let mut exec = ProcedureExecutor::new(db.get_procedure("HYD_2_LOW_PRESSURE").unwrap());

// every simulation frame:
match exec.tick(&aircraft, &mut controls, dt_ms) {
    Tick::Running => {}
    Tick::Waiting { elapsed_ms, timeout_ms } => { /* show a spinner */ }
    Tick::Completed => { /* green tick */ }
    Tick::Failed => { /* the host already got a Failed event with the reason */ }
    Tick::Idle => { /* already finished */ }
}
```

## Properties

* **No allocation.** An executor is a plain value with a fixed frame array, a
  fixed operand stack and a few integers. It can live in a struct the aircraft
  owns.
* **No blocking, no async, no threads.** `tick` does a bounded amount of work
  and returns.
* **No interior mutability, no globals.** Two executors over one database do
  not interact; the database is `Copy`.
* **No panics.** Verification makes malformed data unreachable; anything the
  interpreter still cannot do becomes `FailReason::Runtime(RuntimeError)`.

## Lifecycle

```
Ready ──tick──► Running ──┬──► Completed
                  │  ▲    ├──► Failed
                  ▼  │    └──► Cancelled  (via cancel())
               Waiting
```

`reset()` returns to `Ready` from any state, so a procedure can be re-run
without reallocating anything. `is_finished()` is true for the three terminal
states, and ticking a finished executor returns `Tick::Idle` and does nothing.

## Ticks and `dt_ms`

`dt_ms` is the elapsed simulation time since the previous tick. It is consumed
by exactly one thing: `wait` timeouts. Everything else is driven by state, not
by the clock, so a paused or time-accelerated sim behaves sensibly as long as
the host passes simulation time rather than wall time.

A tick runs until the procedure ends, parks on a `wait`, or hits its
instruction budget. It does **not** stop after one step: a run of `notify`,
`set` and `check` steps all happen in the same tick, which is what a crew
would expect from a checklist.

## Waiting

When a `wait` condition is false the executor:

1. accumulates `dt_ms` into the wait's elapsed time;
2. emits `ProcedureEvent::Waiting { elapsed_ms, timeout_ms }`;
3. returns `Tick::Waiting { .. }`.

Next tick it re-enters the same `AWAIT` and re-evaluates. If the timeout
expires, it emits `Timeout { continued }` and either continues past the wait or
fails, depending on `else fail`.

A wait inside a `call` parks the whole stack: `current_procedure()` reports the
callee, `procedure()` still reports the root.

## Actions and host authority

Every `set` and `check` becomes an `Action` handed to
`AircraftControls::execute`. The host has the final say: returning
`ActionResult::Rejected(code)` fails the procedure with
`FailReason::ActionRejected { control, code }`. A jammed valve stops the
procedure rather than being quietly assumed to have moved.

Actions carry resolved data — the control's name, kind, host tag, and for a
position both its index and its label — so a host can dispatch on an integer
and a UI can render a string without either of them re-reading the database.

## Events

| Event | When |
| --- | --- |
| `Started` | the first tick of a procedure |
| `Entered` / `Returned` | a `call` begins / ends |
| `ActionRequested` | immediately before `execute` |
| `Notification` | a `notify` step |
| `Waiting` | every tick spent parked on a `wait` |
| `Timeout { continued }` | a `wait` expired |
| `Completed` | the procedure finished |
| `Failed { reason }` | it did not |

`on_event` has a default empty implementation, so a host that only cares about
actions need not write it.

## Triggers

`ProcedureExecutor::evaluate_trigger` is an associated function, not a method:
it needs no executor, allocates nothing, and has no side effects. A monitor can
call it for every armed procedure every frame and use the result to decide what
to offer the crew.

```rust
for procedure in db.procedures() {
    if ProcedureExecutor::evaluate_trigger(&procedure, &aircraft)? == Some(true) {
        // this procedure has become relevant
    }
}
```

It returns `Ok(None)` for a procedure with no `trigger`.

## Errors

`RuntimeError` covers the things a corrupt database could ask for —
`InvalidOpcode`, `StackUnderflow`, `CallDepthExceeded`, `NoActiveWait` — plus
two that a *valid* database can still produce:

* `TypeMismatch`, when the host returns a value whose type disagrees with the
  symbol's registered type. The runtime fails rather than coercing; a
  `Value::Bool` where a pressure was promised is a bug in the aircraft, and
  silently reading it as `1.0` would hide it.
* `StepLimitExceeded`, when a tick exhausts its instruction budget.

All of them arrive as `FailReason::Runtime` on `on_event`, and the executor
moves to `Failed`.
