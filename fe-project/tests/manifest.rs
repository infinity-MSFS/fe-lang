use fe_project::{ControlSpec, SymbolRegistry, ValueType, parse};

const DC10: &str = include_str!("../../examples/dc10/fe.toml");

/// Comparable form of a registry. `SymbolRegistry` has no `PartialEq` and does
/// not need one; this is only for asserting two of them agree.
fn snapshot(
    registry: &SymbolRegistry,
) -> (
    Vec<(String, &'static str, u32)>,
    Vec<(String, &'static str, Vec<String>, u32)>,
) {
    let states = registry
        .states()
        .map(|s| (s.name.clone(), s.ty.as_str(), s.tag))
        .collect();
    let controls = registry
        .controls()
        .map(|c| {
            let positions = c.spec.positions().iter().map(|p| p.to_string()).collect();
            (c.name.clone(), c.spec.kind().as_str(), positions, c.tag)
        })
        .collect();
    (states, controls)
}

/// The registry the example procedures are written against, built in Rust.
/// A near-copy of `fe-compiler/tests/support/mod.rs::registry()` — deliberately
/// duplicated rather than shared, so that this test is comparing the manifest
/// against an independently written statement of the same thing.
fn rust_registry() -> SymbolRegistry {
    let mut r = SymbolRegistry::new();

    for (name, ty, tag) in [
        ("hydraulic.1.pressure", ValueType::F32, 10),
        ("hydraulic.2.pressure", ValueType::F32, 11),
        ("hydraulic.3.pressure", ValueType::F32, 12),
        ("hydraulic.2.electric_pump_running", ValueType::Bool, 13),
        ("engine.2.running", ValueType::Bool, 20),
        ("electrical.ac_bus.1.powered", ValueType::Bool, 30),
        ("electrical.ac_bus.2.powered", ValueType::Bool, 31),
        ("generator.2.available", ValueType::Bool, 32),
        ("fuel.imbalance_1_3", ValueType::F32, 40),
        ("fuel.tank.1.quantity", ValueType::F32, 41),
        ("fuel.tank.3.quantity", ValueType::F32, 43),
        ("fuel.crossfeed_open", ValueType::Bool, 44),
        ("cabin.altitude", ValueType::F32, 50),
        ("cabin.rate_of_climb", ValueType::F32, 51),
        ("aircraft.on_ground", ValueType::Bool, 52),
    ] {
        r.define_state(name, ty, tag).unwrap();
    }

    for (name, spec, tag) in [
        ("HYD_1_ENGINE_PUMP", ControlSpec::checklist(), 100),
        ("HYD_2_ENGINE_PUMP", ControlSpec::checklist(), 101),
        ("HYD_3_ENGINE_PUMP", ControlSpec::checklist(), 102),
        ("HYD_2_ELECTRIC_PUMP", ControlSpec::switch(), 103),
        ("HYD_2_ISOLATION_VALVE", ControlSpec::valve(), 104),
        ("GEN_2_FIELD", ControlSpec::switch(), 110),
        ("BUS_TIE_1_2", ControlSpec::valve(), 111),
        (
            "FUEL_XFEED_SELECTOR",
            ControlSpec::selector(["OFF", "TANK_1_TO_3", "TANK_3_TO_1"]),
            120,
        ),
        ("FUEL_CROSSFEED_VALVE", ControlSpec::valve(), 121),
        ("FUEL_QUANTITY_INDICATION", ControlSpec::checklist(), 122),
        (
            "FUEL_PUMP_PRESSURE_TARGET",
            ControlSpec::analog(0.0, 50.0),
            123,
        ),
        ("OUTFLOW_VALVE_FORWARD", ControlSpec::valve(), 130),
        ("OUTFLOW_VALVE_AFT", ControlSpec::valve(), 131),
        (
            "PACK_FLOW_SELECTOR",
            ControlSpec::selector(["OFF", "NORMAL", "HIGH"]),
            132,
        ),
        ("OXYGEN_PRESSURE_INDICATION", ControlSpec::checklist(), 133),
    ] {
        r.define_control(name, spec, tag).unwrap();
    }

    r
}

/// The test the whole crate exists for: what the manifest says and what the
/// aircraft's build says are the same registry, symbol for symbol, tag for tag.
/// If they ever diverge, the editor starts confidently answering questions with
/// the wrong aircraft's rules — which is the one failure mode worse than not
/// answering at all.
#[test]
fn the_example_manifest_matches_the_registry_built_in_rust() {
    let manifest = parse(DC10).expect("examples/dc10/fe.toml should parse");
    assert_eq!(snapshot(&manifest.registry), snapshot(&rust_registry()));
}

#[test]
fn analog_limits_survive_the_round_trip() {
    let manifest = parse(DC10).unwrap();
    let control = manifest
        .registry
        .control("FUEL_PUMP_PRESSURE_TARGET")
        .unwrap();
    assert_eq!(control.spec, ControlSpec::analog(0.0, 50.0));
}

#[test]
fn sources_default_to_the_manifest_directory() {
    let manifest = parse("[state]\n").unwrap();
    assert_eq!(manifest.sources, vec![fe_project::DEFAULT_SOURCE]);
}

// ------------------------------------------------------------------- errors
//
// Each of these should point at the offending text. A manifest error with no
// location is a scavenger hunt.

fn errors(text: &str) -> Vec<String> {
    parse(text)
        .unwrap_err()
        .into_iter()
        .map(|e| e.message)
        .collect()
}

/// The span has to select the text a reader would expect to see underlined.
fn spanned<'a>(text: &'a str, index: usize) -> &'a str {
    let error = &parse(text).unwrap_err()[index];
    let span = error
        .span
        .clone()
        .unwrap_or_else(|| panic!("{error} has no span"));
    &text[span]
}

