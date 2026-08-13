//! End-to-end tests: source text in, aircraft behaviour out.
//!
//! These are the tests that would catch a mistake anywhere in the chain —
//! lexer, parser, analysis, code generation, binary layout, verifier,
//! interpreter — because every one of them exercises all of it.

mod support;

use support::{FakeAircraft, Recorder, tag};

use fe_runtime::{ExecutionState, ProcedureDatabase, ProcedureExecutor, Tick, Value};

/// Compile one procedure against the example registry and hand back its bytes.
fn build(source: &str) -> Vec<u8> {
    support::compile_source(source)
        .unwrap_or_else(|rendered| panic!("{rendered}"))
        .into_bytes()
}

/// Drive an executor until it settles, letting the caller move the aircraft
/// between ticks. Returns the final tick.
///
/// `dt_ms` is fixed at 50ms — a 20Hz systems tick, which is roughly what an
/// MSFS gauge update looks like.
fn run<F>(
    db: &ProcedureDatabase<'_>,
    id: &str,
    aircraft: &mut FakeAircraft,
    recorder: &mut Recorder,
    max_ticks: usize,
    mut between: F,
) -> Tick
where
    F: FnMut(usize, &mut FakeAircraft),
{
    let procedure = db.get_procedure(id).expect("no such procedure");
    let mut exec = ProcedureExecutor::new(procedure);
    let mut last = Tick::Idle;
    for tick in 0..max_ticks {
        between(tick, aircraft);
        last = exec.tick(aircraft, recorder, 50);
        if exec.is_finished() {
            return last;
        }
    }
    panic!("procedure {id} did not settle in {max_ticks} ticks (last tick: {last:?})");
}

// ---------------------------------------------------------------------------
// The headline scenario
// ---------------------------------------------------------------------------

#[test]
fn low_pressure_is_restored_by_the_standby_pump() {
    let bytes = support::compile_examples();
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();

    let mut aircraft = FakeAircraft::new();
    aircraft.set_f32(tag::HYD2_PRESSURE, 1500.0);
    let mut recorder = Recorder::new();

    let result = run(
        &db,
        "HYD_2_LOW_PRESSURE",
        &mut aircraft,
        &mut recorder,
        200,
        |tick, aircraft| {
            // The simulation reacts: the pump spins up, then pressure builds.
            if tick == 3 {
                aircraft.set_bool(tag::HYD2_ELECTRIC_PUMP_RUNNING, true);
            }
            if tick == 6 {
                aircraft.set_f32(tag::HYD2_PRESSURE, 2800.0);
            }
        },
    );

    assert_eq!(result, Tick::Completed);
    assert!(recorder.completed);
    assert!(
        recorder
            .actions
            .iter()
            .any(|a| a == "set HYD_2_ELECTRIC_PUMP = ON")
    );
    assert!(
        recorder
            .notifications
            .iter()
            .any(|n| n.contains("pressure restored"))
    );
}

#[test]
fn the_same_procedure_fails_when_the_aircraft_does_not_respond() {
    // Identical inputs except that pressure never recovers. The 30s timeout
    // is not `else fail`, so the procedure continues and reaches its own
    // explicit failure — which is the behaviour the source asks for.
    let bytes = support::compile_examples();
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();

    let mut aircraft = FakeAircraft::new();
    aircraft.set_f32(tag::HYD2_PRESSURE, 1500.0);
    let mut recorder = Recorder::new();

    let result = run(
        &db,
        "HYD_2_LOW_PRESSURE",
        &mut aircraft,
        &mut recorder,
        2000,
        |tick, aircraft| {
            if tick == 3 {
                aircraft.set_bool(tag::HYD2_ELECTRIC_PUMP_RUNNING, true);
            }
        },
    );

    assert_eq!(result, Tick::Failed);
    assert!(
        recorder
            .actions
            .iter()
            .any(|a| a == "set HYD_2_ISOLATION_VALVE = CLOSED")
    );
    assert!(recorder.failed.is_some());
}

// ---------------------------------------------------------------------------
// Triggers
// ---------------------------------------------------------------------------

