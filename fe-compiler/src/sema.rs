use std::collections::BTreeMap;

use fe_lang::ast::*;
use fe_lang::diagnostics::{Diagnostic, Diagnostics, Label, codes};
use fe_lang::span::Span;
use fe_runtime::format::{Category, MAX_CALL_DEPTH};
use fe_runtime::value::ValueType;

use crate::ir::*;
use crate::symbols::{ControlSpec, Resolved, SymbolRegistry, edit_distance};

const MAX_NESTING: usize = 16;

pub(crate) fn analyze(
    parsed: &[(fe_lang::span::UnitId, Ast)],
    registry: &SymbolRegistry,
    diagnostics: &mut Diagnostics,
) -> Option<IrModule> {
    let mut declarations: Vec<&ProcedureDecl> = Vec::new();
    let mut seen: BTreeMap<&str, Span> = BTreeMap::new();

    for (_, ast) in parsed {
        for decl in &ast.procedures {
            if let Some(previous) = seen.get(decl.id.text.as_str()) {
                diagnostics.push(
                    Diagnostic::error(
                        codes::DUPLICATE_PROCEDURE,
                        format!("procedure `{}` is defined more than once", decl.id.text),
                        Label::new(decl.id.span, "duplicate definition"),
                    )
                    .with_secondary(Label::new(*previous, "first defined here"))
                    .with_note("procedure identifiers share one namespace across all sources"),
                );
                continue;
            }
            seen.insert(decl.id.text.as_str(), decl.id.span);
            declarations.push(decl);
        }
    }

    declarations.sort_by(|a, b| a.id.text.cmp(&b.id.text));

    let mut procedure_index = BTreeMap::new();
    for (index, decl) in declarations.iter().enumerate() {
        procedure_index.insert(decl.id.text.clone(), index as u16);
    }

    let mut analyzer = Analyzer {
        registry,
        diagnostics,
        interner: Interner::default(),
        symbols: Vec::new(),
        symbol_index: BTreeMap::new(),
        controls: Vec::new(),
        control_index: BTreeMap::new(),
        procedure_index,
        edges: vec![Vec::new(); declarations.len()],
    };

    let mut procedures = Vec::with_capacity(declarations.len());
    for (index, decl) in declarations.iter().enumerate() {
        procedures.push(analyzer.procedure(index as u16, decl));
    }
    analyzer.check_call_graph(&declarations);

    let module = IrModule {
        procedures,
        symbols: analyzer.symbols,
        controls: analyzer.controls,
        strings: analyzer.interner.into_strings(),
    };

    if diagnostics.has_errors() {
        None
    } else {
        Some(module)
    }
}

struct Analyzer<'a> {
    registry: &'a SymbolRegistry,
    diagnostics: &'a mut Diagnostics,
    interner: Interner,
    symbols: Vec<IrSymbol>,
    symbol_index: BTreeMap<String, u16>,
    controls: Vec<IrControl>,
    control_index: BTreeMap<String, u16>,
    procedure_index: BTreeMap<String, u16>,
    edges: Vec<Vec<(u16, Span)>>,
}

