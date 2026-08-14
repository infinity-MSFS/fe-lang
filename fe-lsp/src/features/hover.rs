//! What a name means, on hover.
//!
//! The two most useful facts are the ones the source cannot show: a state
//! path's type, and a control's kind. `hydraulic.2.pressure > 2500` reads the
//! same whether that symbol is a number or a bool, and the difference is E0204.

use fe_lang::ast::ProcedureDecl;
use fe_project::{ControlSpec, SymbolRegistry};

use crate::analysis::Analysis;
use crate::features::items::describe;
use crate::locate::{ControlVerb, Occurrence, Role};

pub fn markdown(analysis: &Analysis, occurrence: &Occurrence) -> Option<String> {
    match &occurrence.role {
        Role::ProcedureDecl | Role::ProcedureRef => {
            let (_, decl) = analysis.procedure(&occurrence.text)?;
            Some(procedure(decl))
        }
        Role::Control { verb } => {
            let registry = analysis.registry.as_ref()?;
            Some(control(registry, &occurrence.text, *verb))
        }
        Role::State => {
            let registry = analysis.registry.as_ref()?;
            Some(state(registry, &occurrence.text))
        }
        Role::Position { control } => {
            let registry = analysis.registry.as_ref()?;
            let control = registry.control(control)?;
            Some(format!(
                "**{}**\n\nA position of `{}` — {}.",
                occurrence.text,
                control.name,
                describe(&control.spec)
            ))
        }
        Role::Category => Some(category(&occurrence.text)),
    }
}

fn procedure(decl: &ProcedureDecl) -> String {
    let mut out = format!("```fe\nprocedure {}\n```\n", decl.id.text);
    if let Some(name) = &decl.metadata.name {
        out.push_str(&format!("\n**{}**\n", name.value));
    }
    if let Some(description) = &decl.metadata.description {
        out.push_str(&format!("\n{}\n", description.value));
    }

    let mut facts = Vec::new();
    if let Some(category) = &decl.metadata.category {
        facts.push(format!("category `{}`", category.text));
    }
    if let Some(priority) = &decl.metadata.priority {
        facts.push(format!("priority `{}`", priority.value));
    }
    if let Some(revision) = &decl.metadata.revision {
        facts.push(format!("revision `{}`", revision.value));
    }
    if decl.metadata.trigger.is_some() {
        facts.push("has a trigger".to_string());
    }
    if !decl.metadata.requires.is_empty() {
        facts.push(format!("{} precondition(s)", decl.metadata.requires.len()));
    }
    if !facts.is_empty() {
        out.push_str(&format!("\n{}\n", facts.join(" · ")));
    }
    out
}

fn control(registry: &SymbolRegistry, name: &str, verb: ControlVerb) -> String {
    let Some(control) = registry.control(name) else {
        // Say why rather than saying nothing: the diagnostic explains the
        // error, but hovering is how someone checks a name they are unsure of.
        return match registry.state(name) {
            Some(_) => format!(
                "`{name}` is aircraft **state**, which a procedure reads and never moves.\n\n\
                 See E0202.",
            ),
            None => format!("`{name}` is not registered by this aircraft.\n\nSee E0201."),
        };
    };

    let mut out = format!(
        "```fe\n{name}\n```\n\n**control** — {}\n",
        describe(&control.spec)
    );

    let accepted: Vec<&str> = [
        ControlVerb::Check,
        ControlVerb::Set,
        ControlVerb::Start,
        ControlVerb::Stop,
        ControlVerb::Open,
        ControlVerb::Close,
    ]
    .into_iter()
    .filter(|v| v.accepted_by(control.spec.clone()))
    .map(ControlVerb::as_str)
    .collect();
    out.push_str(&format!("\nAccepts: `{}`\n", accepted.join("`, `")));

    if !verb.accepted_by(control.spec.clone()) {
        out.push_str(&format!(
            "\n⚠ `{}` is not one of them — see E0206.\n",
            verb.as_str()
        ));
    }
    out.push_str(&format!("\nHost tag `{}`.\n", control.tag));
    out
}

fn state(registry: &SymbolRegistry, name: &str) -> String {
    let Some(state) = registry.state(name) else {
        return match registry.control(name) {
            Some(control) => format!(
                "`{name}` is a **control** ({}), which a procedure moves and never reads.\n\n\
                 If a procedure needs to know the result, the aircraft should expose that as \
                 state — \"the switch is on\" and \"the pump is running\" are different facts.\n\n\
                 See E0203.",
                describe(&control.spec)
            ),
            None => format!("`{name}` is not registered by this aircraft.\n\nSee E0201."),
        };
    };
    format!(
        "```fe\n{name}\n```\n\n**state** — `{}`, read-only\n\nHost tag `{}`.\n",
        state.ty.as_str(),
        state.tag
    )
}

fn category(name: &str) -> String {
    let meaning = match name {
        "normal" => "Routine operation.",
        "abnormal" => "A malfunction with a published procedure.",
        "emergency" => "Immediate action required.",
        "reference" => "Looked up rather than triggered.",
        _ => return format!("`{name}` is not a category — see E0212."),
    };
    format!("**category {name}**\n\n{meaning}")
}

/// Positions valid for a control, for a quick fix to offer.
pub fn positions(spec: &ControlSpec) -> Vec<String> {
    spec.positions().iter().map(|p| p.to_string()).collect()
}