#[test]
fn a_trigger_is_evaluated_without_running_anything() {
    let bytes = support::compile_examples();
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let procedure = db.get_procedure("HYD_2_LOW_PRESSURE").unwrap();

    let mut aircraft = FakeAircraft::new();
    assert_eq!(
        ProcedureExecutor::evaluate_trigger(&procedure, &aircraft).unwrap(),
        Some(false)
    );

    aircraft.set_f32(tag::HYD2_PRESSURE, 1200.0);
    assert_eq!(
        ProcedureExecutor::evaluate_trigger(&procedure, &aircraft).unwrap(),
        Some(true)
    );

    // Both halves of the `&&` matter: engine 2 shut down means no trigger.
    aircraft.set_bool(tag::ENG2_RUNNING, false);
    assert_eq!(
        ProcedureExecutor::evaluate_trigger(&procedure, &aircraft).unwrap(),
        Some(false)
    );
}

#[test]
fn a_procedure_without_a_trigger_reports_none() {
    let bytes = support::compile_examples();
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let procedure = db.get_procedure("HYD_ALL_SYSTEMS_CHECK").unwrap();
    let aircraft = FakeAircraft::new();
    assert_eq!(
        ProcedureExecutor::evaluate_trigger(&procedure, &aircraft).unwrap(),
        None
    );
}

// ---------------------------------------------------------------------------
// Preconditions, branching, actions
// ---------------------------------------------------------------------------

#[test]
fn a_failed_precondition_stops_before_touching_anything() {
    let bytes = build(
        r#"
        procedure P {
            name "P"
            category abnormal
            require electrical.ac_bus.2.powered "AC bus 2 must be powered"
            set HYD_2_ELECTRIC_PUMP = ON
            complete
        }
        "#,
    );
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let mut aircraft = FakeAircraft::new();
    aircraft.set_bool(tag::AC_BUS2_POWERED, false);
    let mut recorder = Recorder::new();

    let result = run(&db, "P", &mut aircraft, &mut recorder, 10, |_, _| {});
    assert_eq!(result, Tick::Failed);
    assert!(
        recorder.actions.is_empty(),
        "a precondition failure must not move a control: {:?}",
        recorder.actions
    );
    assert!(recorder.failed.unwrap().contains("Precondition"));
}

#[test]
fn the_else_branch_runs_when_the_condition_is_false() {
    let source = r#"
        procedure P {
            name "P"
            category normal
            if hydraulic.1.pressure > hydraulic.3.pressure {
                notify "one"
            } else {
                notify "three"
            }
            complete
        }
    "#;
    let bytes = build(source);
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();

    for (p1, p3, expected) in [(3000.0, 1000.0, "one"), (1000.0, 3000.0, "three")] {
        let mut aircraft = FakeAircraft::new();
        aircraft.set_f32(tag::HYD1_PRESSURE, p1);
        aircraft.set_f32(tag::HYD3_PRESSURE, p3);
        let mut recorder = Recorder::new();
        run(&db, "P", &mut aircraft, &mut recorder, 10, |_, _| {});
        assert_eq!(recorder.notifications, vec![expected.to_string()]);
    }
}

#[test]
fn nested_conditions_pick_exactly_one_path() {
    let bytes = build(
        r#"
        procedure P {
            name "P"
            category normal
            if generator.2.available {
                if electrical.ac_bus.1.powered { notify "a" } else { notify "b" }
            } else if electrical.ac_bus.1.powered {
                notify "c"
            } else {
                notify "d"
            }
            complete
        }
        "#,
    );
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    for (generator, bus1, expected) in [
        (true, true, "a"),
        (true, false, "b"),
        (false, true, "c"),
        (false, false, "d"),
    ] {
        let mut aircraft = FakeAircraft::new();
        aircraft.set_bool(tag::GEN2_AVAILABLE, generator);
        aircraft.set_bool(tag::AC_BUS1_POWERED, bus1);
        let mut recorder = Recorder::new();
        run(&db, "P", &mut aircraft, &mut recorder, 20, |_, _| {});
        assert_eq!(
            recorder.notifications,
            vec![expected.to_string()],
            "gen={generator} bus1={bus1}"
        );
    }
}

