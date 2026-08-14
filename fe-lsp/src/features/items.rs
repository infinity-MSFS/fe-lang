//! Turning a completion context into a list.
//!
//! The registry is what separates this from a word-completer. `open ` offers
//! valves and not switches; `set FUEL_XFEED_SELECTOR = ` offers the three
//! positions that control actually has. Those are answers, not guesses — and
//! when there is no manifest they are not offered at all, because then the
//! server does not know them either.

use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionItemLabelDetails, Documentation,
    InsertTextFormat, MarkupContent, MarkupKind,
};

use fe_project::{ControlSpec, SymbolRegistry};

use crate::analysis::Analysis;
use crate::completion::{CATEGORIES, COMMON_DURATIONS, Context};
use crate::locate::{Occurrence, Role, occurrences};

/// Snippets live with the server because the server is what serves them. A
/// client contributing its own copy as well would offer every one of them
/// twice.
const SNIPPETS: &str = include_str!("../../snippets/fe.json");

pub struct Snippet {
    pub prefix: String,
    pub body: String,
    pub description: String,
    pub context: SnippetContext,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SnippetContext {
    Top,
    Metadata,
    Step,
}

/// Read the snippet file, working out where each one belongs from its prefix.
pub fn snippets() -> Vec<Snippet> {
    const METADATA: &[&str] = &[
        "name",
        "description",
        "category",
        "priority",
        "revision",
        "trigger",
        "require",
    ];

    let parsed: serde_json::Value = match serde_json::from_str(SNIPPETS) {
        Ok(parsed) => parsed,
        Err(_) => return Vec::new(),
    };
    let Some(entries) = parsed.as_object() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.values() {
        let Some(prefix) = entry.get("prefix").and_then(|v| v.as_str()) else {
            continue;
        };
        let body = match entry.get("body") {
            Some(serde_json::Value::String(text)) => text.clone(),
            Some(serde_json::Value::Array(lines)) => lines
                .iter()
                .filter_map(|line| line.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            _ => continue,
        };
        let context = if prefix == "procedure" {
            SnippetContext::Top
        } else if METADATA.contains(&prefix) {
            SnippetContext::Metadata
        } else {
            SnippetContext::Step
        };
        out.push(Snippet {
            prefix: prefix.to_string(),
            body,
            description: entry
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            context,
        });
    }
    out.sort_by(|a, b| a.prefix.cmp(&b.prefix));
    out
}

/// `sortText` decides the order of the list, which is not the order the labels
/// would sort themselves into. A procedure should be offered `name` before
/// `revision`, and `500ms` before `5m` — the order they are written in, not the
/// order they spell.
fn ordered(group: u8, order: usize) -> String {
    format!("{group}{order:03}")
}

fn snippet_item(snippet: &Snippet, group: u8, order: usize) -> CompletionItem {
    CompletionItem {
        label: snippet.prefix.clone(),
        kind: Some(CompletionItemKind::SNIPPET),
        detail: Some(snippet.description.clone()),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("```fe\n{}\n```", preview(&snippet.body)),
        })),
        insert_text: Some(snippet.body.clone()),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        sort_text: Some(ordered(group, order)),
        ..Default::default()
    }
}

