//! Semantic tokens, and inlay hints.
//!
//! # Why only names
//!
//! `editors/README.md` warns that the TextMate grammar and the tree-sitter
//! grammar can drift, and lists a six-step checklist for keeping them in step.
//! Semantic tokens do not replace either — clients layer them *over* grammar
//! tokens — so this emits only what a grammar cannot get right: whether a name
//! is a control the procedure moves or state it reads, and whether the aircraft
//! has heard of it at all. Keywords, strings, numbers and comments stay with the
//! grammar, where they are cheap and already correct.
//!
//! An unregistered name is deliberately left uncoloured. Colour here means "the
//! registry knows this", which is a fact worth seeing at a glance.

use lsp_types::{SemanticToken, SemanticTokenType};

use crate::analysis::Analysis;
use crate::line_index::{Encoding, LineIndex};
use crate::locate::{Occurrence, Role, occurrences};

/// The legend, in the order the indices below refer to.
pub const LEGEND: &[SemanticTokenType] = &[
    SemanticTokenType::FUNCTION,    // a procedure
    SemanticTokenType::PROPERTY,    // aircraft state, which is read
    SemanticTokenType::VARIABLE,    // a control, which is moved
    SemanticTokenType::ENUM_MEMBER, // a position, or a category
];

const FUNCTION: u32 = 0;
const PROPERTY: u32 = 1;
const VARIABLE: u32 = 2;
const ENUM_MEMBER: u32 = 3;

fn token_type(analysis: &Analysis, occurrence: &Occurrence) -> Option<u32> {
    let registry = analysis.registry.as_ref();
    match &occurrence.role {
        Role::ProcedureDecl | Role::ProcedureRef => {
            analysis.procedure(&occurrence.text).map(|_| FUNCTION)
        }
        // Colour by what the registry says it *is*, not by where it appears —
        // so a control written into a condition shows up as a control, which is
        // the mistake E0203 is about.
        Role::Control { .. } | Role::State => match registry?.resolve(&occurrence.text)? {
            fe_compiler::Resolved::State(_) => Some(PROPERTY),
            fe_compiler::Resolved::Control(_) => Some(VARIABLE),
        },
        Role::Position { control } => registry?
            .control(control)
            .filter(|c| c.spec.position_index(&occurrence.text).is_some())
            .map(|_| ENUM_MEMBER),
        Role::Category => crate::completion::CATEGORIES
            .contains(&occurrence.text.as_str())
            .then_some(ENUM_MEMBER),
    }
}

pub fn semantic_tokens(
    analysis: &Analysis,
    unit: fe_lang::span::UnitId,
    index: &LineIndex,
    encoding: Encoding,
) -> Vec<SemanticToken> {
    let Some(ast) = analysis.ast(unit) else {
        return Vec::new();
    };
    let Some(text) = analysis.text(unit) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    let (mut last_line, mut last_start) = (0u32, 0u32);

    for occurrence in occurrences(unit, ast) {
        let Some(token_type) = token_type(analysis, &occurrence) else {
            continue;
        };
        let range = index.range(text, occurrence.span, encoding);
        // A name never wraps, and the protocol's encoding cannot express one
        // that does.
        if range.start.line != range.end.line {
            continue;
        }

        let delta_line = range.start.line - last_line;
        let delta_start = if delta_line == 0 {
            range.start.character - last_start
        } else {
            range.start.character
        };
        out.push(SemanticToken {
            delta_line,
            delta_start,
            length: range.end.character - range.start.character,
            token_type,
            token_modifiers_bitset: 0,
        });
        last_line = range.start.line;
        last_start = range.start.character;
    }
    out
}

/// Facts the source does not say, shown where they apply.
pub fn inlay_hints(
    analysis: &Analysis,
    unit: fe_lang::span::UnitId,
) -> Vec<(fe_lang::span::Span, String)> {
    use fe_lang::ast::*;

    let (Some(ast), Some(registry)) = (analysis.ast(unit), analysis.registry.as_ref()) else {
        return Vec::new();
    };
    let mut out = Vec::new();

    fn walk(
        block: &Block,
        registry: &fe_project::SymbolRegistry,
        out: &mut Vec<(fe_lang::span::Span, String)>,
    ) {
        for step in &block.steps {
            match step {
                // `start HYD_2_ELECTRIC_PUMP` moves it to ON. The source says
                // the verb; the position is the registry's to know.
                Step::Verb { verb, control, .. } => {
                    if registry.control(&control.text).is_some() {
                        out.push((control.span, format!(" → {}", verb.position())));
                    }
                }
                // The registered range, next to the value being checked
                // against it. E0207 is a compile error, so seeing the limits
                // while typing is the difference between a build and a fix.
                Step::Set { control, value, .. } => {
                    if let (SetValue::Number(number), Some(control)) =
                        (value, registry.control(&control.text))
                    {
                        if let fe_project::ControlSpec::Analog { min, max } = control.spec {
                            out.push((number.span, format!(" ‹{min}..{max}›")));
                        }
                    }
                }
                Step::Wait { timeout, .. } | Step::Complete { timeout, .. } => {
                    if let Some(timeout) = timeout {
                        out.push((timeout.span, format!(" ‹{} ms›", timeout.millis)));
                    }
                }
                Step::If(step) => {
                    walk(&step.then_block, registry, out);
                    match &step.else_branch {
                        Some(ElseBranch::Block(b)) => walk(b, registry, out),
                        Some(ElseBranch::If(nested)) => {
                            let block = Block {
                                steps: vec![Step::If((**nested).clone())],
                                span: nested.span,
                            };
                            walk(&block, registry, out);
                        }
                        None => {}
                    }
                }
                _ => {}
            }
        }
    }

    for decl in &ast.procedures {
        walk(&decl.body, registry, &mut out);
    }
    out
}