#[test]
fn a_rejected_action_fails_the_procedure_immediately() {
    // The aircraft has the final say. A jammed valve must stop the procedure,
    // not be quietly assumed to have moved.
    let bytes = build(
        r#"
        procedure P {
            name "P"
            category normal
            set HYD_2_ELECTRIC_PUMP = ON
            notify "should not be reached"
            complete
        }
        "#,
    );
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let mut aircraft = FakeAircraft::new();
    let mut recorder = Recorder::new().reject("HYD_2_ELECTRIC_PUMP");

    let result = run(&db, "P", &mut aircraft, &mut recorder, 10, |_, _| {});
    assert_eq!(result, Tick::Failed);
    assert!(recorder.notifications.is_empty());
    let failure = recorder.failed.unwrap();
    assert!(failure.contains("ActionRejected"), "{failure}");
    assert!(
        failure.contains("7"),
        "the host's reason code is reported: {failure}"
    );
}

#[test]
fn analog_values_reach_the_host_unchanged() {
    let bytes = build(
        r#"
        procedure P {
            name "P"
            category normal
            set FUEL_PUMP_PRESSURE_TARGET = 22.5
            complete
        }
        "#,
    );
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let mut aircraft = FakeAircraft::new();
    let mut recorder = Recorder::new();
    run(&db, "P", &mut aircraft, &mut recorder, 10, |_, _| {});
    assert_eq!(
        recorder.actions,
        vec!["set FUEL_PUMP_PRESSURE_TARGET = 22.5".to_string()]
    );
}

#[test]
fn selector_positions_arrive_by_name_and_index() {
    let bytes = build(
        r#"
        procedure P {
            name "P"
            category normal
            set FUEL_XFEED_SELECTOR = TANK_3_TO_1
            complete
        }
        "#,
    );
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let mut aircraft = FakeAircraft::new();
    let mut recorder = Recorder::new();
    run(&db, "P", &mut aircraft, &mut recorder, 10, |_, _| {});
    assert_eq!(
        recorder.actions,
        vec!["set FUEL_XFEED_SELECTOR = TANK_3_TO_1".to_string()]
    );
}

// ---------------------------------------------------------------------------
// Waiting
// ---------------------------------------------------------------------------

#[test]
fn a_wait_yields_every_tick_until_its_condition_holds() {
    let bytes = build(
        r#"
        procedure P {
            name "P"
            category normal
            wait hydraulic.2.pressure > 2500
            notify "up"
            complete
        }
        "#,
    );
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let mut aircraft = FakeAircraft::new();
    aircraft.set_f32(tag::HYD2_PRESSURE, 1000.0);
    let mut recorder = Recorder::new();
    let procedure = db.get_procedure("P").unwrap();
    let mut exec = ProcedureExecutor::new(procedure);

    for _ in 0..5 {
        let tick = exec.tick(&aircraft, &mut recorder, 50);
        assert!(matches!(tick, Tick::Waiting { .. }), "{tick:?}");
        assert_eq!(exec.state(), ExecutionState::Waiting);
    }
    // Elapsed time accumulates across ticks.
    match exec.tick(&aircraft, &mut recorder, 50) {
        Tick::Waiting { elapsed_ms, .. } => assert_eq!(elapsed_ms, 250),
        other => panic!("expected to still be waiting, got {other:?}"),
    }

    aircraft.set_f32(tag::HYD2_PRESSURE, 3000.0);
    assert_eq!(exec.tick(&aircraft, &mut recorder, 50), Tick::Completed);
    assert_eq!(recorder.notifications, vec!["up".to_string()]);
}

#[test]
fn a_wait_reports_its_timeout_to_the_host() {
    let bytes = build(
        r#"
        procedure P {
            name "P"
            category normal
            wait hydraulic.2.pressure > 2500 timeout 10s
            complete
        }
        "#,
    );
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let mut aircraft = FakeAircraft::new();
    aircraft.set_f32(tag::HYD2_PRESSURE, 1000.0);
    let mut recorder = Recorder::new();
    let procedure = db.get_procedure("P").unwrap();
    let mut exec = ProcedureExecutor::new(procedure);
    match exec.tick(&aircraft, &mut recorder, 50) {
        Tick::Waiting { timeout_ms, .. } => assert_eq!(timeout_ms, Some(10_000)),
        other => panic!("expected waiting, got {other:?}"),
    }
}

