/// Static type of a state symbol or expression.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum ValueType {
    Bool = 0,
    F32 = 1,
}

impl ValueType {
    pub const fn from_u8(v: u8) -> Option<ValueType> {
        match v {
            0 => Some(ValueType::Bool),
            1 => Some(ValueType::F32),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            ValueType::Bool => "bool",
            ValueType::F32 => "number",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Value {
    Bool(bool),
    F32(f32),
}

impl Value {
    pub const fn ty(self) -> ValueType {
        match self {
            Value::Bool(_) => ValueType::Bool,
            Value::F32(_) => ValueType::F32,
        }
    }

    pub const fn as_bool(self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(b),
            Value::F32(_) => None,
        }
    }

    pub const fn as_f32(self) -> Option<f32> {
        match self {
            Value::F32(v) => Some(v),
            Value::Bool(_) => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum ControlKind {
    /// two positions: OFF : ON
    Switch = 0,
    /// two positions: CLOSED : OPEN
    Valve = 1,
    /// typed positions declared by the host
    Selector = 2,
    /// continuous value within a host specified range
    Analog = 3,
    /// not actuable: only check.
    Checklist = 4,
    /// passes it through to the host.
    Unknown = 255,
}

impl ControlKind {
    pub const fn from_u8(v: u8) -> ControlKind {
        match v {
            0 => ControlKind::Switch,
            1 => ControlKind::Valve,
            2 => ControlKind::Selector,
            3 => ControlKind::Analog,
            4 => ControlKind::Checklist,
            _ => ControlKind::Unknown,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            ControlKind::Switch => "switch",
            ControlKind::Valve => "valve",
            ControlKind::Selector => "selector",
            ControlKind::Analog => "analog",
            ControlKind::Checklist => "checklist",
            ControlKind::Unknown => "unknown",
        }
    }
}
