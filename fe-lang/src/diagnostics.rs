use core::fmt::Write as _;

use crate::span::{SourceMap, Span};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity {
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Label {
    pub span: Span,
    pub message: Option<String>,
}

impl Label {
    pub fn new(span: Span, message: impl Into<String>) -> Label {
        Label {
            span,
            message: Some(message.into()),
        }
    }

    pub fn bare(span: Span) -> Label {
        Label {
            span,
            message: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub primary: Label,
    pub secondary: Vec<Label>,
    pub notes: Vec<String>,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn error(code: &'static str, message: impl Into<String>, primary: Label) -> Diagnostic {
        Diagnostic {
            code,
            severity: Severity::Error,
            message: message.into(),
            primary,
            secondary: Vec::new(),
            notes: Vec::new(),
            help: None,
        }
    }

    pub fn warning(code: &'static str, message: impl Into<String>, primary: Label) -> Diagnostic {
        Diagnostic {
            severity: Severity::Warning,
            ..Diagnostic::error(code, message, primary)
        }
    }

    pub fn with_secondary(mut self, label: Label) -> Diagnostic {
        self.secondary.push(label);
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Diagnostic {
        self.notes.push(note.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Diagnostic {
        self.help = Some(help.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    pub fn render(&self, sources: &SourceMap<'_>) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{}[{}]: {}",
            self.severity.as_str(),
            self.code,
            self.message
        );
        let width = self
            .secondary
            .iter()
            .chain(core::iter::once(&self.primary))
            .map(|label| sources.location(label.span).line.to_string().len())
            .max()
            .unwrap_or(1);

        render_label(&mut out, sources, &self.primary, true, width);
        for label in &self.secondary {
            render_label(&mut out, sources, label, false, width);
        }
        let pad = " ".repeat(width);
        if !self.notes.is_empty() || self.help.is_some() {
            let _ = writeln!(out, "{pad} |");
        }
        for note in &self.notes {
            let _ = writeln!(out, "{pad} = note: {note}");
        }
        if let Some(help) = &self.help {
            let _ = writeln!(out, "{pad} = help: {help}");
        }
        out
    }
}

fn render_label(
    out: &mut String,
    sources: &SourceMap<'_>,
    label: &Label,
    primary: bool,
    width: usize,
) {
    let loc = sources.location(label.span);
    let pad = " ".repeat(width);
    let _ = writeln!(
        out,
        "{pad}--> {}:{}:{}",
        sources.name(label.span.unit),
        loc.line,
        loc.column
    );
    let line = sources.line_text(label.span);
    let gutter = format!("{:>width$}", loc.line, width = width);
    let _ = writeln!(out, "{pad} |");
    let _ = writeln!(out, "{gutter} | {line}");
    let caret = if primary { '^' } else { '-' };
    let width = label.span.len().max(1);
    let _ = write!(
        out,
        "{pad} | {}{}",
        " ".repeat(loc.column.saturating_sub(1) as usize),
        core::iter::repeat(caret).take(width).collect::<String>()
    );
    match &label.message {
        Some(message) => {
            let _ = writeln!(out, " {message}");
        }
        None => {
            let _ = writeln!(out);
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new() -> Diagnostics {
        Diagnostics::default()
    }

    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.items.push(diagnostic);
    }

    pub fn extend(&mut self, other: Diagnostics) {
        self.items.extend(other.items);
    }

    pub fn has_errors(&self) -> bool {
        self.items.iter().any(Diagnostic::is_error)
    }

    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.items.iter().filter(|d| d.is_error())
    }

    pub fn warnings(&self) -> impl Iterator<Item = &Diagnostic> {
        self.items.iter().filter(|d| !d.is_error())
    }

    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic> {
        self.items.iter()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn into_vec(self) -> Vec<Diagnostic> {
        self.items
    }

    pub fn render(&self, sources: &SourceMap<'_>) -> String {
        let mut out = String::new();
        for item in &self.items {
            out.push_str(&item.render(sources));
            out.push('\n');
        }
        out
    }
}

pub mod codes {
    // lexical (E00xx)
    pub const UNEXPECTED_CHARACTER: &str = "E0001";
    pub const UNTERMINATED_STRING: &str = "E0002";
    pub const INVALID_ESCAPE: &str = "E0003";
    pub const MALFORMED_NUMBER: &str = "E0004";
    pub const UNTERMINATED_COMMENT: &str = "E0005";
    pub const INVALID_DURATION: &str = "E0006";

    // syntax
    pub const EXPECTED_TOKEN: &str = "E0101";
    pub const EXPECTED_DECLARATION: &str = "E0102";
    pub const EXPECTED_STEP: &str = "E0103";
    pub const EXPECTED_EXPRESSION: &str = "E0104";
    pub const METADATA_AFTER_STEPS: &str = "E0105";
    pub const DUPLICATE_METADATA: &str = "E0106";
    pub const CHAINED_COMPARISON: &str = "E0107";
    pub const EMPTY_PROCEDURE: &str = "E0108";

    // semantic
    pub const UNKNOWN_SYMBOL: &str = "E0201";
    pub const NOT_A_CONTROL: &str = "E0202";
    pub const NOT_READABLE: &str = "E0203";
    pub const TYPE_MISMATCH: &str = "E0204";
    pub const INVALID_CONTROL_VALUE: &str = "E0205";
    pub const INVALID_ACTION_FOR_CONTROL: &str = "E0206";
    pub const VALUE_OUT_OF_RANGE: &str = "E0207";
    pub const UNKNOWN_PROCEDURE: &str = "E0208";
    pub const DUPLICATE_PROCEDURE: &str = "E0209";
    pub const RECURSIVE_CALL: &str = "E0210";
    pub const MISSING_METADATA: &str = "E0211";
    pub const INVALID_METADATA_VALUE: &str = "E0212";
    pub const INVALID_TIMEOUT: &str = "E0213";
    pub const NESTING_TOO_DEEP: &str = "E0214";
    pub const CALL_DEPTH_EXCEEDED: &str = "E0215";
    pub const PROCEDURE_TOO_COMPLEX: &str = "E0216";
    pub const DATABASE_TOO_LARGE: &str = "E0217";

    //  warnings
    pub const UNREACHABLE_STEP: &str = "W0001";
    pub const FLOAT_EQUALITY: &str = "W0002";
    pub const EMPTY_BRANCH: &str = "W0003";
    pub const CONSTANT_CONDITION: &str = "W0005";
}
