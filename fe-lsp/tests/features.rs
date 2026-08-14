//! Completion, hover, navigation, quick fixes, formatting.

mod harness;

use harness::{DC10_MANIFEST, Harness, at};
use lsp_types::*;

const HYDRAULIC: &str = include_str!("../../examples/dc10/hydraulic.fe");

fn labels(response: Option<CompletionResponse>) -> Vec<String> {
    match response {
        Some(CompletionResponse::Array(items)) => items.into_iter().map(|i| i.label).collect(),
        Some(CompletionResponse::List(list)) => list.items.into_iter().map(|i| i.label).collect(),
        None => Vec::new(),
    }
}

fn complete(server: &Harness, name: &str, source: &str, cursor: &str) -> Vec<String> {
    let position = at(source, cursor);
    let position = Position {
        line: position.line,
        character: position.character + cursor.len() as u32,
    };
    labels(server.request::<request::Completion>(CompletionParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri(name),
            },
            position,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
        context: None,
    }))
}

/// The point of the whole exercise: the list is the control's own positions,
/// not a guess at what aircraft usually call things.
#[test]
fn a_position_list_is_the_controls_own() {
    let source =
        "procedure P {\n    name \"P\"\n    category normal\n    set FUEL_XFEED_SELECTOR = \n}\n";
    let server = Harness::new(&[("a.fe", source)], Some(DC10_MANIFEST));
    server.open("a.fe", source);

    assert_eq!(
        complete(&server, "a.fe", source, "set FUEL_XFEED_SELECTOR = "),
        ["OFF", "TANK_1_TO_3", "TANK_3_TO_1"]
    );
}

/// `open` is valid on a valve and not on a switch, so a switch must not be
/// offered — offering one would be offering E0206.
#[test]
fn a_verb_only_offers_controls_that_accept_it() {
    let source = "procedure P {\n    name \"P\"\n    category normal\n    open \n}\n";
    let server = Harness::new(&[("a.fe", source)], Some(DC10_MANIFEST));
    server.open("a.fe", source);

    let offered = complete(&server, "a.fe", source, "    open ");
    assert!(
        offered.contains(&"HYD_2_ISOLATION_VALVE".to_string()),
        "{offered:?}"
    );
    assert!(offered.contains(&"BUS_TIE_1_2".to_string()), "{offered:?}");
    assert!(
        !offered.contains(&"HYD_2_ELECTRIC_PUMP".to_string()),
        "a switch should not be offered after `open`: {offered:?}"
    );
    assert!(
        !offered.contains(&"HYD_1_ENGINE_PUMP".to_string()),
        "a checklist item has no actuator: {offered:?}"
    );
}

/// `check` is the one verb every kind accepts.
#[test]
fn check_offers_every_control() {
    let source = "procedure P {\n    name \"P\"\n    category normal\n    check \n}\n";
    let server = Harness::new(&[("a.fe", source)], Some(DC10_MANIFEST));
    server.open("a.fe", source);

    let offered = complete(&server, "a.fe", source, "    check ");
    for control in [
        "HYD_1_ENGINE_PUMP",
        "HYD_2_ELECTRIC_PUMP",
        "HYD_2_ISOLATION_VALVE",
        "FUEL_XFEED_SELECTOR",
        "FUEL_PUMP_PRESSURE_TARGET",
    ] {
        assert!(
            offered.contains(&control.to_string()),
            "{control} missing from {offered:?}"
        );
    }
}

/// A condition reads state. A control there is E0203, so no control is offered.
#[test]
fn a_condition_offers_state_and_not_controls() {
    let source = "procedure P {\n    name \"P\"\n    category normal\n    wait \n}\n";
    let server = Harness::new(&[("a.fe", source)], Some(DC10_MANIFEST));
    server.open("a.fe", source);

    let offered = complete(&server, "a.fe", source, "    wait ");
    assert!(
        offered.contains(&"hydraulic.2.pressure".to_string()),
        "{offered:?}"
    );
    assert!(offered.contains(&"true".to_string()));
    assert!(
        !offered.iter().any(|label| label.starts_with("HYD_")),
        "a control is not readable: {offered:?}"
    );
}

/// Procedures come from anywhere in the project, not just the open file.
#[test]
fn call_offers_procedures_from_every_file() {
    let a = "procedure P {\n    name \"P\"\n    category normal\n    call \n}\n";
    let b = "procedure ELSEWHERE {\n    name \"Elsewhere\"\n    category normal\n    complete\n}\n";
    let server = Harness::new(&[("a.fe", a), ("b.fe", b)], Some(DC10_MANIFEST));
    server.open("a.fe", a);

    let offered = complete(&server, "a.fe", a, "    call ");
    assert!(offered.contains(&"ELSEWHERE".to_string()), "{offered:?}");
}

