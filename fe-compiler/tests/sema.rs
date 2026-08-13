//! Semantic analysis tests.
//!
//! The point of most of these is not that compilation fails, but *how*: an
//! author who mistypes a symbol at 2am should get the right code, the right
//! span, and where possible the right suggestion.

mod support;

use support::{compile_source, expect_errors};

/// Wrap a body in a minimal valid procedure.
fn wrap(body: &str) -> String {
    format!("procedure P {{\n  name \"P\"\n  category normal\n{body}\n}}")
}

fn errors(body: &str) -> String {
    expect_errors(&wrap(body))
}

fn warnings(body: &str) -> Vec<String> {
    let compiled = compile_source(&wrap(body)).expect("expected this to compile");
    compiled
        .warnings()
        .iter()
        .map(|w| format!("{} {}", w.code, w.message))
        .collect()
}

fn assert_has(haystack: &str, needle: &str) {
    assert!(
        haystack.contains(needle),
        "expected {needle:?} in:\n{haystack}"
    );
}

// ---------------------------------------------------------------------------
// Symbol resolution
// ---------------------------------------------------------------------------

#[test]
fn unknown_symbols_are_rejected() {
    let rendered = errors("check NO_SUCH_CONTROL");
    assert_has(&rendered, "error[E0201]");
    assert_has(&rendered, "NO_SUCH_CONTROL");
}

#[test]
fn a_near_miss_gets_a_suggestion() {
    // The single most common authoring mistake, and the one where a compiler
    // earns its keep.
    let rendered = errors("check HYD_2_ENGINE_PUM");
    assert_has(&rendered, "did you mean");
    assert_has(&rendered, "HYD_2_ENGINE_PUMP");
}

#[test]
fn state_symbols_cannot_be_written() {
    // The aircraft owns its state. A procedure moves controls; the simulation
    // decides what that does to pressure.
    let rendered = errors("set hydraulic.2.pressure = 3000");
    assert_has(&rendered, "error[E0202]");
}

#[test]
fn controls_cannot_be_read() {
    let rendered = errors("wait HYD_2_ELECTRIC_PUMP");
    assert_has(&rendered, "error[E0203]");
}

