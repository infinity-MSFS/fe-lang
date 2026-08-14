//! The server, end to end.

mod harness;

use harness::{DC10_MANIFEST, Harness, at};
use lsp_types::*;

const HYDRAULIC: &str = include_str!("../../examples/dc10/hydraulic.fe");
const ELECTRICAL: &str = include_str!("../../examples/dc10/electrical.fe");

/// The strongest single assertion available: the repository's own example
/// procedures compile clean, so opening one must produce nothing at all. Any
/// false positive anywhere in the pipeline shows up here.
#[test]
fn the_examples_report_nothing() {
    let server = Harness::new(
        &[("hydraulic.fe", HYDRAULIC), ("electrical.fe", ELECTRICAL)],
        Some(DC10_MANIFEST),
    );
    server.open("hydraulic.fe", HYDRAULIC);
    assert_eq!(server.diagnostics("hydraulic.fe"), []);
}

/// `open` on a switch is E0206. The span is the verb, which is the word that is
/// actually wrong — the control is fine, it is the thing being asked of it that
/// is not — and the range has to reach only as far as that word.
#[test]
fn an_invalid_action_is_reported_where_it_is_written() {
    let source =
        "procedure P {\n    name \"P\"\n    category normal\n    open HYD_2_ELECTRIC_PUMP\n}\n";
    let server = Harness::new(&[("a.fe", source)], Some(DC10_MANIFEST));
    server.open("a.fe", source);

    let diagnostics = server.diagnostics("a.fe");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].code,
        Some(NumberOrString::String("E0206".to_string()))
    );
    assert_eq!(diagnostics[0].range.start, at(source, "open HYD"));
    assert_eq!(diagnostics[0].range.end, harness::position(3, 8));
    assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::ERROR));
    assert!(diagnostics[0].message.contains("accepts: OFF, ON"));
}

/// A fixed error has to disappear. Publishing only the files that still have
/// problems would leave the squiggle there for good.
#[test]
fn fixing_an_error_clears_it() {
    let broken =
        "procedure P {\n    name \"P\"\n    category normal\n    open HYD_2_ELECTRIC_PUMP\n}\n";
    let fixed =
        "procedure P {\n    name \"P\"\n    category normal\n    start HYD_2_ELECTRIC_PUMP\n}\n";

    let server = Harness::new(&[("a.fe", broken)], Some(DC10_MANIFEST));
    server.open("a.fe", broken);
    assert_eq!(server.codes("a.fe"), ["E0206"]);

    server.change("a.fe", fixed);
    assert_eq!(server.diagnostics("a.fe"), []);
}

/// Procedure identifiers share one namespace across every file compiled
/// together, so a duplicate is only visible to something looking at all of
/// them — including the file that is not open.
#[test]
fn a_duplicate_across_files_is_reported_in_both() {
    let one = "procedure P {\n    name \"one\"\n    category normal\n    complete\n}\n";
    let two = "procedure P {\n    name \"two\"\n    category normal\n    complete\n}\n";

    let server = Harness::new(&[("a.fe", one), ("b.fe", two)], Some(DC10_MANIFEST));
    server.open("a.fe", one);

    // Reported once, against the later definition, with the first as related
    // information — which is the only way to find the other half.
    let mut found = None;
    for name in ["a.fe", "b.fe"] {
        let diagnostics = server.diagnostics(name);
        if let Some(diagnostic) = diagnostics
            .iter()
            .find(|d| d.code == Some(NumberOrString::String("E0209".to_string())))
        {
            found = Some(diagnostic.clone());
        }
    }
    let diagnostic = found.expect("a duplicate procedure should be reported");
    let related = diagnostic
        .related_information
        .expect("the first definition should be pointed at");
    assert_eq!(related.len(), 1);
    assert!(related[0].message.contains("first defined"));
}

/// Without a manifest the server has no registry, and analysing against an
/// empty one would paint every name in the file red. Syntax errors still have
/// to arrive.
#[test]
fn no_manifest_means_syntax_only() {
    let source =
        "procedure P {\n    name \"P\"\n    category normal\n    check TOTALLY_MADE_UP\n}\n";
    let server = Harness::new(&[("a.fe", source)], None);
    server.open("a.fe", source);
    assert_eq!(server.diagnostics("a.fe"), []);

    let broken = "procedure P {\n    wait a < b < c\n}\n";
    server.change("a.fe", broken);
    assert!(server.codes("a.fe").contains(&"E0107".to_string()));
}

/// Adding the manifest turns the checking on, without a restart.
#[test]
fn writing_a_manifest_enables_semantic_checking() {
    let source =
        "procedure P {\n    name \"P\"\n    category normal\n    check TOTALLY_MADE_UP\n}\n";
    let server = Harness::new(&[("a.fe", source)], None);
    server.open("a.fe", source);
    assert_eq!(server.diagnostics("a.fe"), []);

    server.write("fe.toml", DC10_MANIFEST);
    server.watched("fe.toml", FileChangeType::CREATED);
    assert_eq!(server.codes("a.fe"), ["E0201"]);
}