/// Without a manifest the server has no registry, so it falls back to names the
/// files already use — a way to retype a name, not a claim that it is real.
#[test]
fn without_a_manifest_completion_is_what_the_files_say() {
    let source = "procedure P {\n    name \"P\"\n    category normal\n    check MADE_UP_CONTROL\n    check \n}\n";
    let server = Harness::new(&[("a.fe", source)], None);
    server.open("a.fe", source);

    let offered = complete(&server, "a.fe", source, "MADE_UP_CONTROL\n    check ");
    assert_eq!(offered, ["MADE_UP_CONTROL"]);
}

#[test]
fn hover_reports_the_type_of_a_state_path() {
    let server = Harness::new(&[("hydraulic.fe", HYDRAULIC)], Some(DC10_MANIFEST));
    server.open("hydraulic.fe", HYDRAULIC);

    let hover = server
        .request::<request::HoverRequest>(HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: server.uri("hydraulic.fe"),
                },
                position: at(HYDRAULIC, "hydraulic.2.pressure"),
            },
            work_done_progress_params: Default::default(),
        })
        .expect("a registered state path should hover");

    let HoverContents::Markup(markup) = hover.contents else {
        panic!("expected markdown");
    };
    assert!(markup.value.contains("state"), "{}", markup.value);
    assert!(markup.value.contains("number"), "{}", markup.value);
    assert!(markup.value.contains("read-only"), "{}", markup.value);
}

#[test]
fn hover_reports_a_controls_kind_and_positions() {
    let source =
        "procedure P {\n    name \"P\"\n    category normal\n    check FUEL_XFEED_SELECTOR\n}\n";
    let server = Harness::new(&[("a.fe", source)], Some(DC10_MANIFEST));
    server.open("a.fe", source);

    let hover = server
        .request::<request::HoverRequest>(HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: server.uri("a.fe"),
                },
                position: at(source, "FUEL_XFEED_SELECTOR"),
            },
            work_done_progress_params: Default::default(),
        })
        .expect("a registered control should hover");

    let HoverContents::Markup(markup) = hover.contents else {
        panic!("expected markdown");
    };
    assert!(markup.value.contains("selector"), "{}", markup.value);
    assert!(markup.value.contains("TANK_1_TO_3"), "{}", markup.value);
    assert!(markup.value.contains("check"), "{}", markup.value);
}

/// One flat namespace across every file, so `call` jumps wherever the
/// declaration happens to be.
#[test]
fn call_jumps_to_a_procedure_in_another_file() {
    let a = "procedure P {\n    name \"P\"\n    category normal\n    call ELSEWHERE\n}\n";
    let b = "procedure ELSEWHERE {\n    name \"Elsewhere\"\n    category normal\n    complete\n}\n";
    let server = Harness::new(&[("a.fe", a), ("b.fe", b)], Some(DC10_MANIFEST));
    server.open("a.fe", a);

    let response = server
        .request::<request::GotoDefinition>(GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: server.uri("a.fe"),
                },
                position: at(a, "ELSEWHERE"),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .expect("a call should resolve");

    let GotoDefinitionResponse::Scalar(location) = response else {
        panic!("expected one location");
    };
    assert_eq!(location.uri, server.uri("b.fe"));
    assert_eq!(location.range.start, at(b, "ELSEWHERE"));
}

/// "What else moves this valve?" — the question a checklist author actually
/// asks, answered across the project.
#[test]
fn references_find_every_place_a_control_is_touched() {
    let a =
        "procedure P {\n    name \"P\"\n    category normal\n    open HYD_2_ISOLATION_VALVE\n}\n";
    let b = "procedure Q {\n    name \"Q\"\n    category normal\n    close HYD_2_ISOLATION_VALVE\n    check HYD_2_ISOLATION_VALVE\n}\n";
    let server = Harness::new(&[("a.fe", a), ("b.fe", b)], Some(DC10_MANIFEST));
    server.open("a.fe", a);

    let found = server
        .request::<request::References>(ReferenceParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: server.uri("a.fe"),
                },
                position: at(a, "HYD_2_ISOLATION_VALVE"),
            },
            context: ReferenceContext {
                include_declaration: true,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .expect("references should resolve");

    assert_eq!(found.len(), 3, "{found:#?}");
}

