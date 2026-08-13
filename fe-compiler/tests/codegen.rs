//! Code generation tests, written against the disassembler.
//!
//! Snapshots of instruction text rather than of raw bytes: a byte-level golden
//! file would change every time a string id shifted, whereas this fails only
//! when the *shape* of the generated code changes — which is when a human
//! should be looking.

mod support;

use fe_runtime::{Instr, ProcedureDatabase};

/// Compile one procedure and disassemble its body to a trimmed line list.
fn body(source: &str) -> Vec<String> {
    let compiled = support::compile_source(source).unwrap_or_else(|e| panic!("{e}"));
    let bytes = compiled.into_bytes();
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let procedure = db.procedures().next().unwrap();
    let mut text = String::new();
    fe_runtime::disassemble_code(&db, procedure.body_code(), &mut text).unwrap();
    text.lines().map(|l| l.trim_end().to_string()).collect()
}

/// Just the opcode mnemonics, for tests about structure rather than operands.
fn opcodes(source: &str) -> Vec<String> {
    body(source)
        .iter()
        .filter_map(|line| line.split_whitespace().nth(1).map(String::from))
        .collect()
}

fn wrap(steps: &str) -> String {
    format!("procedure P {{ name \"P\" category normal {steps} }}")
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

#[test]
fn expressions_are_emitted_in_post_order() {
    let ops = opcodes(&wrap("wait hydraulic.2.pressure > 2500"));
    assert_eq!(
        ops,
        vec!["AWAIT", "LOAD_F32", "PUSH_F32", "GT", "AWAIT_TEST", "END"]
    );
}

#[test]
fn precedence_survives_into_the_bytecode() {
    // `a && b || c` must combine `a && b` first. In post-order that is
    // AND before OR.
    let ops = opcodes(&wrap(
        "wait engine.2.running && electrical.ac_bus.1.powered || generator.2.available",
    ));
    let and = ops.iter().position(|o| o == "AND").unwrap();
    let or = ops.iter().position(|o| o == "OR").unwrap();
    assert!(and < or, "{ops:?}");
}

#[test]
fn comparison_opcodes_are_type_specific() {
    assert!(opcodes(&wrap("wait hydraulic.2.pressure == 2500")).contains(&"EQ_F32".to_string()));
    assert!(
        opcodes(&wrap(
            "wait engine.2.running == electrical.ac_bus.1.powered"
        ))
        .contains(&"EQ_BOOL".to_string())
    );
}

#[test]
fn a_negated_literal_is_folded_into_the_constant() {
    let lines = body(&wrap("wait hydraulic.2.pressure > -50"));
    assert!(
        lines.iter().any(|l| l.contains("PUSH_F32       -50")),
        "{lines:?}"
    );
}

// ---------------------------------------------------------------------------
// Statements
// ---------------------------------------------------------------------------

#[test]
fn verbs_lower_to_position_indices() {
    let lines = body(&wrap(
        "start HYD_2_ELECTRIC_PUMP close HYD_2_ISOLATION_VALVE",
    ));
    assert!(
        lines[0].contains("SET_POSITION   #0 1 ; HYD_2_ELECTRIC_PUMP = ON"),
        "{lines:?}"
    );
    assert!(
        lines[1].contains("SET_POSITION   #1 0 ; HYD_2_ISOLATION_VALVE = CLOSED"),
        "{lines:?}"
    );
}

#[test]
fn an_if_without_an_else_jumps_straight_past_the_body() {
    let lines = body(&wrap("if engine.2.running { notify \"yes\" } complete"));
    // LOAD, JUMP_IF_FALSE past the NOTIFY, NOTIFY, COMPLETE, END.
    assert_eq!(
        opcodes(&wrap("if engine.2.running { notify \"yes\" } complete")),
        vec!["LOAD_BOOL", "JUMP_IF_FALSE", "NOTIFY", "COMPLETE", "END"]
    );
    // The conditional jump must land on the COMPLETE, not the NOTIFY.
    let target: u32 = lines[1].split('@').nth(1).unwrap().trim().parse().unwrap();
    let complete_at: u32 = lines[3].split_whitespace().next().unwrap().parse().unwrap();
    assert_eq!(target, complete_at);
}

#[test]
fn an_if_else_uses_one_conditional_and_one_unconditional_jump() {
    assert_eq!(
        opcodes(&wrap(
            "if engine.2.running { notify \"a\" } else { notify \"b\" } complete"
        )),
        vec![
            "LOAD_BOOL",
            "JUMP_IF_FALSE",
            "NOTIFY",
            "JUMP",
            "NOTIFY",
            "COMPLETE",
            "END"
        ]
    );
}

#[test]
fn a_terminating_then_branch_needs_no_jump_over_the_else() {
    // `then` ends the procedure, so the usual skip-the-else jump would be
    // dead code. Not emitting it keeps the verifier's reachability analysis
    // honest and saves five bytes in the most common abnormal-procedure shape.
    let ops = opcodes(&wrap(
        "if engine.2.running { fail \"no\" } else { notify \"b\" } complete",
    ));
    assert_eq!(
        ops,
        vec![
            "LOAD_BOOL",
            "JUMP_IF_FALSE",
            "FAIL",
            "NOTIFY",
            "COMPLETE",
            "END"
        ]
    );
}

#[test]
fn else_if_chains_flatten_into_nested_jumps() {
    let ops = opcodes(&wrap(
        "if engine.2.running { notify \"a\" } else if generator.2.available { notify \"b\" } else { notify \"c\" } complete",
    ));
    assert_eq!(ops.iter().filter(|o| *o == "JUMP_IF_FALSE").count(), 2);
    assert_eq!(ops.iter().filter(|o| *o == "JUMP").count(), 2);
}

#[test]
fn wait_is_await_condition_await_test() {
    let lines = body(&wrap("wait engine.2.running timeout 30s else fail"));
    assert!(
        lines[0].contains("AWAIT          body=3 timeout=30000ms on_timeout=fail"),
        "{lines:?}"
    );
    assert!(lines[1].contains("LOAD_BOOL"), "{lines:?}");
    assert!(lines[2].contains("AWAIT_TEST"), "{lines:?}");
}

#[test]
fn a_wait_without_a_timeout_encodes_zero() {
    let lines = body(&wrap("wait engine.2.running"));
    assert!(lines[0].contains("timeout=0ms"), "{lines:?}");
    assert!(lines[0].contains("on_timeout=continue"), "{lines:?}");
}

#[test]
fn complete_when_lowers_to_a_failing_wait_then_complete() {
    // The safety-relevant lowering: a completion criterion that times out
    // must not report completion.
    let ops = opcodes(&wrap(
        "complete when hydraulic.2.pressure > 2500 timeout 10s",
    ));
    assert_eq!(
        ops,
        vec![
            "AWAIT",
            "LOAD_F32",
            "PUSH_F32",
            "GT",
            "AWAIT_TEST",
            "COMPLETE",
            "END"
        ]
    );
    let lines = body(&wrap(
        "complete when hydraulic.2.pressure > 2500 timeout 10s",
    ));
    assert!(lines[0].contains("on_timeout=fail"), "{lines:?}");
}

#[test]
fn require_clauses_are_emitted_before_the_first_step() {
    let ops = opcodes(
        "procedure P { name \"P\" category normal require engine.2.running \"nope\" notify \"hi\" complete }",
    );
    assert_eq!(
        ops,
        vec!["LOAD_BOOL", "REQUIRE", "NOTIFY", "COMPLETE", "END"]
    );
}

#[test]
fn a_bare_fail_uses_the_no_string_sentinel() {
    let compiled = support::compile_source(&wrap("fail")).unwrap();
    let bytes = compiled.into_bytes();
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let procedure = db.procedures().next().unwrap();
    // Decoding through the public API rather than reading bytes directly.
    let mut text = String::new();
    fe_runtime::disassemble_code(&db, procedure.body_code(), &mut text).unwrap();
    assert!(text.contains("FAIL"), "{text}");
    assert!(
        text.contains("\"\""),
        "no message should render as empty: {text}"
    );
}

// ---------------------------------------------------------------------------
// Invariants that must hold for every procedure we generate
// ---------------------------------------------------------------------------

/// Walk a code slice, yielding `(offset, instruction)`.
fn instructions(code: &[u8]) -> Vec<(usize, Instr)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < code.len() {
        let (instr, len) = fe_runtime::decode(code, at).expect("verified code must decode");
        out.push((at, instr));
        at += len;
    }
    out
}

