//! Every name in a syntax tree, with what kind of name it is.
//!
//! Hover, go-to-definition, references, rename, semantic tokens and inlay hints
//! are all the same question asked at different times: *which name is this, and
//! what role is it playing?* One walk answers it for all of them, so there is
//! one place where "a path in an expression is state, a path after `set` is a
//! control" is written down.

use fe_lang::ast::*;
use fe_lang::span::{Span, UnitId};

/// The verbs that name a control, and what each one means for the control's
/// kind. `docs/symbols.md` is the authority; this is the same table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ControlVerb {
    Check,
    Set,
    Start,
    Stop,
    Open,
    Close,
}

impl ControlVerb {
    pub fn as_str(self) -> &'static str {
        match self {
            ControlVerb::Check => "check",
            ControlVerb::Set => "set",
            ControlVerb::Start => "start",
            ControlVerb::Stop => "stop",
            ControlVerb::Open => "open",
            ControlVerb::Close => "close",
        }
    }

    pub fn from_verb(verb: Verb) -> ControlVerb {
        match verb {
            Verb::Start => ControlVerb::Start,
            Verb::Stop => ControlVerb::Stop,
            Verb::Open => ControlVerb::Open,
            Verb::Close => ControlVerb::Close,
        }
    }

    /// Whether a control of this kind accepts the verb.
    pub fn accepted_by(self, kind: fe_project::ControlSpec) -> bool {
        use fe_project::ControlSpec as Spec;
        match self {
            ControlVerb::Check => true,
            ControlVerb::Set => !matches!(kind, Spec::Checklist),
            ControlVerb::Start | ControlVerb::Stop => matches!(kind, Spec::Switch),
            ControlVerb::Open | ControlVerb::Close => matches!(kind, Spec::Valve),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Role {
    /// `procedure HYD_2_LOW_PRESSURE {`
    ProcedureDecl,
    /// `call HYD_2_ELECTRIC_PUMP_START`
    ProcedureRef,
    /// A name the procedure moves.
    Control { verb: ControlVerb },
    /// A name the procedure reads.
    State,
    /// `set FUEL_XFEED_SELECTOR = TANK_1_TO_3` — the value, and the control it
    /// belongs to, since the valid set depends on it.
    Position { control: String },
    /// `category abnormal`
    Category,
}

impl Role {
    pub fn is_procedure(&self) -> bool {
        matches!(self, Role::ProcedureDecl | Role::ProcedureRef)
    }
}

#[derive(Clone, Debug)]
pub struct Occurrence {
    pub unit: UnitId,
    pub span: Span,
    pub text: String,
    pub role: Role,
    /// The procedure this name appears in. `None` only for a declaration's own
    /// identifier, which is the procedure.
    pub within: Option<String>,
}

/// Every name in `ast`, in source order.
pub fn occurrences(unit: UnitId, ast: &Ast) -> Vec<Occurrence> {
    let mut out = Vec::new();
    for decl in &ast.procedures {
        let within = decl.id.text.clone();
        out.push(Occurrence {
            unit,
            span: decl.id.span,
            text: decl.id.text.clone(),
            role: Role::ProcedureDecl,
            within: None,
        });

        if let Some(category) = &decl.metadata.category {
            out.push(Occurrence {
                unit,
                span: category.span,
                text: category.text.clone(),
                role: Role::Category,
                within: Some(within.clone()),
            });
        }
        if let Some(trigger) = &decl.metadata.trigger {
            expression(unit, &within, trigger, &mut out);
        }
        for require in &decl.metadata.requires {
            expression(unit, &within, &require.condition, &mut out);
        }
        block(unit, &within, &decl.body, &mut out);
    }
    out
}

/// The name at `offset`, preferring the narrowest match.
pub fn at(unit: UnitId, ast: &Ast, offset: u32) -> Option<Occurrence> {
    occurrences(unit, ast)
        .into_iter()
        .filter(|o| o.span.contains(offset))
        .min_by_key(|o| o.span.len())
}

/// The first control named at or after `offset`.
///
/// E0206's span is the *verb*, because the verb is the word that is wrong — the
/// control is fine, it is what is being asked of it that is not. So a fix for it
/// has to look forward from the diagnostic to find the control it is about.
pub fn control_after(unit: UnitId, ast: &Ast, offset: u32) -> Option<Occurrence> {
    occurrences(unit, ast)
        .into_iter()
        .filter(|o| matches!(o.role, Role::Control { .. }) && o.span.start >= offset)
        .min_by_key(|o| o.span.start)
}

/// The procedure whose body encloses `offset`.
pub fn enclosing_procedure(ast: &Ast, offset: u32) -> Option<&ProcedureDecl> {
    ast.procedures.iter().find(|d| d.span.contains(offset))
}

fn block(unit: UnitId, within: &str, block: &Block, out: &mut Vec<Occurrence>) {
    for step in &block.steps {
        match step {
            Step::Check { control, .. } => {
                control_path(unit, within, control, ControlVerb::Check, out)
            }
            Step::Set { control, value, .. } => {
                control_path(unit, within, control, ControlVerb::Set, out);
                if let SetValue::Position(position) = value {
                    out.push(Occurrence {
                        unit,
                        span: position.span,
                        text: position.text.clone(),
                        role: Role::Position {
                            control: control.text.clone(),
                        },
                        within: Some(within.to_string()),
                    });
                }
            }
            Step::Verb { verb, control, .. } => {
                control_path(unit, within, control, ControlVerb::from_verb(*verb), out)
            }
            Step::Call { target, .. } => out.push(Occurrence {
                unit,
                span: target.span,
                text: target.text.clone(),
                role: Role::ProcedureRef,
                within: Some(within.to_string()),
            }),
            Step::Wait { condition, .. } => expression(unit, within, condition, out),
            Step::If(step) => if_step(unit, within, step, out),
            Step::Complete { condition, .. } => {
                if let Some(condition) = condition {
                    expression(unit, within, condition, out);
                }
            }
            Step::Notify { .. } | Step::Fail { .. } => {}
        }
    }
}

fn if_step(unit: UnitId, within: &str, step: &IfStep, out: &mut Vec<Occurrence>) {
    expression(unit, within, &step.condition, out);
    block(unit, within, &step.then_block, out);
    match &step.else_branch {
        Some(ElseBranch::Block(b)) => block(unit, within, b, out),
        Some(ElseBranch::If(nested)) => if_step(unit, within, nested, out),
        None => {}
    }
}

fn control_path(
    unit: UnitId,
    within: &str,
    path: &Path,
    verb: ControlVerb,
    out: &mut Vec<Occurrence>,
) {
    out.push(Occurrence {
        unit,
        span: path.span,
        text: path.text.clone(),
        role: Role::Control { verb },
        within: Some(within.to_string()),
    });
}

/// Every path in a condition is state. A control there is E0203 — the compiler
/// says so, and this is the same rule seen from the other side.
fn expression(unit: UnitId, within: &str, expr: &Expr, out: &mut Vec<Occurrence>) {
    match expr {
        Expr::Symbol(path) => out.push(Occurrence {
            unit,
            span: path.span,
            text: path.text.clone(),
            role: Role::State,
            within: Some(within.to_string()),
        }),
        Expr::Not { operand, .. } => expression(unit, within, operand, out),
        Expr::Binary { lhs, rhs, .. } => {
            expression(unit, within, lhs, out);
            expression(unit, within, rhs, out);
        }
        Expr::Bool(..) | Expr::Number(..) | Expr::Error(_) => {}
    }
}