/// A manifest the server cannot read is reported against the manifest, where
/// it can be fixed — and the server falls back rather than going quiet.
#[test]
fn a_broken_manifest_is_reported_against_itself() {
    let source = "procedure P {\n    name \"P\"\n    category normal\n    complete\n}\n";
    let server = Harness::new(
        &[("a.fe", source)],
        Some("[controls]\nA = { kind = \"toggle\", tag = 1 }\n"),
    );
    server.open("a.fe", source);

    let diagnostics = server.diagnostics("fe.toml");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert!(
        diagnostics[0].message.contains("unknown control kind"),
        "{}",
        diagnostics[0].message
    );
    // …and the range points at the offending value, not at the top of the file.
    assert!(diagnostics[0].range.start.line > 0);
}

/// A file that is not open is still part of the project, and a change to it
/// from outside the editor still has to re-check everything.
#[test]
fn a_file_changed_on_disk_is_picked_up() {
    let good = "procedure OTHER {\n    name \"other\"\n    category normal\n    complete\n}\n";
    let caller = "procedure P {\n    name \"P\"\n    category normal\n    call OTHER\n}\n";

    let server = Harness::new(&[("a.fe", caller), ("b.fe", good)], Some(DC10_MANIFEST));
    server.open("a.fe", caller);
    assert_eq!(server.diagnostics("a.fe"), []);

    // The procedure `a.fe` calls is deleted behind the editor's back.
    server.write("b.fe", "");
    server.watched("b.fe", FileChangeType::CHANGED);
    assert_eq!(server.codes("a.fe"), ["E0208"]);
}

/// An open buffer is the client's, and a filesystem event must not overwrite
/// unsaved work with what is on disk.
#[test]
fn an_open_buffer_wins_over_the_file_on_disk() {
    let on_disk =
        "procedure P {\n    name \"P\"\n    category normal\n    open HYD_2_ELECTRIC_PUMP\n}\n";
    let in_editor =
        "procedure P {\n    name \"P\"\n    category normal\n    start HYD_2_ELECTRIC_PUMP\n}\n";

    let server = Harness::new(&[("a.fe", on_disk)], Some(DC10_MANIFEST));
    server.open("a.fe", in_editor);
    assert_eq!(server.diagnostics("a.fe"), []);

    server.watched("a.fe", FileChangeType::CHANGED);
    assert_eq!(
        server.diagnostics("a.fe"),
        [],
        "the unsaved buffer should still be what is checked"
    );
}

/// A limit only code generation can see.
///
/// The runtime evaluates conditions on a fixed 32-slot stack, and whether a
/// condition needs more than that is not knowable until it has been compiled —
/// `fe_compiler::check` stops before that and would report nothing. An author
/// told their procedure is fine, whose build then rejects it, has been let down
/// by the editor rather than helped by it.
#[test]
fn a_limit_only_code_generation_can_see_is_still_reported() {
    // Right-nested, so each level has to hold its left operand while the rest
    // is evaluated. Flat `a && b && c` folds as it goes and never grows.
    let mut condition = String::from("engine.2.running");
    for _ in 0..40 {
        condition = format!("engine.2.running && ({condition})");
    }
    let source = format!(
        "procedure DEEP {{\n    name \"deep\"\n    category normal\n    wait {condition}\n}}\n"
    );

    let server = Harness::new(&[("a.fe", &source)], Some(DC10_MANIFEST));
    server.open("a.fe", &source);
    assert_eq!(server.codes("a.fe"), ["E0216"]);
}

/// Warnings are warnings, not errors: a float comparison is worth flagging and
/// is not a reason to refuse the file.
#[test]
fn warnings_arrive_as_warnings() {
    let source = "procedure P {\n    name \"P\"\n    category normal\n\
                  \n    wait hydraulic.2.pressure == 2500.5\n    complete\n}\n";
    let server = Harness::new(&[("a.fe", source)], Some(DC10_MANIFEST));
    server.open("a.fe", source);

    let diagnostics = server.diagnostics("a.fe");
    assert!(!diagnostics.is_empty());
    assert!(
        diagnostics
            .iter()
            .all(|d| d.severity == Some(DiagnosticSeverity::WARNING)),
        "{diagnostics:#?}"
    );
}

/// Every diagnostic carries its code and a link to where the code is written
/// down, so `E0206` is a question the editor can answer.
#[test]
fn a_diagnostic_links_to_its_documentation() {
    let source =
        "procedure P {\n    name \"P\"\n    category normal\n    open HYD_2_ELECTRIC_PUMP\n}\n";
    let server = Harness::new(&[("a.fe", source)], Some(DC10_MANIFEST));
    server.open("a.fe", source);

    let diagnostics = server.diagnostics("a.fe");
    let href = diagnostics[0]
        .code_description
        .as_ref()
        .expect("a code should be documented");
    assert!(href.href.as_str().ends_with("diagnostics.md#semantic"));
    assert_eq!(diagnostics[0].source.as_deref(), Some("fe"));
}