#[test]
fn a_plain_timeout_continues_and_a_fail_timeout_does_not() {
    let source = |clause: &str| {
        format!(
            r#"
            procedure P {{
                name "P"
                category normal
                wait hydraulic.2.pressure > 2500 timeout 1s{clause}
                notify "continued"
                complete
            }}
            "#
        )
    };

    // `timeout 1s` — the wait gives up, the procedure carries on.
    let bytes = build(&source(""));
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let mut aircraft = FakeAircraft::new();
    aircraft.set_f32(tag::HYD2_PRESSURE, 1000.0);
    let mut recorder = Recorder::new();
    let result = run(&db, "P", &mut aircraft, &mut recorder, 100, |_, _| {});
    assert_eq!(result, Tick::Completed);
    assert_eq!(recorder.notifications, vec!["continued".to_string()]);
    assert!(
        recorder
            .events
            .iter()
            .any(|e| e == "timeout (continued=true)")
    );

    // `timeout 1s else fail` — it does not.
    let bytes = build(&source(" else fail"));
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let mut aircraft = FakeAircraft::new();
    aircraft.set_f32(tag::HYD2_PRESSURE, 1000.0);
    let mut recorder = Recorder::new();
    let result = run(&db, "P", &mut aircraft, &mut recorder, 100, |_, _| {});
    assert_eq!(result, Tick::Failed);
    assert!(recorder.notifications.is_empty());
    assert_eq!(recorder.failed.as_deref(), Some("Timeout"));
}

#[test]
fn complete_when_never_reports_success_on_timeout() {
    // The one place where `timeout` must imply failure: a procedure that
    // times out waiting for its completion criterion has not completed.
    let bytes = build(
        r#"
        procedure P {
            name "P"
            category normal
            check HYD_2_ENGINE_PUMP
            complete when hydraulic.2.pressure > 2500 timeout 1s
        }
        "#,
    );
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let mut aircraft = FakeAircraft::new();
    aircraft.set_f32(tag::HYD2_PRESSURE, 1000.0);
    let mut recorder = Recorder::new();
    let result = run(&db, "P", &mut aircraft, &mut recorder, 100, |_, _| {});
    assert_eq!(result, Tick::Failed);
    assert!(!recorder.completed);
}

// ---------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------

#[test]
fn a_call_runs_the_callee_and_returns() {
    let bytes = build(
        r#"
        procedure OUTER {
            name "Outer"
            category normal
            notify "before"
            call INNER
            notify "after"
            complete
        }
        procedure INNER {
            name "Inner"
            category reference
            notify "inside"
        }
        "#,
    );
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let mut aircraft = FakeAircraft::new();
    let mut recorder = Recorder::new();
    run(&db, "OUTER", &mut aircraft, &mut recorder, 20, |_, _| {});
    assert_eq!(
        recorder.notifications,
        vec![
            "before".to_string(),
            "inside".to_string(),
            "after".to_string()
        ]
    );
    assert!(recorder.events.iter().any(|e| e == "entered INNER"));
    assert!(recorder.events.iter().any(|e| e == "returned INNER"));
}

#[test]
fn a_wait_inside_a_call_parks_the_whole_stack() {
    let bytes = build(
        r#"
        procedure OUTER {
            name "Outer"
            category normal
            call INNER
            notify "after"
            complete
        }
        procedure INNER {
            name "Inner"
            category reference
            wait hydraulic.2.pressure > 2500
        }
        "#,
    );
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let mut aircraft = FakeAircraft::new();
    aircraft.set_f32(tag::HYD2_PRESSURE, 1000.0);
    let mut recorder = Recorder::new();
    let procedure = db.get_procedure("OUTER").unwrap();
    let mut exec = ProcedureExecutor::new(procedure);

    assert!(matches!(
        exec.tick(&aircraft, &mut recorder, 50),
        Tick::Waiting { .. }
    ));
    assert_eq!(exec.current_procedure().unwrap().id, "INNER");
    assert!(recorder.notifications.is_empty());

    aircraft.set_f32(tag::HYD2_PRESSURE, 3000.0);
    assert_eq!(exec.tick(&aircraft, &mut recorder, 50), Tick::Completed);
    assert_eq!(recorder.notifications, vec!["after".to_string()]);
}

#[test]
fn a_failure_inside_a_call_stops_the_caller_too() {
    let bytes = build(
        r#"
        procedure OUTER {
            name "Outer"
            category normal
            call INNER
            notify "unreachable"
            complete
        }
        procedure INNER {
            name "Inner"
            category reference
            fail "inner said no"
        }
        "#,
    );
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let mut aircraft = FakeAircraft::new();
    let mut recorder = Recorder::new();
    let result = run(&db, "OUTER", &mut aircraft, &mut recorder, 20, |_, _| {});
    assert_eq!(result, Tick::Failed);
    assert!(recorder.notifications.is_empty());
    assert!(recorder.failed.unwrap().contains("inner said no"));
}

