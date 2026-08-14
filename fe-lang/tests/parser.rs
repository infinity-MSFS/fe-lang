use fe_lang::ast::*;
use fe_lang::diagnostics::Diagnostics;
use fe_lang::span::{SourceMap, SourceUnit, UnitId};
use fe_lang::{Ast, parse_unit};

fn parse(source: &str) -> (Ast, Diagnostics) {
    let mut diagnostics = Diagnostics::new();
    let ast = parse_unit(UnitId(0), source, &mut diagnostics);
    (ast, diagnostics)
}

fn parse_ok(source: &str) -> Ast {
    let (ast, diagnostics) = parse(source);
    if diagnostics.has_errors() {
        let units = [SourceUnit::new("test.fe", source)];
        panic!("{}", diagnostics.render(&SourceMap::new(&units)));
    }
    ast
}

fn codes(diagnostics: &Diagnostics) -> Vec<&str> {
    diagnostics.iter().map(|d| d.code).collect()
}

fn wrap(body: &str) -> String {
    format!("procedure P {{\n name \"P\"\n category normal\n{body}\n}}")
}

#[test]
fn parses_metadata_and_steps() {
    let ast = parse_ok(
        r#"
        procedure HYD_2_LOW {
            name        "Hydraulic 2 Low"
            description "Words."
            category    abnormal
            priority    80
            revision    3
            trigger     hydraulic.2.pressure < 1800
            require     engine.2.running "engine 2 must be running"

            check HYD_2_ENGINE_PUMP
            complete
        }
        "#,
    );
    assert_eq!(ast.procedures.len(), 1);
    let p = &ast.procedures[0];
    assert_eq!(p.id.text, "HYD_2_LOW");
    assert_eq!(p.metadata.name.as_ref().unwrap().value, "Hydraulic 2 Low");
    assert_eq!(p.metadata.category.as_ref().unwrap().text, "abnormal");
    assert_eq!(p.metadata.priority.as_ref().unwrap().value, 80.0);
    assert_eq!(p.metadata.requires.len(), 1);
    assert!(p.metadata.trigger.is_some());
    assert_eq!(p.body.steps.len(), 2);
}

#[test]
fn parses_several_procedures_from_one_unit() {
    let ast = parse_ok(
        r#"
        procedure A { name "A" category normal complete }
        procedure B { name "B" category normal complete }
        "#,
    );
    assert_eq!(ast.procedures.len(), 2);
}

#[test]
fn dotted_paths_keep_their_segments() {
    let ast = parse_ok(&wrap("check hydraulic.2.electric_pump"));
    let Step::Check { control, .. } = &ast.procedures[0].body.steps[0] else {
        panic!("expected a check step");
    };
    assert_eq!(control.text, "hydraulic.2.electric_pump");
    assert_eq!(control.segments.len(), 3);
    assert_eq!(control.segments[1].text, "2");
}

#[test]
fn keywords_are_contextual() {
    let ast = parse_ok(&wrap("check hydraulic.name.check.set"));
    let Step::Check { control, .. } = &ast.procedures[0].body.steps[0] else {
        panic!("expected a check step");
    };
    assert_eq!(control.text, "hydraulic.name.check.set");
}

#[test]
fn verbs_are_sugar_for_positions() {
    let ast = parse_ok(&wrap("start P1\nstop P2\nopen V1\nclose V2"));
    let verbs: Vec<Verb> = ast.procedures[0]
        .body
        .steps
        .iter()
        .map(|s| match s {
            Step::Verb { verb, .. } => *verb,
            other => panic!("expected a verb step, got {other:?}"),
        })
        .collect();
    assert_eq!(
        verbs,
        vec![Verb::Start, Verb::Stop, Verb::Open, Verb::Close]
    );
    assert_eq!(Verb::Start.position(), "ON");
    assert_eq!(Verb::Stop.position(), "OFF");
    assert_eq!(Verb::Open.position(), "OPEN");
    assert_eq!(Verb::Close.position(), "CLOSED");
}

