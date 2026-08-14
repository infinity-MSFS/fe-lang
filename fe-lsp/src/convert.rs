//! Compiler diagnostics as protocol diagnostics.
//!
//! `fe-lang` already produces the structured form an editor wants — a stable
//! code, a primary span, secondary spans that say where the *other* thing is,
//! notes and a suggestion. `Diagnostic::render` turns that into text for a
//! terminal; this turns the same values into something a client can underline.

use lsp_types::{
    Diagnostic as LspDiagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location,
    NumberOrString, Uri,
};

use fe_compiler::{Diagnostic, Severity};

use crate::analysis::Analysis;
use crate::line_index::{Encoding, LineIndex};
use crate::uri;

const DOCS: &str = "https://github.com/infinity-MSFS/fe-lang/blob/main/docs/diagnostics.md";

/// Where in `docs/diagnostics.md` a code is described.
fn documentation(code: &str) -> Option<Uri> {
    let section = match code.as_bytes() {
        [b'W', ..] => "warnings",
        [b'E', b'0', b'0', ..] => "lexical",
        [b'E', b'0', b'1', ..] => "syntax",
        [b'E', b'0', b'2', ..] => "semantic",
        _ => "internal",
    };
    format!("{DOCS}#{section}").parse().ok()
}

fn severity(severity: Severity) -> DiagnosticSeverity {
    match severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
    }
}

/// The message a client shows.
///
/// Notes and help are folded in rather than dropped, because they are usually
/// the part that resolves the problem — E0201's "did you mean `HYD_2_ENGINE_PUMP`?"
/// is worth more than the word "unknown". A client showing only the first line
/// still gets the message proper.
fn message(diagnostic: &Diagnostic) -> String {
    let mut out = diagnostic.message.clone();
    if let Some(label) = &diagnostic.primary.message {
        out.push_str("\n\n");
        out.push_str(label);
    }
    for note in &diagnostic.notes {
        out.push_str("\nnote: ");
        out.push_str(note);
    }
    if let Some(help) = &diagnostic.help {
        out.push_str("\nhelp: ");
        out.push_str(help);
    }
    out
}

pub fn diagnostic(
    analysis: &Analysis,
    indexes: &dyn Fn(fe_lang::span::UnitId) -> Option<LineIndex>,
    diagnostic: &Diagnostic,
    encoding: Encoding,
) -> Option<LspDiagnostic> {
    let span = diagnostic.primary.span;
    let text = analysis.text(span.unit)?;
    let index = indexes(span.unit)?;

    // A secondary label points at the other end of the story — where the first
    // definition was, which `require` conflicts. Losing it would leave the
    // author knowing something is duplicated and not where.
    let related: Vec<DiagnosticRelatedInformation> = diagnostic
        .secondary
        .iter()
        .filter_map(|label| {
            let text = analysis.text(label.span.unit)?;
            let index = indexes(label.span.unit)?;
            let path = analysis.path(label.span.unit)?;
            Some(DiagnosticRelatedInformation {
                location: Location {
                    uri: uri::from_path(path)?,
                    range: index.range(text, label.span, encoding),
                },
                message: label
                    .message
                    .clone()
                    .unwrap_or_else(|| "related".to_string()),
            })
        })
        .collect();

    Some(LspDiagnostic {
        range: index.range(text, span, encoding),
        severity: Some(severity(diagnostic.severity)),
        code: Some(NumberOrString::String(diagnostic.code.to_string())),
        code_description: documentation(diagnostic.code)
            .map(|href| lsp_types::CodeDescription { href }),
        source: Some("fe".to_string()),
        message: message(diagnostic),
        related_information: (!related.is_empty()).then_some(related),
        tags: None,
        data: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_link_to_their_section() {
        let of = |code: &str| documentation(code).unwrap().as_str().to_string();
        assert!(of("E0001").ends_with("#lexical"));
        assert!(of("E0107").ends_with("#syntax"));
        assert!(of("E0201").ends_with("#semantic"));
        assert!(of("W0002").ends_with("#warnings"));
        assert!(of("E0999").ends_with("#internal"));
    }

    #[test]
    fn the_suggestion_survives_into_the_message() {
        use fe_compiler::{Diagnostic, Label};
        use fe_lang::span::{Span, UnitId};

        let span = Span::new(UnitId(0), 0, 4);
        let diagnostic = Diagnostic::error(
            "E0201",
            "unknown aircraft symbol",
            Label::new(span, "`X` is not registered"),
        )
        .with_help("did you mean `HYD_2_ENGINE_PUMP`?");

        let text = message(&diagnostic);
        assert!(text.starts_with("unknown aircraft symbol"), "{text}");
        assert!(text.contains("`X` is not registered"), "{text}");
        assert!(text.contains("did you mean `HYD_2_ENGINE_PUMP`?"), "{text}");
    }
}