// ---------------------------------------------------------------------------
// Executor lifecycle
// ---------------------------------------------------------------------------

#[test]
fn a_finished_executor_is_idle_and_stays_finished() {
    let bytes = build("procedure P { name \"P\" category normal notify \"once\" complete }");
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let aircraft = FakeAircraft::new();
    let mut recorder = Recorder::new();
    let mut exec = ProcedureExecutor::new(db.get_procedure("P").unwrap());

    assert_eq!(exec.tick(&aircraft, &mut recorder, 50), Tick::Completed);
    for _ in 0..5 {
        assert_eq!(exec.tick(&aircraft, &mut recorder, 50), Tick::Idle);
    }
    assert_eq!(exec.state(), ExecutionState::Completed);
    assert_eq!(recorder.notifications.len(), 1);
}

#[test]
fn reset_runs_the_procedure_again_from_the_top() {
    let bytes = build("procedure P { name \"P\" category normal notify \"go\" complete }");
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let aircraft = FakeAircraft::new();
    let mut recorder = Recorder::new();
    let mut exec = ProcedureExecutor::new(db.get_procedure("P").unwrap());

    exec.tick(&aircraft, &mut recorder, 50);
    exec.reset();
    assert_eq!(exec.state(), ExecutionState::Ready);
    exec.tick(&aircraft, &mut recorder, 50);
    assert_eq!(recorder.notifications.len(), 2);
}

#[test]
fn cancelling_a_waiting_procedure_reports_why() {
    let bytes = build(
        r#"
        procedure P {
            name "P"
            category normal
            wait hydraulic.2.pressure > 2500
            notify "unreachable"
            complete
        }
        "#,
    );
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let mut aircraft = FakeAircraft::new();
    aircraft.set_f32(tag::HYD2_PRESSURE, 100.0);
    let mut recorder = Recorder::new();
    let mut exec = ProcedureExecutor::new(db.get_procedure("P").unwrap());

    exec.tick(&aircraft, &mut recorder, 50);
    exec.cancel(&mut recorder);
    assert_eq!(exec.state(), ExecutionState::Cancelled);
    assert!(exec.is_finished());
    assert_eq!(exec.tick(&aircraft, &mut recorder, 50), Tick::Idle);
    assert!(recorder.failed.unwrap().contains("Cancelled"));
    assert!(recorder.notifications.is_empty());
}

#[test]
fn two_executors_over_one_database_do_not_interfere() {
    // The realistic case: an engineer's panel running a checklist while an
    // abnormal procedure monitors in the background.
    let bytes = support::compile_examples();
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let mut aircraft = FakeAircraft::new();
    aircraft.set_f32(tag::HYD2_PRESSURE, 1000.0);

    let mut a = ProcedureExecutor::new(db.get_procedure("HYD_ALL_SYSTEMS_CHECK").unwrap());
    let mut b = ProcedureExecutor::new(db.get_procedure("HYD_2_ELECTRIC_PUMP_START").unwrap());
    let mut ra = Recorder::new();
    let mut rb = Recorder::new();

    for tick in 0..40 {
        if tick == 4 {
            aircraft.set_bool(tag::HYD2_ELECTRIC_PUMP_RUNNING, true);
        }
        if tick == 10 {
            aircraft.set_f32(tag::HYD2_PRESSURE, 3000.0);
        }
        a.tick(&aircraft, &mut ra, 50);
        b.tick(&aircraft, &mut rb, 50);
    }

    assert_eq!(a.state(), ExecutionState::Completed);
    assert_eq!(b.state(), ExecutionState::Completed);
    // Each executor saw only its own actions.
    assert!(ra.actions.iter().all(|action| action.starts_with("check")));
    assert_eq!(rb.actions, vec!["set HYD_2_ELECTRIC_PUMP = ON".to_string()]);
}