/// A snippet body with its placeholders resolved, for the documentation popup.
fn preview(body: &str) -> String {
    let mut out = String::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('{') => {
                chars.next();
                let mut inner = String::new();
                let mut depth = 1;
                for c in chars.by_ref() {
                    match c {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    if depth > 0 {
                        inner.push(c);
                    }
                }
                // `1:text` keeps `text`; `1|a,b|` keeps `a`; `1` keeps nothing.
                let body = inner.split_once(':').map(|(_, rest)| rest.to_string());
                let choice = inner
                    .split_once('|')
                    .and_then(|(_, rest)| rest.split(',').next().map(str::to_string));
                out.push_str(&body.or(choice).unwrap_or_default());
            }
            Some(c) if c.is_ascii_digit() => {
                while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                    chars.next();
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn value_item(
    label: String,
    kind: CompletionItemKind,
    detail: &str,
    sort: String,
) -> CompletionItem {
    CompletionItem {
        label,
        kind: Some(kind),
        label_details: (!detail.is_empty()).then(|| CompletionItemLabelDetails {
            detail: None,
            description: Some(detail.to_string()),
        }),
        detail: (!detail.is_empty()).then(|| detail.to_string()),
        sort_text: Some(sort),
        ..Default::default()
    }
}

/// Describe a control the way its row in `docs/symbols.md` does.
pub fn describe(spec: &ControlSpec) -> String {
    match spec {
        ControlSpec::Analog { min, max } => format!("analog {min}..{max}"),
        ControlSpec::Selector(positions) => format!("selector {}", positions.join(" | ")),
        ControlSpec::Switch => "switch OFF | ON".to_string(),
        ControlSpec::Valve => "valve CLOSED | OPEN".to_string(),
        ControlSpec::Checklist => "checklist".to_string(),
    }
}

pub fn completions(
    context: &Context,
    analysis: &Analysis,
    snippets: &[Snippet],
) -> Vec<CompletionItem> {
    let registry = analysis.registry.as_ref();
    let in_context = |where_: SnippetContext| {
        snippets
            .iter()
            .filter(move |snippet| snippet.context == where_)
    };

    match context {
        Context::None => Vec::new(),

        Context::Top => in_context(SnippetContext::Top)
            .enumerate()
            .map(|(index, snippet)| snippet_item(snippet, 0, index))
            .collect(),

        // A step is legal wherever metadata is, so both are offered — metadata
        // first, since that is what comes next in a procedure being written.
        Context::Metadata => in_context(SnippetContext::Metadata)
            .enumerate()
            .map(|(index, snippet)| snippet_item(snippet, 0, index))
            .chain(
                in_context(SnippetContext::Step)
                    .enumerate()
                    .map(|(index, snippet)| snippet_item(snippet, 1, index)),
            )
            .collect(),

        Context::Step => in_context(SnippetContext::Step)
            .enumerate()
            .map(|(index, snippet)| snippet_item(snippet, 0, index))
            .collect(),

        Context::Category => CATEGORIES
            .iter()
            .enumerate()
            .map(|(index, category)| {
                value_item(
                    category.to_string(),
                    CompletionItemKind::ENUM_MEMBER,
                    "category",
                    ordered(0, index),
                )
            })
            .collect(),

        Context::Duration => COMMON_DURATIONS
            .iter()
            .enumerate()
            .map(|(index, duration)| {
                value_item(
                    duration.to_string(),
                    CompletionItemKind::UNIT,
                    "duration — ms, s or m",
                    ordered(0, index),
                )
            })
            .collect(),

        // Only controls the verb is valid on. Offering a switch after `open`
        // would be offering E0206.
        Context::Control { verb } => match registry {
            Some(registry) => registry
                .controls()
                .filter(|control| verb.accepted_by(control.spec.clone()))
                .map(|control| {
                    value_item(
                        control.name.clone(),
                        CompletionItemKind::CONSTANT,
                        &describe(&control.spec),
                        format!("0{}", control.name),
                    )
                })
                .collect(),
            None => harvested(
                analysis,
                |role| matches!(role, Role::Control { .. }),
                "control",
            ),
        },

        // The positions this control actually has — not a list of the ones
        // aircraft tend to have.
        Context::Position { control } => match registry.and_then(|r| r.control(control)) {
            Some(control) => match &control.spec {
                ControlSpec::Analog { min, max } => vec![value_item(
                    format!("{min}"),
                    CompletionItemKind::VALUE,
                    &format!("{min}..{max}"),
                    ordered(0, 0),
                )],
                spec => spec
                    .positions()
                    .iter()
                    .enumerate()
                    .map(|(index, position)| {
                        value_item(
                            position.to_string(),
                            CompletionItemKind::ENUM_MEMBER,
                            &describe(spec),
                            ordered(0, index),
                        )
                    })
                    .collect(),
            },
            // An unknown control has no positions to offer, and inventing some
            // would be inventing the aircraft.
            None => harvested(
                analysis,
                |role| matches!(role, Role::Position { .. }),
                "used here",
            ),
        },

        Context::Expression => {
            let mut items = match registry {
                Some(registry) => registry
                    .states()
                    .map(|state| {
                        value_item(
                            state.name.clone(),
                            CompletionItemKind::PROPERTY,
                            state.ty.as_str(),
                            format!("0{}", state.name),
                        )
                    })
                    .collect(),
                None => harvested(
                    analysis,
                    |role| matches!(role, Role::State),
                    "aircraft state",
                ),
            };
            for (index, literal) in ["true", "false"].iter().enumerate() {
                items.push(value_item(
                    literal.to_string(),
                    CompletionItemKind::VALUE,
                    "",
                    ordered(1, index),
                ));
            }
            items
        }

        Context::Procedure => analysis
            .procedures()
            .map(|(_, decl)| {
                value_item(
                    decl.id.text.clone(),
                    CompletionItemKind::FUNCTION,
                    decl.metadata
                        .name
                        .as_ref()
                        .map(|n| n.value.as_str())
                        .unwrap_or_default(),
                    format!("0{}", decl.id.text),
                )
            })
            .collect(),
    }
}

/// Names the project's own files already use.
///
/// This is the fallback when there is no manifest: a fast way to retype a name
/// you have written before, and explicitly not a claim that the name is real.
fn harvested(
    analysis: &Analysis,
    wanted: impl Fn(&Role) -> bool,
    detail: &str,
) -> Vec<CompletionItem> {
    let mut names: Vec<String> = analysis
        .asts()
        .flat_map(|(unit, ast)| occurrences(unit, ast))
        .filter(|occurrence: &Occurrence| wanted(&occurrence.role))
        .map(|occurrence| occurrence.text)
        .collect();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .map(|name| {
            let sort = format!("0{name}");
            value_item(name, CompletionItemKind::TEXT, detail, sort)
        })
        .collect()
}

/// Whether the server knows enough to answer authoritatively.
pub fn is_authoritative(registry: Option<&SymbolRegistry>) -> bool {
    registry.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_snippet_is_placed() {
        let snippets = snippets();
        assert_eq!(snippets.len(), 25, "all of snippets/fe.json should load");
        assert_eq!(
            snippets
                .iter()
                .filter(|s| s.context == SnippetContext::Top)
                .map(|s| s.prefix.as_str())
                .collect::<Vec<_>>(),
            ["procedure"]
        );
        assert!(
            snippets
                .iter()
                .any(|s| s.prefix == "category" && s.context == SnippetContext::Metadata)
        );
        assert!(
            snippets
                .iter()
                .any(|s| s.prefix == "check" && s.context == SnippetContext::Step)
        );
    }

    #[test]
    fn placeholders_resolve_for_the_preview() {
        assert_eq!(
            preview("wait ${1:system.state} ${2:>} ${3:0}"),
            "wait system.state > 0"
        );
        assert_eq!(
            preview("category ${1|abnormal,normal|}"),
            "category abnormal"
        );
        assert_eq!(preview("notify \"$1\"$0"), "notify \"\"");
    }

    #[test]
    fn a_control_reads_the_way_its_documentation_does() {
        assert_eq!(describe(&ControlSpec::switch()), "switch OFF | ON");
        assert_eq!(describe(&ControlSpec::valve()), "valve CLOSED | OPEN");
        assert_eq!(describe(&ControlSpec::analog(0.0, 50.0)), "analog 0..50");
        assert_eq!(
            describe(&ControlSpec::selector(["OFF", "TANK_1_TO_3"])),
            "selector OFF | TANK_1_TO_3"
        );
    }
}