/// The near miss, made one keystroke. It is the most common authoring mistake
/// and the compiler already computed the answer.
#[test]
fn a_misspelled_control_offers_the_right_name() {
    let source =
        "procedure P {\n    name \"P\"\n    category normal\n    check HYD_2_ENGINE_PUM\n}\n";
    let server = Harness::new(&[("a.fe", source)], Some(DC10_MANIFEST));
    server.open("a.fe", source);

    let diagnostics = server.diagnostics("a.fe");
    assert_eq!(diagnostics.len(), 1);

    let actions = server
        .request::<request::CodeActionRequest>(CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("a.fe"),
            },
            range: diagnostics[0].range,
            context: CodeActionContext {
                diagnostics: diagnostics.clone(),
                ..Default::default()
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .expect("a quick fix should be offered");

    let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
        panic!("expected a code action");
    };
    assert_eq!(action.title, "Change to `HYD_2_ENGINE_PUMP`");
    assert_eq!(action.is_preferred, Some(true));

    let edits = action.edit.as_ref().unwrap().changes.as_ref().unwrap();
    let edit = &edits[&server.uri("a.fe")][0];
    assert_eq!(edit.new_text, "HYD_2_ENGINE_PUMP");
    assert_eq!(edit.range, diagnostics[0].range);
}

/// `open` on a switch has one sensible repair, and it is not "use a valve".
#[test]
fn an_invalid_verb_offers_the_one_that_means_the_same() {
    let source =
        "procedure P {\n    name \"P\"\n    category normal\n    open HYD_2_ELECTRIC_PUMP\n}\n";
    let server = Harness::new(&[("a.fe", source)], Some(DC10_MANIFEST));
    server.open("a.fe", source);

    let diagnostics = server.diagnostics("a.fe");
    let actions = server
        .request::<request::CodeActionRequest>(CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("a.fe"),
            },
            range: diagnostics[0].range,
            context: CodeActionContext {
                diagnostics: diagnostics.clone(),
                ..Default::default()
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .expect("a quick fix should be offered");

    let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
        panic!("expected a code action");
    };
    assert_eq!(action.title, "Use `start` instead");
    let edits = action.edit.as_ref().unwrap().changes.as_ref().unwrap();
    assert_eq!(edits[&server.uri("a.fe")][0].new_text, "start");
}

/// An unlisted position offers the ones the control actually has.
#[test]
fn an_invalid_position_offers_the_registered_ones() {
    let source = "procedure P {\n    name \"P\"\n    category normal\n    set FUEL_XFEED_SELECTOR = TANK_9\n}\n";
    let server = Harness::new(&[("a.fe", source)], Some(DC10_MANIFEST));
    server.open("a.fe", source);

    let diagnostics = server.diagnostics("a.fe");
    assert_eq!(
        diagnostics[0].code,
        Some(NumberOrString::String("E0205".to_string())),
        "{diagnostics:#?}"
    );

    let actions = server
        .request::<request::CodeActionRequest>(CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("a.fe"),
            },
            range: diagnostics[0].range,
            context: CodeActionContext {
                diagnostics: diagnostics.clone(),
                ..Default::default()
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .expect("quick fixes should be offered");

    let titles: Vec<String> = actions
        .iter()
        .map(|action| match action {
            CodeActionOrCommand::CodeAction(action) => action.title.clone(),
            _ => panic!("expected code actions"),
        })
        .collect();
    assert_eq!(
        titles,
        [
            "Change to `OFF`",
            "Change to `TANK_1_TO_3`",
            "Change to `TANK_3_TO_1`"
        ]
    );
}

#[test]
fn renaming_a_procedure_updates_every_call() {
    let a = "procedure P {\n    name \"P\"\n    category normal\n    call ELSEWHERE\n}\n";
    let b = "procedure ELSEWHERE {\n    name \"E\"\n    category normal\n    complete\n}\n";
    let server = Harness::new(&[("a.fe", a), ("b.fe", b)], Some(DC10_MANIFEST));
    server.open("a.fe", a);

    let edit = server
        .request::<request::Rename>(RenameParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: server.uri("a.fe"),
                },
                position: at(a, "ELSEWHERE"),
            },
            new_name: "RESTORE_PRESSURE".to_string(),
            work_done_progress_params: Default::default(),
        })
        .expect("a procedure should be renameable");

    let changes = edit.changes.unwrap();
    assert_eq!(changes.len(), 2, "both the call and the declaration");
    assert_eq!(changes[&server.uri("a.fe")][0].new_text, "RESTORE_PRESSURE");
    assert_eq!(changes[&server.uri("b.fe")][0].new_text, "RESTORE_PRESSURE");
}

/// A control's name belongs to the aircraft's manifest and its systems code.
/// Renaming it here would rename one half of a relationship.
#[test]
fn a_control_cannot_be_renamed() {
    let source =
        "procedure P {\n    name \"P\"\n    category normal\n    check HYD_2_ENGINE_PUMP\n}\n";
    let server = Harness::new(&[("a.fe", source)], Some(DC10_MANIFEST));
    server.open("a.fe", source);

    let response = server.request::<request::PrepareRenameRequest>(TextDocumentPositionParams {
        text_document: TextDocumentIdentifier {
            uri: server.uri("a.fe"),
        },
        position: at(source, "HYD_2_ENGINE_PUMP"),
    });
    assert!(response.is_none());
}

