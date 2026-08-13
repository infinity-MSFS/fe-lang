use fe_runtime::value::{ControlKind, ValueType};

pub(crate) type StringId = u32;

#[derive(Clone, Debug)]
pub(crate) struct IrModule {
    pub procedures: Vec<IrProcedure>,
    pub symbols: Vec<IrSymbol>,
    pub controls: Vec<IrControl>,
    pub strings: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct IrSymbol {
    pub name: StringId,
    pub ty: ValueType,
    pub tag: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct IrControl {
    pub name: StringId,
    pub kind: ControlKind,
    pub tag: u32,
    pub positions: Vec<StringId>,
}

#[derive(Clone, Debug)]
pub(crate) struct IrProcedure {
    pub span: fe_lang::span::Span,
    pub id: StringId,
    pub name: StringId,
    pub description: Option<StringId>,
    pub category: u8,
    pub priority: u8,
    pub revision: u16,
    pub trigger: Option<IrExpr>,
    pub steps: Vec<IrStep>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum IrCmp {
    Lt,
    Le,
    Gt,
    Ge,
    EqF32,
    NeF32,
    EqBool,
    NeBool,
}

#[derive(Clone, Debug)]
pub(crate) enum IrExpr {
    Bool(bool),
    Number(f32),
    Load {
        symbol: u16,
        ty: ValueType,
    },
    Not(Box<IrExpr>),
    And(Box<IrExpr>, Box<IrExpr>),
    Or(Box<IrExpr>, Box<IrExpr>),
    Compare {
        op: IrCmp,
        lhs: Box<IrExpr>,
        rhs: Box<IrExpr>,
    },
}

impl IrExpr {
    pub fn stack_depth(&self) -> usize {
        match self {
            IrExpr::Bool(_) | IrExpr::Number(_) | IrExpr::Load { .. } => 1,
            IrExpr::Not(inner) => inner.stack_depth(),
            IrExpr::And(lhs, rhs) | IrExpr::Or(lhs, rhs) => {
                lhs.stack_depth().max(1 + rhs.stack_depth())
            }
            IrExpr::Compare { lhs, rhs, .. } => lhs.stack_depth().max(1 + rhs.stack_depth()),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum IrStep {
    SetPosition {
        control: u16,
        position: u8,
    },
    SetAnalog {
        control: u16,
        value: f32,
    },
    Check {
        control: u16,
    },
    Notify {
        message: StringId,
    },
    Call {
        procedure: u16,
    },
    Wait {
        condition: IrExpr,
        timeout_ms: u32,
        fail_on_timeout: bool,
    },
    If {
        condition: IrExpr,
        then_steps: Vec<IrStep>,
        else_steps: Vec<IrStep>,
    },
    Require {
        condition: IrExpr,
        message: Option<StringId>,
    },
    Complete,
    Fail {
        message: Option<StringId>,
    },
}

impl IrStep {
    pub fn terminates(&self) -> bool {
        match self {
            IrStep::Complete | IrStep::Fail { .. } => true,
            IrStep::If {
                then_steps,
                else_steps,
                ..
            } => {
                !else_steps.is_empty()
                    && then_steps.last().map(IrStep::terminates).unwrap_or(false)
                    && else_steps.last().map(IrStep::terminates).unwrap_or(false)
            }
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Interner {
    strings: Vec<String>,
    index: std::collections::BTreeMap<String, StringId>,
}

impl Interner {
    pub fn intern(&mut self, value: &str) -> StringId {
        if let Some(id) = self.index.get(value) {
            return *id;
        }
        let id = self.strings.len() as StringId;
        self.strings.push(value.to_string());
        self.index.insert(value.to_string(), id);
        id
    }

    pub fn into_strings(self) -> Vec<String> {
        self.strings
    }
}
