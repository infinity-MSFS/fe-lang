use crate::db::{Control, Procedure, Symbol};
use crate::interpreter::RuntimeError;
use crate::value::Value;

/// read access to aircraft state
pub trait AircraftState {
    fn read(&self, symbol: Symbol<'_>) -> Value;
}

/// write access to aircraft controls
pub trait AircraftControls {
    fn execute(&mut self, action: Action<'_>) -> ActionResult;

    fn on_event(&mut self, event: ProcedureEvent<'_>) {
        let _ = event;
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ControlValue<'a> {
    Position { index: u8, name: &'a str },
    Analog(f32),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action<'a> {
    Set {
        control: Control<'a>,
        value: ControlValue<'a>,
    },
    Check {
        control: Control<'a>,
    },
}

impl<'a> Action<'a> {
    pub fn control(&self) -> Control<'a> {
        match self {
            Action::Set { control, .. } | Action::Check { control } => *control,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActionResult {
    Accepted,
    Rejected(u16),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FailReason<'a> {
    Precondition { message: Option<&'a str> },
    Explicit { message: Option<&'a str> },
    ActionRejected { control: &'a str, code: u16 },
    Timeout,
    Cancelled,
    Runtime(RuntimeError),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ProcedureEvent<'a> {
    Started {
        procedure: Procedure<'a>,
    },
    ActionRequested {
        action: Action<'a>,
    },
    Notification {
        message: &'a str,
    },
    Waiting {
        elapsed_ms: u32,
        timeout_ms: Option<u32>,
    },
    Timeout {
        continued: bool,
    },
    Entered {
        procedure: Procedure<'a>,
    },
    Returned {
        procedure: Procedure<'a>,
    },
    Completed,
    Failed {
        reason: FailReason<'a>,
    },
}
