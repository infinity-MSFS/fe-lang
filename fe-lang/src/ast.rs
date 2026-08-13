use crate::span::Span;

#[derive(Clone, Debug)]
pub struct Spanned<T> {
    pub value: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub fn new(value: T, span: Span) -> Spanned<T> {
        Spanned { value, span }
    }
}

#[derive(Clone, Debug)]
pub struct Ident {
    pub text: String,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Path {
    pub text: String,
    pub segments: Vec<Ident>,
    pub span: Span,
}

#[derive(Clone, Debug, Default)]
pub struct Ast {
    pub procedures: Vec<ProcedureDecl>,
}

#[derive(Clone, Debug)]
pub struct ProcedureDecl {
    pub id: Ident,
    pub metadata: Metadata,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug, Default)]
pub struct Metadata {
    pub name: Option<Spanned<String>>,
    pub description: Option<Spanned<String>>,
    pub category: Option<Ident>,
    pub priority: Option<Spanned<f64>>,
    pub revision: Option<Spanned<f64>>,
    pub trigger: Option<Expr>,
    pub requires: Vec<RequireClause>,
}

#[derive(Clone, Debug)]
pub struct RequireClause {
    pub condition: Expr,
    pub message: Option<Spanned<String>>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub steps: Vec<Step>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Step {
    Check {
        control: Path,
        span: Span,
    },
    Set {
        control: Path,
        value: SetValue,
        span: Span,
    },
    Verb {
        verb: Verb,
        verb_span: Span,
        control: Path,
        span: Span,
    },
    Notify {
        message: Spanned<String>,
        span: Span,
    },
    Call {
        target: Ident,
        span: Span,
    },
    Wait {
        condition: Expr,
        timeout: Option<Timeout>,
        span: Span,
    },
    If(IfStep),
    Complete {
        condition: Option<Expr>,
        timeout: Option<Timeout>,
        span: Span,
    },
    Fail {
        message: Option<Spanned<String>>,
        span: Span,
    },
}

impl Step {
    pub fn span(&self) -> Span {
        match self {
            Step::Check { span, .. }
            | Step::Set { span, .. }
            | Step::Verb { span, .. }
            | Step::Notify { span, .. }
            | Step::Call { span, .. }
            | Step::Wait { span, .. }
            | Step::Complete { span, .. }
            | Step::Fail { span, .. } => *span,
            Step::If(step) => step.span,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Timeout {
    pub millis: u32,
    pub fail: bool,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct IfStep {
    pub condition: Expr,
    pub then_block: Block,
    pub else_branch: Option<ElseBranch>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ElseBranch {
    Block(Block),
    If(Box<IfStep>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verb {
    Start,
    Stop,
    Open,
    Close,
}

impl Verb {
    pub fn as_str(self) -> &'static str {
        match self {
            Verb::Start => "start",
            Verb::Stop => "stop",
            Verb::Open => "open",
            Verb::Close => "close",
        }
    }

    pub fn position(self) -> &'static str {
        match self {
            Verb::Start => "ON",
            Verb::Stop => "OFF",
            Verb::Open => "OPEN",
            Verb::Close => "CLOSED",
        }
    }
}

#[derive(Clone, Debug)]
pub enum SetValue {
    Position(Ident),
    Number(Spanned<f64>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinOp {
    And,
    Or,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl BinOp {
    pub fn as_str(self) -> &'static str {
        match self {
            BinOp::And => "&&",
            BinOp::Or => "||",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
        }
    }

    pub fn is_logical(self) -> bool {
        matches!(self, BinOp::And | BinOp::Or)
    }

    pub fn is_ordering(self) -> bool {
        matches!(self, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
    }
}

#[derive(Clone, Debug)]
pub enum Expr {
    Bool(bool, Span),
    Number(f64, Span),
    Symbol(Path),
    Not {
        operand: Box<Expr>,
        span: Span,
    },
    Binary {
        op: BinOp,
        op_span: Span,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    Error(Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Bool(_, span)
            | Expr::Number(_, span)
            | Expr::Not { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Error(span) => *span,
            Expr::Symbol(path) => path.span,
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Expr::Error(_))
    }
}