impl<'a> Analyzer<'a> {
    fn error(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    fn intern_state(&mut self, name: &str, ty: ValueType, tag: u32) -> u16 {
        if let Some(id) = self.symbol_index.get(name) {
            return *id;
        }
        let id = self.symbols.len() as u16;
        let name_id = self.interner.intern(name);
        self.symbols.push(IrSymbol {
            name: name_id,
            ty,
            tag,
        });
        self.symbol_index.insert(name.to_string(), id);
        id
    }

    fn intern_control(&mut self, name: &str, spec: &ControlSpec, tag: u32) -> u16 {
        if let Some(id) = self.control_index.get(name) {
            return *id;
        }
        let id = self.controls.len() as u16;
        let name_id = self.interner.intern(name);
        let positions = spec
            .positions()
            .iter()
            .map(|p| self.interner.intern(p))
            .collect();
        self.controls.push(IrControl {
            name: name_id,
            kind: spec.kind(),
            tag,
            positions,
        });
        self.control_index.insert(name.to_string(), id);
        id
    }

    fn procedure(&mut self, index: u16, decl: &ProcedureDecl) -> IrProcedure {
        let id = self.interner.intern(&decl.id.text);

        let name = match &decl.metadata.name {
            Some(name) => self.interner.intern(&name.value),
            None => {
                self.error(
                    Diagnostic::error(
                        codes::MISSING_METADATA,
                        format!("procedure `{}` has no `name`", decl.id.text),
                        Label::new(decl.id.span, "missing crew-facing title"),
                    )
                    .with_help("add `name \"...\"` as the first line of the procedure")
                    .with_note("the title is what appears to the crew; the identifier is not"),
                );
                self.interner.intern(&decl.id.text)
            }
        };

        let description = decl
            .metadata
            .description
            .as_ref()
            .map(|d| self.interner.intern(&d.value));

        let category = match &decl.metadata.category {
            Some(ident) => match category_from_str(&ident.text) {
                Some(category) => category as u8,
                None => {
                    self.error(
                        Diagnostic::error(
                            codes::INVALID_METADATA_VALUE,
                            format!("`{}` is not a known category", ident.text),
                            Label::bare(ident.span),
                        )
                        .with_help("expected one of: normal, abnormal, emergency, reference"),
                    );
                    Category::Normal as u8
                }
            },
            None => {
                self.error(
                    Diagnostic::error(
                        codes::MISSING_METADATA,
                        format!("procedure `{}` has no `category`", decl.id.text),
                        Label::new(decl.id.span, "missing category"),
                    )
                    .with_help("add `category normal`, `abnormal`, `emergency` or `reference`"),
                );
                Category::Normal as u8
            }
        };

        let priority =
            self.integer_metadata(decl.metadata.priority.as_ref(), 0, 255, "priority") as u8;
        let revision =
            self.integer_metadata(decl.metadata.revision.as_ref(), 0, 65535, "revision") as u16;

        let trigger = decl
            .metadata
            .trigger
            .as_ref()
            .and_then(|expr| self.condition(expr, "trigger"));

        let mut steps = Vec::new();
        for clause in &decl.metadata.requires {
            if let Some(condition) = self.condition(&clause.condition, "require") {
                let message = clause
                    .message
                    .as_ref()
                    .map(|m| self.interner.intern(&m.value));
                steps.push(IrStep::Require { condition, message });
            }
        }

        let mut context = Context { index, depth: 0 };
        let body = self.block(&decl.body.steps, &mut context);
        steps.extend(body);

        if decl.body.steps.is_empty() && decl.metadata.requires.is_empty() {
            self.error(
                Diagnostic::error(
                    codes::EMPTY_PROCEDURE,
                    format!("procedure `{}` has no steps", decl.id.text),
                    Label::new(decl.id.span, "nothing for the crew to do"),
                )
                .with_help("add at least one step, or delete the procedure"),
            );
        }

        IrProcedure {
            span: decl.id.span,
            id,
            name,
            description,
            category,
            priority,
            revision,
            trigger,
            steps,
        }
    }

    fn integer_metadata(
        &mut self,
        value: Option<&Spanned<f64>>,
        min: i64,
        max: i64,
        what: &str,
    ) -> i64 {
        let Some(value) = value else { return 0 };
        if value.value.fract() != 0.0 {
            self.error(Diagnostic::error(
                codes::INVALID_METADATA_VALUE,
                format!("`{what}` must be a whole number"),
                Label::bare(value.span),
            ));
            return 0;
        }
        let integer = value.value as i64;
        if integer < min || integer > max {
            self.error(
                Diagnostic::error(
                    codes::INVALID_METADATA_VALUE,
                    format!("`{what}` must be between {min} and {max}"),
                    Label::new(value.span, format!("found {integer}")),
                )
                .with_note("the field is stored in a fixed-width integer in the binary"),
            );
            return min;
        }
        integer
    }

    fn block(&mut self, steps: &[Step], context: &mut Context) -> Vec<IrStep> {
        let mut lowered: Vec<IrStep> = Vec::new();
        let mut terminated_at: Option<Span> = None;
        for step in steps {
            if let Some(previous) = terminated_at {
                self.diagnostics.push(
                    Diagnostic::warning(
                        codes::UNREACHABLE_STEP,
                        "this step can never run",
                        Label::new(step.span(), "unreachable"),
                    )
                    .with_secondary(Label::new(previous, "the procedure already ended here"))
                    .with_note("the step is omitted from the compiled procedure"),
                );
                continue;
            }
            let span = step.span();
            let expansion = self.step(step, context);
            if expansion.last().map(IrStep::terminates).unwrap_or(false) {
                terminated_at = Some(span);
            }
            lowered.extend(expansion);
        }
        lowered
    }

    fn step(&mut self, step: &Step, context: &mut Context) -> Vec<IrStep> {
        if let Step::Complete {
            condition: Some(condition),
            timeout,
            ..
        } = step
        {
            let Some(condition) = self.condition(condition, "complete when") else {
                return Vec::new();
            };
            let (timeout_ms, _) = self.timeout(timeout.as_ref(), true);
            return vec![
                IrStep::Wait {
                    condition,
                    timeout_ms,
                    fail_on_timeout: true,
                },
                IrStep::Complete,
            ];
        }
        self.single_step(step, context).into_iter().collect()
    }

    fn single_step(&mut self, step: &Step, context: &mut Context) -> Option<IrStep> {
        match step {
            Step::Check { control, .. } => {
                let (id, _) = self.resolve_control(control)?;
                Some(IrStep::Check { control: id })
            }
            Step::Set {
                control,
                value,
                span,
            } => self.set_step(control, value, *span),
            Step::Verb {
                verb,
                verb_span,
                control,
                ..
            } => {
                let (id, spec) = self.resolve_control(control)?;
                match spec.position_index(verb.position()) {
                    Some(position) => Some(IrStep::SetPosition {
                        control: id,
                        position,
                    }),
                    None => {
                        let positions = spec.positions().join(", ");
                        self.error(
                            Diagnostic::error(
                                codes::INVALID_ACTION_FOR_CONTROL,
                                format!(
                                    "`{}` cannot be applied to `{}`",
                                    verb.as_str(),
                                    control.text
                                ),
                                Label::new(
                                    *verb_span,
                                    format!(
                                        "`{}` needs a `{}` position",
                                        verb.as_str(),
                                        verb.position()
                                    ),
                                ),
                            )
                            .with_note(if positions.is_empty() {
                                format!("`{}` is a {} control", control.text, spec.kind().as_str())
                            } else {
                                format!("`{}` accepts: {positions}", control.text)
                            }),
                        );
                        None
                    }
                }
            }
            Step::Notify { message, .. } => {
                let id = self.interner.intern(&message.value);
                Some(IrStep::Notify { message: id })
            }
            Step::Call { target, span } => {
                let Some(index) = self.procedure_index.get(&target.text).copied() else {
                    let suggestion = self.suggest_procedure(&target.text);
                    let mut diagnostic = Diagnostic::error(
                        codes::UNKNOWN_PROCEDURE,
                        format!("unknown procedure `{}`", target.text),
                        Label::new(target.span, "not defined in any source"),
                    );
                    if let Some(suggestion) = suggestion {
                        diagnostic = diagnostic.with_help(format!("did you mean `{suggestion}`?"));
                    }
                    self.error(diagnostic);
                    return None;
                };
                self.edges[context.index as usize].push((index, *span));
                Some(IrStep::Call { procedure: index })
            }
            Step::Wait {
                condition, timeout, ..
            } => {
                let condition = self.condition(condition, "wait")?;
                let (timeout_ms, fail_on_timeout) = self.timeout(timeout.as_ref(), false);
                Some(IrStep::Wait {
                    condition,
                    timeout_ms,
                    fail_on_timeout,
                })
            }
            Step::If(step) => self.if_step(step, context),
            Step::Complete { timeout, .. } => {
                if let Some(timeout) = timeout {
                    self.error(Diagnostic::error(
                        codes::INVALID_TIMEOUT,
                        "`timeout` needs a `when` condition",
                        Label::bare(timeout.span),
                    ));
                }
                Some(IrStep::Complete)
            }
            Step::Fail { message, .. } => {
                let message = message.as_ref().map(|m| self.interner.intern(&m.value));
                Some(IrStep::Fail { message })
            }
        }
    }

    fn if_step(&mut self, step: &IfStep, context: &mut Context) -> Option<IrStep> {
        if context.depth >= MAX_NESTING {
            self.error(
                Diagnostic::error(
                    codes::NESTING_TOO_DEEP,
                    format!("conditions are nested more than {MAX_NESTING} deep"),
                    Label::bare(step.span),
                )
                .with_help("split the procedure and use `call`"),
            );
            return None;
        }
        let condition = self.condition(&step.condition, "if")?;
        context.depth += 1;
        let then_steps = self.block(&step.then_block.steps, context);
        let else_steps = match &step.else_branch {
            None => Vec::new(),
            Some(ElseBranch::Block(block)) => self.block(&block.steps, context),
            Some(ElseBranch::If(nested)) => self
                .if_step(nested, context)
                .map(|step| vec![step])
                .unwrap_or_default(),
        };
        context.depth -= 1;

        if then_steps.is_empty() && else_steps.is_empty() {
            self.diagnostics.push(Diagnostic::warning(
                codes::EMPTY_BRANCH,
                "this `if` has no steps",
                Label::bare(step.span),
            ));
            return None;
        }

        Some(IrStep::If {
            condition,
            then_steps,
            else_steps,
        })
    }

    fn set_step(&mut self, control: &Path, value: &SetValue, span: Span) -> Option<IrStep> {
        let (id, spec) = self.resolve_control(control)?;
        match (&spec, value) {
            (ControlSpec::Checklist, _) => {
                self.error(
                    Diagnostic::error(
                        codes::INVALID_ACTION_FOR_CONTROL,
                        format!("`{}` cannot be set", control.text),
                        Label::new(control.span, "registered as a checklist item"),
                    )
                    .with_help("use `check` instead"),
                );
                None
            }
            (ControlSpec::Analog { min, max }, SetValue::Number(number)) => {
                let value = number.value as f32;
                if !value.is_finite() || value < *min || value > *max {
                    self.error(Diagnostic::error(
                        codes::VALUE_OUT_OF_RANGE,
                        format!("`{}` accepts values from {min} to {max}", control.text),
                        Label::new(number.span, format!("found {}", number.value)),
                    ));
                    return None;
                }
                Some(IrStep::SetAnalog { control: id, value })
            }
            (ControlSpec::Analog { min, max }, SetValue::Position(position)) => {
                self.error(
                    Diagnostic::error(
                        codes::INVALID_CONTROL_VALUE,
                        format!("`{}` is an analog control", control.text),
                        Label::new(position.span, "expected a number"),
                    )
                    .with_help(format!("give a value between {min} and {max}")),
                );
                None
            }
            (spec, SetValue::Position(position)) => match spec.position_index(&position.text) {
                Some(index) => Some(IrStep::SetPosition {
                    control: id,
                    position: index,
                }),
                None => {
                    let positions = spec.positions();
                    let suggestion = closest(&position.text, positions.iter().copied());
                    let mut diagnostic = Diagnostic::error(
                        codes::INVALID_CONTROL_VALUE,
                        format!(
                            "`{}` is not a position of `{}`",
                            position.text, control.text
                        ),
                        Label::bare(position.span),
                    )
                    .with_note(format!("accepted positions: {}", positions.join(", ")));
                    if let Some(suggestion) = suggestion {
                        diagnostic = diagnostic.with_help(format!("did you mean `{suggestion}`?"));
                    }
                    self.error(diagnostic);
                    None
                }
            },
            (spec, SetValue::Number(number)) => {
                self.error(
                    Diagnostic::error(
                        codes::INVALID_CONTROL_VALUE,
                        format!("`{}` is a {} control", control.text, spec.kind().as_str()),
                        Label::new(number.span, "expected a named position, not a number"),
                    )
                    .with_note(format!(
                        "accepted positions: {}",
                        spec.positions().join(", ")
                    )),
                );
                let _ = span;
                None
            }
        }
    }

    fn timeout(&mut self, timeout: Option<&Timeout>, force_fail: bool) -> (u32, bool) {
        let Some(timeout) = timeout else {
            return (0, false);
        };
        if timeout.millis == 0 {
            self.error(
                Diagnostic::error(
                    codes::INVALID_TIMEOUT,
                    "a timeout must be longer than zero",
                    Label::bare(timeout.span),
                )
                .with_note("omit `timeout` to wait indefinitely"),
            );
            return (0, false);
        }
        (timeout.millis, force_fail || timeout.fail)
    }

    fn resolve_control(&mut self, path: &Path) -> Option<(u16, ControlSpec)> {
        match self.registry.resolve(&path.text) {
            Some(Resolved::Control(control)) => {
                let spec = control.spec.clone();
                let tag = control.tag;
                let name = control.name.clone();
                Some((self.intern_control(&name, &spec, tag), spec))
            }
            Some(Resolved::State(_)) => {
                self.error(
                    Diagnostic::error(
                        codes::NOT_A_CONTROL,
                        format!("`{}` is aircraft state, not a control", path.text),
                        Label::new(path.span, "read-only"),
                    )
                    .with_note(
                        "procedures may only actuate registered controls; the simulation decides \
                         what happens to state",
                    ),
                );
                None
            }
            None => {
                self.unknown_symbol(path);
                None
            }
        }
    }

    fn unknown_symbol(&mut self, path: &Path) {
        let suggestion = self.registry.suggest(&path.text).map(str::to_string);
        let mut diagnostic = Diagnostic::error(
            codes::UNKNOWN_SYMBOL,
            "unknown aircraft symbol",
            Label::new(path.span, format!("`{}` is not registered", path.text)),
        );
        if let Some(suggestion) = suggestion {
            diagnostic = diagnostic.with_help(format!("did you mean `{suggestion}`?"));
        } else {
            diagnostic = diagnostic.with_note(
                "the aircraft registers the symbols procedures may use; add it there first",
            );
        }
        self.error(diagnostic);
    }

    fn suggest_procedure(&self, name: &str) -> Option<String> {
        closest(name, self.procedure_index.keys().map(String::as_str)).map(str::to_string)
    }

    fn condition(&mut self, expr: &Expr, context: &str) -> Option<IrExpr> {
        let (ir, ty) = self.expression(expr)?;
        if ty != ValueType::Bool {
            self.error(
                Diagnostic::error(
                    codes::TYPE_MISMATCH,
                    format!("`{context}` needs a yes/no condition"),
                    Label::new(expr.span(), "this is a number"),
                )
                .with_help("compare it, for example `... > 2500`"),
            );
            return None;
        }
        if ir.stack_depth() > fe_runtime::STACK_CAPACITY {
            self.error(
                Diagnostic::error(
                    codes::PROCEDURE_TOO_COMPLEX,
                    "condition is too deeply nested".to_string(),
                    Label::new(expr.span(), "needs more operand slots than the runtime has"),
                )
                .with_note(format!(
                    "the runtime evaluates conditions on a {}-slot stack",
                    fe_runtime::STACK_CAPACITY
                ))
                .with_help("split the condition across nested `if` steps"),
            );
            return None;
        }
        if matches!(ir, IrExpr::Bool(_)) {
            self.diagnostics.push(
                Diagnostic::warning(
                    codes::CONSTANT_CONDITION,
                    format!("`{context}` condition is always the same"),
                    Label::bare(expr.span()),
                )
                .with_note("this condition does not read any aircraft state"),
            );
        }
        Some(ir)
    }

    fn expression(&mut self, expr: &Expr) -> Option<(IrExpr, ValueType)> {
        match expr {
            Expr::Error(_) => None,
            Expr::Bool(value, _) => Some((IrExpr::Bool(*value), ValueType::Bool)),
            Expr::Number(value, span) => {
                let narrowed = *value as f32;
                if !narrowed.is_finite() {
                    self.error(Diagnostic::error(
                        codes::VALUE_OUT_OF_RANGE,
                        "number is too large to represent",
                        Label::bare(*span),
                    ));
                    return None;
                }
                Some((IrExpr::Number(narrowed), ValueType::F32))
            }
            Expr::Symbol(path) => match self.registry.resolve(&path.text) {
                Some(Resolved::State(state)) => {
                    let ty = state.ty;
                    let tag = state.tag;
                    let name = state.name.clone();
                    let id = self.intern_state(&name, ty, tag);
                    Some((IrExpr::Load { symbol: id, ty }, ty))
                }
                Some(Resolved::Control(_)) => {
                    self.error(
                        Diagnostic::error(
                            codes::NOT_READABLE,
                            format!("`{}` is a control and cannot be read", path.text),
                            Label::bare(path.span),
                        )
                        .with_note(
                            "a control is a request to the aircraft, not a measurement; register \
                             a state symbol for its actual position if a procedure needs to test it",
                        ),
                    );
                    None
                }
                None => {
                    self.unknown_symbol(path);
                    None
                }
            },
            Expr::Not { operand, span } => {
                let (inner, ty) = self.expression(operand)?;
                if ty != ValueType::Bool {
                    self.error(Diagnostic::error(
                        codes::TYPE_MISMATCH,
                        "`!` needs a yes/no value",
                        Label::new(*span, "this is a number"),
                    ));
                    return None;
                }
                Some((IrExpr::Not(Box::new(inner)), ValueType::Bool))
            }
            Expr::Binary {
                op,
                op_span,
                lhs,
                rhs,
                ..
            } => {
                let lhs = self.expression(lhs);
                let rhs = self.expression(rhs);
                let ((lhs, lty), (rhs, rty)) = (lhs?, rhs?);
                if op.is_logical() {
                    if lty != ValueType::Bool || rty != ValueType::Bool {
                        self.error(Diagnostic::error(
                            codes::TYPE_MISMATCH,
                            format!("`{}` combines yes/no conditions", op.as_str()),
                            Label::new(
                                *op_span,
                                format!("found {} {} {}", lty.as_str(), op.as_str(), rty.as_str()),
                            ),
                        ));
                        return None;
                    }
                    let ir = if *op == BinOp::And {
                        IrExpr::And(Box::new(lhs), Box::new(rhs))
                    } else {
                        IrExpr::Or(Box::new(lhs), Box::new(rhs))
                    };
                    return Some((ir, ValueType::Bool));
                }

                if lty != rty {
                    self.error(
                        Diagnostic::error(
                            codes::TYPE_MISMATCH,
                            "cannot compare a number with a yes/no value",
                            Label::new(
                                *op_span,
                                format!("{} {} {}", lty.as_str(), op.as_str(), rty.as_str()),
                            ),
                        )
                        .with_note("both sides of a comparison must have the same type"),
                    );
                    return None;
                }
                if op.is_ordering() && lty != ValueType::F32 {
                    self.error(Diagnostic::error(
                        codes::TYPE_MISMATCH,
                        format!("`{}` only orders numbers", op.as_str()),
                        Label::bare(*op_span),
                    ));
                    return None;
                }
                let cmp = match (op, lty) {
                    (BinOp::Lt, _) => IrCmp::Lt,
                    (BinOp::Le, _) => IrCmp::Le,
                    (BinOp::Gt, _) => IrCmp::Gt,
                    (BinOp::Ge, _) => IrCmp::Ge,
                    (BinOp::Eq, ValueType::F32) => IrCmp::EqF32,
                    (BinOp::Ne, ValueType::F32) => IrCmp::NeF32,
                    (BinOp::Eq, ValueType::Bool) => IrCmp::EqBool,
                    (BinOp::Ne, ValueType::Bool) => IrCmp::NeBool,
                    _ => IrCmp::EqBool,
                };
                if matches!(cmp, IrCmp::EqF32 | IrCmp::NeF32) {
                    self.diagnostics.push(
                        Diagnostic::warning(
                            codes::FLOAT_EQUALITY,
                            "comparing measured values for exact equality",
                            Label::bare(*op_span),
                        )
                        .with_help("prefer a threshold, for example `> 2500`")
                        .with_note("simulated sensor values rarely land on an exact figure"),
                    );
                }
                Some((
                    IrExpr::Compare {
                        op: cmp,
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                    },
                    ValueType::Bool,
                ))
            }
        }
    }

    fn check_call_graph(&mut self, declarations: &[&ProcedureDecl]) {
        let count = self.edges.len();
        // 0 = unvisited, 1 = on stack, 2 = done
        let mut state = vec![0u8; count];
        let mut stack: Vec<usize> = Vec::new();
        let mut acyclic = true;

        for start in 0..count {
            if state[start] != 0 {
                continue;
            }
            let mut work: Vec<(usize, usize)> = vec![(start, 0)];
            state[start] = 1;
            stack.push(start);
            while let Some((node, cursor)) = work.pop() {
                if cursor < self.edges[node].len() {
                    work.push((node, cursor + 1));
                    let (target, span) = self.edges[node][cursor];
                    let target = target as usize;
                    match state.get(target).copied().unwrap_or(2) {
                        0 => {
                            state[target] = 1;
                            stack.push(target);
                            work.push((target, 0));
                        }
                        1 => {
                            acyclic = false;
                            let name = &declarations[target].id.text;
                            let via = &declarations[node].id.text;
                            self.diagnostics.push(
                                Diagnostic::error(
                                    codes::RECURSIVE_CALL,
                                    format!("`{via}` calls `{name}`, which leads back to `{via}`"),
                                    Label::new(span, "recursive call"),
                                )
                                .with_note(
                                    "procedures may not recurse: execution must be provably finite",
                                ),
                            );
                        }
                        _ => {}
                    }
                } else {
                    state[node] = 2;
                    stack.pop();
                }
            }
        }

        if !acyclic {
            return;
        }

        let mut depth = vec![usize::MAX; count];
        for index in 0..count {
            let d = self.call_depth(index, &mut depth);
            if d >= MAX_CALL_DEPTH {
                self.diagnostics.push(
                    Diagnostic::error(
                        codes::CALL_DEPTH_EXCEEDED,
                        format!(
                            "`{}` can nest calls {} deep; the runtime allows {}",
                            declarations[index].id.text,
                            d + 1,
                            MAX_CALL_DEPTH
                        ),
                        Label::bare(declarations[index].id.span),
                    )
                    .with_help("flatten the procedure or inline a subprocedure"),
                );
            }
        }
    }

    fn call_depth(&self, index: usize, memo: &mut Vec<usize>) -> usize {
        if memo[index] != usize::MAX {
            return memo[index];
        }
        memo[index] = 0;
        let mut best = 0;
        for cursor in 0..self.edges[index].len() {
            let (target, _) = self.edges[index][cursor];
            let depth = 1 + self.call_depth(target as usize, memo);
            best = best.max(depth);
        }
        memo[index] = best;
        best
    }
}

struct Context {
    index: u16,
    depth: usize,
}

fn category_from_str(text: &str) -> Option<Category> {
    Some(match text {
        "normal" => Category::Normal,
        "abnormal" => Category::Abnormal,
        "emergency" => Category::Emergency,
        "reference" => Category::Reference,
        _ => return None,
    })
}

fn closest<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let budget = (name.len() / 3).max(2);
    let mut best: Option<(usize, &str)> = None;
    for candidate in candidates {
        let distance = edit_distance(name, candidate);
        if distance <= budget && best.map(|(d, _)| distance < d).unwrap_or(true) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, name)| name)
}