#[test]
fn an_unknown_control_kind_is_reported_at_the_kind() {
    let text = "[controls]\nA = { kind = \"toggle\", tag = 1 }\n";
    assert!(errors(text)[0].contains("unknown control kind `toggle`"));
    assert_eq!(spanned(text, 0), "\"toggle\"");
}

#[test]
fn an_unknown_state_type_is_reported_at_the_type() {
    let text = "[state]\n\"a.b\" = { type = \"u32\", tag = 1 }\n";
    assert!(errors(text)[0].contains("unknown state type `u32`"));
    assert_eq!(spanned(text, 0), "\"u32\"");
}

/// A key that does not apply to the kind is refused rather than ignored: it
/// means the author believes something the compiler does not.
#[test]
fn positions_on_a_switch_are_refused() {
    let text = "[controls]\nA = { kind = \"switch\", tag = 1, positions = [\"UP\", \"DOWN\"] }\n";
    assert_eq!(
        errors(text),
        ["`positions` does not apply to a switch control"]
    );
    assert_eq!(spanned(text, 0), "[\"UP\", \"DOWN\"]");
}

#[test]
fn a_range_on_a_valve_is_refused() {
    let text = "[controls]\nA = { kind = \"valve\", tag = 1, min = 0.0, max = 1.0 }\n";
    assert_eq!(errors(text), ["`min` does not apply to a valve control"]);
}

#[test]
fn a_selector_without_positions_is_refused() {
    let text = "[controls]\nA = { kind = \"selector\", tag = 1 }\n";
    assert_eq!(errors(text), ["a selector needs `positions`"]);
}

#[test]
fn an_analog_without_limits_is_refused() {
    let text = "[controls]\nA = { kind = \"analog\", tag = 1, min = 0.0 }\n";
    assert_eq!(errors(text), ["an analog control needs `min` and `max`"]);
}

/// The registry's own validation reaches the manifest, spanned at the entry
/// that failed it, rather than being re-implemented here.
#[test]
fn registry_validation_is_reported_against_the_entry() {
    let text = "[controls]\nA = { kind = \"analog\", tag = 1, min = 50.0, max = 0.0 }\n";
    assert!(errors(text)[0].contains("analog minimum exceeds maximum"));

    let text = "[state]\n\"a..b\" = { type = \"f32\", tag = 1 }\n";
    assert!(errors(text)[0].contains("path segments may not be empty"));
    assert_eq!(spanned(text, 0), "\"a..b\"");

    let text = "[state]\n\"a\" = { type = \"f32\", tag = 1 }\n\n[controls]\na = { kind = \"switch\", tag = 2 }\n";
    assert!(errors(text)[0].contains("already registered"));
}

/// Fixing a manifest one error per round-trip is not a workflow.
#[test]
fn every_error_is_reported_at_once() {
    let text = "[controls]\n\
                A = { kind = \"toggle\", tag = 1 }\n\
                B = { kind = \"selector\", tag = 2 }\n\
                C = { kind = \"analog\", tag = 3 }\n";
    assert_eq!(errors(text).len(), 3);
}

#[test]
fn a_misspelled_table_is_not_silently_ignored() {
    let text = "[control]\nA = { kind = \"switch\", tag = 1 }\n";
    assert!(!errors(text).is_empty());
}

#[test]
fn malformed_toml_reports_where_it_gave_up() {
    let text = "[controls\nA = 1\n";
    let error = &parse(text).unwrap_err()[0];
    assert!(error.span.is_some(), "{error}");
}