#[test]
fn every_jump_in_the_examples_points_forwards() {
    // The property the whole termination argument rests on, checked against
    // real generated code rather than a hand-written case.
    let bytes = support::compile_examples();
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let mut jumps = 0;
    for procedure in db.procedures() {
        let code = procedure.body_code();
        for (at, instr) in instructions(code) {
            if let Instr::Jump(target) | Instr::JumpIfFalse(target) = instr {
                jumps += 1;
                assert!(
                    target as usize > at,
                    "{} has a backward jump at {at} -> {target}",
                    procedure.id
                );
                assert!((target as usize) < code.len());
            }
        }
    }
    assert!(jumps > 0, "the examples contain no jumps to check");
}

#[test]
fn every_body_ends_with_end() {
    let bytes = support::compile_examples();
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    for procedure in db.procedures() {
        let code = procedure.body_code();
        assert_eq!(
            *code.last().unwrap(),
            fe_runtime::format::op::END,
            "{} does not end with END",
            procedure.id
        );
    }
}

#[test]
fn a_procedure_that_never_says_complete_still_terminates() {
    // Falling off the end is completion: an author who writes a checklist
    // without a closing `complete` gets the obvious behaviour, not a hang.
    let ops = opcodes(&wrap("check HYD_2_ENGINE_PUMP"));
    assert_eq!(ops, vec!["CHECK", "END"]);
}