#[test]
fn a_symbol_not_in_the_registry_cannot_be_invented() {
    let rendered = errors("wait hydraulic.9.pressure > 100");
    assert_has(&rendered, "error[E0201]");
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[test]
fn conditions_must_be_boolean() {
    let rendered = errors("wait hydraulic.2.pressure");
    assert_has(&rendered, "error[E0204]");
}

#[test]
fn numbers_and_booleans_do_not_compare() {
    let rendered = errors("wait engine.2.running > 1");
    assert_has(&rendered, "error[E0204]");
}

#[test]
fn logical_operators_need_booleans() {
    let rendered = errors("wait hydraulic.2.pressure && engine.2.running");
    assert_has(&rendered, "error[E0204]");
}

#[test]
fn booleans_compare_for_equality_only() {
    let rendered = errors("wait engine.2.running < electrical.ac_bus.2.powered");
    assert_has(&rendered, "error[E0204]");
    compile_source(&wrap(
        "wait engine.2.running == electrical.ac_bus.2.powered",
    ))
    .expect("boolean equality is allowed");
}

// ---------------------------------------------------------------------------
// Controls
// ---------------------------------------------------------------------------

#[test]
fn a_switch_does_not_accept_valve_positions() {
    let rendered = errors("open HYD_2_ELECTRIC_PUMP");
    assert_has(&rendered, "error[E0206]");
    assert_has(&rendered, "OPEN");
}

#[test]
fn a_selector_rejects_an_unlisted_position() {
    let rendered = errors("set FUEL_XFEED_SELECTOR = TANK_2_TO_4");
    assert_has(&rendered, "error[E0205]");
    // The error must say what *is* allowed, or the author has to go read the
    // registry source to find out.
    assert_has(&rendered, "TANK_1_TO_3");
}

#[test]
fn an_analog_control_rejects_a_named_position() {
    let rendered = errors("set FUEL_PUMP_PRESSURE_TARGET = ON");
    assert_has(&rendered, "error[E0205]");
}

#[test]
fn an_analog_value_must_be_in_range() {
    let rendered = errors("set FUEL_PUMP_PRESSURE_TARGET = 500");
    assert_has(&rendered, "error[E0207]");
    assert_has(&rendered, "50");
}

#[test]
fn a_checklist_item_cannot_be_actuated() {
    let rendered = errors("start HYD_1_ENGINE_PUMP");
    assert_has(&rendered, "error[E0206]");
}

#[test]
fn positions_are_matched_case_insensitively() {
    compile_source(&wrap("set HYD_2_ELECTRIC_PUMP = on")).expect("`on` should match `ON`");
}

// ---------------------------------------------------------------------------
// Metadata
// ---------------------------------------------------------------------------

#[test]
fn name_and_category_are_required() {
    let rendered = expect_errors("procedure P { complete }");
    assert_has(&rendered, "error[E0211]");
    assert_has(&rendered, "name");
    assert_has(&rendered, "category");
}

#[test]
fn the_category_must_be_one_of_the_four() {
    let rendered = expect_errors("procedure P { name \"P\" category urgent complete }");
    assert_has(&rendered, "error[E0212]");
    assert_has(&rendered, "emergency");
}

#[test]
fn priority_must_fit_in_its_field() {
    let rendered =
        expect_errors("procedure P { name \"P\" category normal priority 900 complete }");
    assert_has(&rendered, "error[E0212]");
}

#[test]
fn a_zero_timeout_is_an_error() {
    let rendered = errors("wait engine.2.running timeout 0s");
    assert_has(&rendered, "error[E0213]");
}

#[test]
fn timeout_without_when_is_an_error() {
    let rendered = errors("complete timeout 10s");
    assert_has(&rendered, "error[E0213]");
}

// ---------------------------------------------------------------------------
// Procedures and the call graph
// ---------------------------------------------------------------------------

#[test]
fn duplicate_identifiers_across_units_collide() {
    use fe_compiler::SourceUnit;
    let units = vec![
        SourceUnit::new(
            "a.fe",
            "procedure P { name \"a\" category normal complete }",
        ),
        SourceUnit::new(
            "b.fe",
            "procedure P { name \"b\" category normal complete }",
        ),
    ];
    let error = fe_compiler::compile(&units, &support::registry()).unwrap_err();
    let rendered = error.render(&units);
    assert_has(&rendered, "error[E0209]");
    // Both sites, so the author knows which two files to look at.
    assert_has(&rendered, "a.fe");
    assert_has(&rendered, "b.fe");
}

#[test]
fn calling_an_undefined_procedure_is_an_error() {
    let rendered = errors("call NOPE");
    assert_has(&rendered, "error[E0208]");
}

#[test]
fn direct_recursion_is_rejected() {
    let rendered = expect_errors("procedure P { name \"P\" category normal call P }");
    assert_has(&rendered, "error[E0210]");
}

#[test]
fn mutual_recursion_is_rejected() {
    let rendered = expect_errors(
        r#"
        procedure A { name "A" category normal call B }
        procedure B { name "B" category normal call C }
        procedure C { name "C" category normal call A }
        "#,
    );
    assert_has(&rendered, "error[E0210]");
}

#[test]
fn a_call_chain_deeper_than_the_runtime_stack_is_rejected() {
    // The executor's frame array is fixed, so depth is a compile-time property
    // rather than a runtime surprise.
    let mut source = String::new();
    let depth = fe_runtime::MAX_CALL_DEPTH + 2;
    for i in 0..depth {
        source.push_str(&format!("procedure P{i} {{ name \"P{i}\" category normal "));
        if i + 1 < depth {
            source.push_str(&format!("call P{} ", i + 1));
        } else {
            source.push_str("complete ");
        }
        source.push_str("}\n");
    }
    let rendered = expect_errors(&source);
    assert_has(&rendered, "error[E0215]");
}

#[test]
fn a_chain_within_the_limit_compiles() {
    let mut source = String::new();
    let depth = fe_runtime::MAX_CALL_DEPTH;
    for i in 0..depth {
        source.push_str(&format!("procedure P{i} {{ name \"P{i}\" category normal "));
        if i + 1 < depth {
            source.push_str(&format!("call P{} ", i + 1));
        } else {
            source.push_str("complete ");
        }
        source.push_str("}\n");
    }
    compile_source(&source).expect("a chain at the limit must compile");
}

#[test]
fn an_empty_procedure_is_an_error() {
    let rendered = expect_errors("procedure P { name \"P\" category normal }");
    assert_has(&rendered, "error[E0108]");
}

// ---------------------------------------------------------------------------
// Warnings
// ---------------------------------------------------------------------------

#[test]
fn steps_after_a_terminator_are_flagged() {
    let warnings = warnings("complete\ncheck HYD_2_ENGINE_PUMP");
    assert!(
        warnings.iter().any(|w| w.starts_with("W0001")),
        "{warnings:?}"
    );
}

#[test]
fn comparing_floats_for_equality_is_flagged() {
    let warnings = warnings("wait hydraulic.2.pressure == 3000");
    assert!(
        warnings.iter().any(|w| w.starts_with("W0002")),
        "{warnings:?}"
    );
}

#[test]
fn a_condition_that_reads_nothing_is_flagged() {
    let warnings = warnings("wait true");
    assert!(
        warnings.iter().any(|w| w.starts_with("W0005")),
        "{warnings:?}"
    );
}

#[test]
fn warnings_do_not_prevent_compilation() {
    let compiled = compile_source(&wrap("wait hydraulic.2.pressure == 3000")).unwrap();
    assert!(!compiled.warnings().is_empty());
    assert!(!compiled.as_bytes().is_empty());
}

// ---------------------------------------------------------------------------
// check-only entry point
// ---------------------------------------------------------------------------

#[test]
fn check_reports_the_same_diagnostics_without_emitting() {
    use fe_compiler::SourceUnit;
    let source = wrap("check NO_SUCH_CONTROL");
    let units = vec![SourceUnit::new("test.fe", source)];
    let diagnostics = fe_compiler::check(&units, &support::registry());
    assert!(diagnostics.has_errors());
    assert!(diagnostics.errors().any(|d| d.code == "E0201"));
}

#[test]
fn every_error_reports_a_span_inside_its_source() {
    use fe_compiler::SourceUnit;
    let source = wrap("check NO_SUCH_CONTROL\nset FUEL_PUMP_PRESSURE_TARGET = 900");
    let units = vec![SourceUnit::new("test.fe", source.clone())];
    let diagnostics = fe_compiler::check(&units, &support::registry());
    assert!(diagnostics.has_errors());
    for diagnostic in diagnostics.iter() {
        let span = diagnostic.primary.span;
        assert!(
            span.end as usize <= source.len(),
            "{} has a span past the end of the source",
            diagnostic.code
        );
    }
}