#[test]
fn set_accepts_positions_and_numbers() {
    let ast = parse_ok(&wrap(
        "set SEL = TANK_1_TO_3\nset PRESS = 22.5\nset TRIM = -1.5",
    ));
    let steps = &ast.procedures[0].body.steps;
    assert!(matches!(
        &steps[0],
        Step::Set {
            value: SetValue::Position(ident),
            ..
        } if ident.text == "TANK_1_TO_3"
    ));
    assert!(matches!(
        &steps[1],
        Step::Set { value: SetValue::Number(n), .. } if n.value == 22.5
    ));
    assert!(matches!(
        &steps[2],
        Step::Set { value: SetValue::Number(n), .. } if n.value == -1.5
    ));
}

#[test]
fn wait_timeout_variants() {
    let ast = parse_ok(&wrap(
        "wait a\nwait b timeout 30s\nwait c timeout 500ms else fail",
    ));
    let steps = &ast.procedures[0].body.steps;
    let timeout = |i: usize| match &steps[i] {
        Step::Wait { timeout, .. } => timeout.clone(),
        other => panic!("expected wait, got {other:?}"),
    };
    assert!(timeout(0).is_none());
    let t1 = timeout(1).unwrap();
    assert_eq!(t1.millis, 30_000);
    assert!(!t1.fail);
    let t2 = timeout(2).unwrap();
    assert_eq!(t2.millis, 500);
    assert!(t2.fail);
}

#[test]
fn complete_when_is_optional() {
    let ast = parse_ok(&wrap("complete"));
    assert!(matches!(
        &ast.procedures[0].body.steps[0],
        Step::Complete {
            condition: None,
            ..
        }
    ));

    let ast = parse_ok(&wrap("complete when a > 1 timeout 10s"));
    let Step::Complete {
        condition, timeout, ..
    } = &ast.procedures[0].body.steps[0]
    else {
        panic!("expected complete");
    };
    assert!(condition.is_some());
    assert_eq!(timeout.as_ref().unwrap().millis, 10_000);
}

#[test]
fn else_if_chains_nest() {
    let ast = parse_ok(&wrap(
        "if a { check X } else if b { check Y } else { check Z }",
    ));
    let Step::If(outer) = &ast.procedures[0].body.steps[0] else {
        panic!("expected if");
    };
    let Some(ElseBranch::If(inner)) = &outer.else_branch else {
        panic!("expected `else if`");
    };
    assert!(matches!(inner.else_branch, Some(ElseBranch::Block(_))));
}

fn shape(expr: &Expr) -> String {
    match expr {
        Expr::Bool(v, _) => v.to_string(),
        Expr::Number(v, _) => format!("{v}"),
        Expr::Symbol(path) => path.text.clone(),
        Expr::Not { operand, .. } => format!("(! {})", shape(operand)),
        Expr::Binary { op, lhs, rhs, .. } => {
            format!("({} {} {})", op.as_str(), shape(lhs), shape(rhs))
        }
        Expr::Error(_) => "<error>".to_string(),
    }
}

fn condition_of(source: &str) -> String {
    let ast = parse_ok(&wrap(&format!("wait {source}")));
    match &ast.procedures[0].body.steps[0] {
        Step::Wait { condition, .. } => shape(condition),
        other => panic!("expected wait, got {other:?}"),
    }
}

#[test]
fn precedence_is_or_then_and_then_comparison() {
    assert_eq!(condition_of("a && b || c"), "(|| (&& a b) c)");
    assert_eq!(condition_of("a || b && c"), "(|| a (&& b c))");
    assert_eq!(condition_of("x > 1 && y < 2"), "(&& (> x 1) (< y 2))");
    assert_eq!(condition_of("!a && b"), "(&& (! a) b)");
    assert_eq!(condition_of("!(a && b)"), "(! (&& a b))");
    assert_eq!(condition_of("(a || b) && c"), "(&& (|| a b) c)");
}

