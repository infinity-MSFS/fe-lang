# The procedure language

A `.fe` file is a list of procedures. There is no import statement, no
variables, no functions, no arithmetic, and no loops. A procedure reads
aircraft state, moves controls, waits for things to happen, and finishes.

## A complete example

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

## Lexical structure

* **Comments** — `// to end of line` and `/* block, non-nesting */`.
* **Identifiers** — letters, digits and `_`, not starting with a digit.
* **Paths** — identifiers joined by `.`, with numeric segments allowed:
  `hydraulic.2.pressure`. A path is the registry lookup key, written exactly as
  the host registered it.
* **Strings** — double-quoted, with `\"`, `\\`, `\n`, `\t` escapes.
* **Numbers** — `1800`, `22.5`, `-1.5`. All numbers are 32-bit floats.
* **Durations** — a number with a unit: `500ms`, `30s`, `5m`. Durations are
  only valid after `timeout`.
* **Keywords are contextual.** `name`, `check`, `set`, `open` and the rest are
  keywords at the start of a statement and ordinary identifiers everywhere
  else, so `hydraulic.name.check` is a legal path. Aircraft symbol names were
  chosen by engineers, not by this language, and it is not their job to avoid
  our vocabulary.

## Declarations

```
procedure IDENTIFIER { metadata... step... }
```

The identifier is how the host and other procedures refer to it
(`db.get_procedure("HYD_2_LOW_PRESSURE")`, `call HYD_2_LOW_PRESSURE`). It is
not shown to the crew. Identifiers share one flat namespace across every source
file compiled together; a duplicate is an error.

## Metadata

Metadata must appear before the first step.

| Entry | Required | Value |
| --- | --- | --- |
| `name` | yes | string — the crew-facing title |
| `category` | yes | `normal`, `abnormal`, `emergency` or `reference` |
| `description` | no | string |
| `priority` | no | integer 0–255, default 0 |
| `revision` | no | integer 0–65535, default 0 |
| `trigger` | no | condition — see below |
| `require` | no, repeatable | condition, optional message |

`trigger` is a *pure* condition a host can evaluate every frame, with no side
effects, to decide whether a procedure has become relevant. Evaluating it does
not start the procedure:

```rust
if ProcedureExecutor::evaluate_trigger(&procedure, &aircraft)? == Some(true) {
    // offer it to the crew
}
```

`require` is a precondition. It is checked when the procedure starts, before
any step runs, and a false one fails the procedure without touching a single
control. The optional message is what the host reports:

```
require electrical.ac_bus.2.powered "AC bus 2 must be powered"
```

## Steps

### `check CONTROL`

Ask the crew (or the host) to verify an item. Implies no state change. This is
the only thing a `checklist` control accepts.

### `set CONTROL = VALUE`

Move a control. `VALUE` is a position name for switches, valves and selectors,
or a number for analog controls:

```
set HYD_2_ELECTRIC_PUMP = ON
set FUEL_XFEED_SELECTOR = TANK_3_TO_1
set FUEL_PUMP_PRESSURE_TARGET = 22.5
```

Position names are matched case-insensitively. An analog value outside the
range the host registered is a compile error, not a runtime clamp.

### `start` / `stop` / `open` / `close`

Sugar for the four positions that come up constantly:

| Verb | Equivalent |
| --- | --- |
| `start X` | `set X = ON` |
| `stop X` | `set X = OFF` |
| `open X` | `set X = OPEN` |
| `close X` | `set X = CLOSED` |

Applying a verb to a control without that position is an error, with the
control's actual positions listed in the message.

### `notify "message"`

Send a string to the host. The runtime has no opinion about whether that
becomes an aural warning, an EICAS line or a log entry.

### `call PROCEDURE`

Run another procedure to completion, then continue. Calls may nest up to
`MAX_CALL_DEPTH` (8) and may not form a cycle; both are compile errors, so the
executor's fixed frame array can never overflow.

A `fail` inside a callee stops the caller too. A `wait` inside a callee parks
the whole stack.

### `wait CONDITION [timeout DURATION [else fail]]`

Yield until the condition holds. Each simulation tick re-evaluates it; nothing
blocks.

```
wait hydraulic.2.pressure > 2500                    // forever
wait hydraulic.2.pressure > 2500 timeout 30s        // give up, carry on
wait hydraulic.2.pressure > 2500 timeout 30s else fail
```

Without `else fail`, a timeout emits `ProcedureEvent::Timeout { continued: true }`
and execution continues with the next step — which is why the example above
re-tests the pressure afterwards rather than assuming it succeeded.

### `if CONDITION { ... } [else if CONDITION { ... }] [else { ... }]`

Ordinary branching. Nesting is limited to 16 deep.

### `complete [when CONDITION [timeout DURATION]]`

Finish successfully. `complete when C` waits for `C` first; if a `timeout` is
given and expires, the procedure **fails** rather than completing. A completion
criterion that never came true has not been met, and reporting otherwise would
be the most dangerous lie this language could tell.

Falling off the end of a body also completes, so a checklist need not end with
`complete`.

### `fail ["message"]`

Stop unsuccessfully, with an optional reason for the host.

## Expressions

Conditions are boolean. There are two types — number and boolean — and no
conversions between them.

| Precedence | Operators |
| --- | --- |
| lowest | `\|\|` |
| | `&&` |
| | `<` `<=` `>` `>=` `==` `!=` |
| highest | `!`, parentheses |

Comparisons do not chain: `a < b < c` is an error, not a subtle bug.
Booleans support only `==` and `!=`. Comparing two floats with `==` compiles
but warns.

There is deliberately no arithmetic. A procedure that needs a derived quantity
should read it from a symbol the aircraft computes, where it can be tested and
where the flight engineer's manual can point at it. See
[`design-decisions.md`](design-decisions.md).

## Grammar

```
unit        := procedure*
procedure   := "procedure" IDENT "{" metadata* step* "}"

metadata    := "name" STRING
             | "description" STRING
             | "category" IDENT
             | "priority" NUMBER
             | "revision" NUMBER
             | "trigger" expr
             | "require" expr STRING?

step        := "check" path
             | "set" path "=" (IDENT | NUMBER)
             | ("start" | "stop" | "open" | "close") path
             | "notify" STRING
             | "call" IDENT
             | "wait" expr timeout?
             | "if" expr block ("else" (if_step | block))?
             | "complete" ("when" expr timeout?)?
             | "fail" STRING?

timeout     := "timeout" DURATION ("else" "fail")?
block       := "{" step* "}"

expr        := or
or          := and ("||" and)*
and         := cmp ("&&" cmp)*
cmp         := unary (("<" | "<=" | ">" | ">=" | "==" | "!=") unary)?
unary       := "!" unary | primary
primary     := "true" | "false" | NUMBER | "-" NUMBER | path | "(" expr ")"
path        := IDENT ("." (IDENT | NUMBER))*
```
