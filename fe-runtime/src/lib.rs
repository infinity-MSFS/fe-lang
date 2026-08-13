#![no_std]
#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]
#![allow(clippy::result_unit_err)]

#[cfg(feature = "std")]
extern crate std;

pub mod db;
pub mod disasm;
pub mod format;
pub mod host;
pub mod interpreter;
pub mod value;
mod verify;

pub use db::{
    BytecodeError, Cmp, Control, ControlId, FormatError, Instr, Procedure, ProcedureDatabase,
    ProcedureIndex, Section, Symbol, SymbolId, decode,
};
pub use disasm::{disassemble, disassemble_code};
pub use format::{Category, FORMAT_VERSION, MAGIC, MAX_CALL_DEPTH, STACK_CAPACITY};
pub use host::{
    Action, ActionResult, AircraftControls, AircraftState, ControlValue, FailReason, ProcedureEvent,
};
pub use interpreter::{DEFAULT_STEP_LIMIT, ExecutionState, ProcedureExecutor, RuntimeError, Tick};
pub use value::{ControlKind, Value, ValueType};