#[test]
fn a_tick_is_bounded_even_with_a_tiny_budget() {
    // The step limit is a backstop for a hypothetical verifier bug. Setting it
    // absurdly low proves the executor honours it rather than looping.
    let bytes = build(
        r#"
        procedure P {
            name "P"
            category normal
            notify "a"
            notify "b"
            notify "c"
            complete
        }
        "#,
    );
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let aircraft = FakeAircraft::new();
    let mut recorder = Recorder::new();
    let mut exec = ProcedureExecutor::new(db.get_procedure("P").unwrap()).with_step_limit(2);
    assert_eq!(exec.tick(&aircraft, &mut recorder, 50), Tick::Failed);
    assert!(recorder.failed.unwrap().contains("StepLimitExceeded"));
}

#[test]
fn a_host_returning_the_wrong_type_fails_rather_than_guesses() {
    // A registry says `hydraulic.2.pressure` is a number. If the aircraft
    // hands back a bool, the procedure must stop, not coerce.
    let bytes = build(
        r#"
        procedure P {
            name "P"
            category normal
            wait hydraulic.2.pressure > 100
            complete
        }
        "#,
    );
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let mut aircraft = FakeAircraft::new();
    aircraft.set(tag::HYD2_PRESSURE, Value::Bool(true));
    let mut recorder = Recorder::new();
    let mut exec = ProcedureExecutor::new(db.get_procedure("P").unwrap());
    assert_eq!(exec.tick(&aircraft, &mut recorder, 50), Tick::Failed);
    assert!(recorder.failed.unwrap().contains("TypeMismatch"));
}

#[test]
fn the_emergency_procedure_recovers_or_escalates() {
    let bytes = support::compile_examples();
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();

    // The cabin responds to the outflow valves: rate arrests, altitude falls.
    let mut aircraft = FakeAircraft::new();
    aircraft.set_f32(tag::CABIN_ALTITUDE, 14_000.0);
    aircraft.set_f32(tag::CABIN_RATE, 3_000.0);
    let mut recorder = Recorder::new();
    let result = run(
        &db,
        "CABIN_RAPID_DEPRESSURIZATION",
        &mut aircraft,
        &mut recorder,
        400,
        |tick, aircraft| {
            if tick == 5 {
                aircraft.set_f32(tag::CABIN_RATE, 100.0);
            }
            if tick == 12 {
                aircraft.set_f32(tag::CABIN_ALTITUDE, 8_000.0);
            }
        },
    );
    assert_eq!(result, Tick::Completed);
    assert!(
        recorder
            .actions
            .iter()
            .any(|a| a == "set OUTFLOW_VALVE_FORWARD = CLOSED")
    );
    assert!(
        recorder
            .actions
            .iter()
            .any(|a| a == "set PACK_FLOW_SELECTOR = HIGH")
    );

    // The cabin does not respond: the procedure calls for support and fails
    // rather than reporting a recovery that did not happen.
    let mut aircraft = FakeAircraft::new();
    aircraft.set_f32(tag::CABIN_ALTITUDE, 14_000.0);
    aircraft.set_f32(tag::CABIN_RATE, 3_000.0);
    let mut recorder = Recorder::new();
    let result = run(
        &db,
        "CABIN_RAPID_DEPRESSURIZATION",
        &mut aircraft,
        &mut recorder,
        2000,
        |_, _| {},
    );
    assert_eq!(result, Tick::Failed);
    assert!(!recorder.completed);
    assert!(
        recorder
            .events
            .iter()
            .any(|e| e.contains("entered CABIN_EMERGENCY_DESCENT_SUPPORT"))
    );
}

#[test]
fn a_trigger_on_an_emergency_procedure_arms_correctly() {
    let bytes = support::compile_examples();
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let procedure = db.get_procedure("CABIN_RAPID_DEPRESSURIZATION").unwrap();
    let mut aircraft = FakeAircraft::new();

    assert_eq!(
        ProcedureExecutor::evaluate_trigger(&procedure, &aircraft).unwrap(),
        Some(false)
    );
    // A high cabin alone is not enough — a slow climb to a high field
    // elevation should not fire an emergency procedure.
    aircraft.set_f32(tag::CABIN_ALTITUDE, 12_000.0);
    assert_eq!(
        ProcedureExecutor::evaluate_trigger(&procedure, &aircraft).unwrap(),
        Some(false)
    );
    aircraft.set_f32(tag::CABIN_RATE, 4_000.0);
    assert_eq!(
        ProcedureExecutor::evaluate_trigger(&procedure, &aircraft).unwrap(),
        Some(true)
    );
}