#[test]
fn the_outline_lists_procedures_with_their_titles() {
    let server = Harness::new(&[("hydraulic.fe", HYDRAULIC)], Some(DC10_MANIFEST));
    server.open("hydraulic.fe", HYDRAULIC);

    let response = server
        .request::<request::DocumentSymbolRequest>(DocumentSymbolParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("hydraulic.fe"),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .expect("the outline should resolve");

    let DocumentSymbolResponse::Nested(symbols) = response else {
        panic!("expected nested symbols");
    };
    assert!(!symbols.is_empty());
    assert!(symbols.iter().all(|s| s.detail.is_some()), "{symbols:#?}");
    // The range covers the whole procedure, not just its keyword.
    assert!(symbols[0].range.end.line > symbols[0].range.start.line);
}

/// Inlay hints show the things the source cannot: the position a verb moves a
/// control to, and the range a value is checked against.
#[test]
fn inlay_hints_show_what_the_source_does_not_say() {
    let source = "procedure P {\n    name \"P\"\n    category normal\n\
                  \n    start HYD_2_ELECTRIC_PUMP\n    set FUEL_PUMP_PRESSURE_TARGET = 22.5\n}\n";
    let server = Harness::new(&[("a.fe", source)], Some(DC10_MANIFEST));
    server.open("a.fe", source);

    let hints = server
        .request::<request::InlayHintRequest>(InlayHintParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("a.fe"),
            },
            range: Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 99,
                    character: 0,
                },
            },
            work_done_progress_params: Default::default(),
        })
        .expect("hints should resolve");

    let labels: Vec<String> = hints
        .iter()
        .map(|hint| match &hint.label {
            InlayHintLabel::String(text) => text.clone(),
            _ => panic!("expected string labels"),
        })
        .collect();
    assert!(labels.contains(&" → ON".to_string()), "{labels:?}");
    assert!(labels.contains(&" ‹0..50›".to_string()), "{labels:?}");
}

/// Formatting must not lose a comment. In a document about what to do when a
/// system fails, the note explaining *why* a step exists is the part worth
/// most.
#[test]
fn formatting_keeps_every_comment() {
    let source = "// A leading note.\nprocedure P{\n// why this step\ncheck HYD_2_ENGINE_PUMP // trailing\ncomplete\n}\n";
    let server = Harness::new(&[("a.fe", source)], Some(DC10_MANIFEST));
    server.open("a.fe", source);

    let edits = server
        .request::<request::Formatting>(DocumentFormattingParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("a.fe"),
            },
            options: FormattingOptions::default(),
            work_done_progress_params: Default::default(),
        })
        .expect("a well-formed file should format");

    let formatted = &edits[0].new_text;
    assert!(formatted.contains("// A leading note."), "{formatted}");
    assert!(formatted.contains("// why this step"), "{formatted}");
    assert!(formatted.contains("// trailing"), "{formatted}");
    assert!(
        formatted.contains("    check HYD_2_ENGINE_PUMP"),
        "{formatted}"
    );
}

/// A file that does not parse is left exactly as it is. Reflowing text the
/// server does not understand is how half-written work gets scrambled.
#[test]
fn a_file_that_does_not_parse_is_not_reformatted() {
    let source = "procedure P {\n    wait a < b < c\n";
    let server = Harness::new(&[("a.fe", source)], Some(DC10_MANIFEST));
    server.open("a.fe", source);

    let edits = server.request::<request::Formatting>(DocumentFormattingParams {
        text_document: TextDocumentIdentifier {
            uri: server.uri("a.fe"),
        },
        options: FormattingOptions::default(),
        work_done_progress_params: Default::default(),
    });
    assert!(edits.is_none() || edits.unwrap().is_empty());
}

/// Colour means "the registry knows this". An unregistered name gets none,
/// which is a fact worth seeing at a glance.
#[test]
fn semantic_tokens_only_cover_names_the_registry_knows() {
    let source = "procedure P {\n    name \"P\"\n    category normal\n    check HYD_2_ENGINE_PUMP\n    check MADE_UP\n}\n";
    let server = Harness::new(&[("a.fe", source)], Some(DC10_MANIFEST));
    server.open("a.fe", source);

    let response = server
        .request::<request::SemanticTokensFullRequest>(SemanticTokensParams {
            text_document: TextDocumentIdentifier {
                uri: server.uri("a.fe"),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .expect("tokens should resolve");

    let SemanticTokensResult::Tokens(tokens) = response else {
        panic!("expected tokens");
    };
    // `P` (the procedure), `normal` (the category) and `HYD_2_ENGINE_PUMP`.
    // `MADE_UP` is not registered and so is not coloured.
    assert_eq!(tokens.data.len(), 3, "{:#?}", tokens.data);
}