#[test]
fn comparisons_do_not_chain() {
    let (_, diagnostics) = parse(&wrap("wait a < b < c"));
    assert!(codes(&diagnostics).contains(&"E0107"));
}

#[test]
fn metadata_after_a_step_is_an_error() {
    let (_, diagnostics) = parse(&wrap("check X\npriority 10"));
    assert!(codes(&diagnostics).contains(&"E0105"));
}

#[test]
fn duplicate_metadata_is_reported_once() {
    let (_, diagnostics) = parse("procedure P { name \"a\" name \"b\" category normal complete }");
    assert_eq!(
        codes(&diagnostics)
            .iter()
            .filter(|c| **c == "E0106")
            .count(),
        1
    );
}

#[test]
fn recovery_keeps_parsing_later_procedures() {
    let (ast, diagnostics) = parse(
        r#"
        procedure BROKEN {
            name "Broken"
            category normal
            set = ON
        }
        procedure FINE {
            name "Fine"
            category normal
            complete
        }
        "#,
    );
    assert!(diagnostics.has_errors());
    let ids: Vec<&str> = ast.procedures.iter().map(|p| p.id.text.as_str()).collect();
    assert!(ids.contains(&"FINE"), "recovery lost the second procedure");
}

#[test]
fn recovery_inside_a_body_keeps_later_steps() {
    let (ast, diagnostics) = parse(&wrap("set = ON\ncheck GOOD\ncomplete"));
    assert!(diagnostics.has_errors());
    let steps = &ast.procedures[0].body.steps;
    assert!(
        steps.iter().any(|s| matches!(s, Step::Check { .. })),
        "recovery lost the following step"
    );
}

#[test]
fn missing_brace_does_not_hang_or_panic() {
    let (_, diagnostics) = parse("procedure P { name \"P\" category normal check X");
    assert!(diagnostics.has_errors());
}

#[test]
fn stray_tokens_at_top_level_are_reported() {
    let (_, diagnostics) = parse("hello procedure P { name \"P\" category normal complete }");
    assert!(codes(&diagnostics).contains(&"E0102"));
}

#[test]
fn diagnostics_render_with_a_caret_and_the_source_line() {
    let source = wrap("wait a < b < c");
    let (_, diagnostics) = parse(&source);
    let units = [SourceUnit::new("hydraulic.fe", source.clone())];
    let rendered = diagnostics.render(&SourceMap::new(&units));
    assert!(rendered.contains("error[E0107]"), "{rendered}");
    assert!(rendered.contains("hydraulic.fe:"), "{rendered}");
    assert!(rendered.contains('^'), "{rendered}");
}

#[test]
fn an_empty_source_is_an_empty_ast() {
    let ast = parse_ok("   \n// just a comment\n");
    assert!(ast.procedures.is_empty());
}

#[test]
fn a_procedure_span_reaches_its_closing_brace() {
    let source = "procedure P {\n    name \"P\"\n    category normal\n    complete\n}\n";
    let ast = parse_ok(source);
    let span = ast.procedures[0].span;
    assert_eq!(
        &source[span.start as usize..span.end as usize],
        source.trim_end()
    );
}

#[test]
fn a_step_span_reaches_its_last_token() {
    let source = wrap("wait hydraulic.2.pressure > 2500 timeout 30s");
    let ast = parse_ok(&source);
    let span = ast.procedures[0].body.steps[0].span();
    assert_eq!(
        &source[span.start as usize..span.end as usize],
        "wait hydraulic.2.pressure > 2500 timeout 30s"
    );
}

#[test]
fn an_if_span_covers_both_branches() {
    let source = wrap("if a {\n        complete\n    } else {\n        fail\n    }");
    let ast = parse_ok(&source);
    let span = ast.procedures[0].body.steps[0].span();
    let text = &source[span.start as usize..span.end as usize];
    assert!(text.starts_with("if a {"), "{text:?}");
    assert!(text.ends_with('}'), "{text:?}");
    assert!(text.contains("else"), "{text:?}");
}
