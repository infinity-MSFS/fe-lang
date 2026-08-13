use crate::db::{Cmp, Instr, Procedure, ProcedureDatabase, ProcedureIndex, decode};
use crate::format::{self, on_timeout};
use crate::host::{
    Action, ActionResult, AircraftControls, AircraftState, ControlValue, FailReason, ProcedureEvent,
};
use crate::value::{ControlKind, Value};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RuntimeError {
    UnexpectedEnd,
    InvalidOpcode(u8),
    InvalidReference,
    StackOverflow,
    StackUnderflow,
    TypeMismatch,
    CallDepthExceeded,
    NoActiveWait,
    StepLimitExceeded,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExecutionState {
    Ready,
    Running,
    Waiting,
    Completed,
    Failed,
    Cancelled,
}

impl ExecutionState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            ExecutionState::Completed | ExecutionState::Failed | ExecutionState::Cancelled
        )
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tick {
    Running,
    Waiting {
        elapsed_ms: u32,
        timeout_ms: Option<u32>,
    },
    Completed,
    Failed,
    Idle,
}

#[derive(Clone, Copy)]
struct Frame {
    procedure: u16,
    ip: u32,
    wait_ip: u32,
    wait_active: bool,
    wait_elapsed_ms: u32,
}

impl Frame {
    const EMPTY: Frame = Frame {
        procedure: 0,
        ip: 0,
        wait_ip: 0,
        wait_active: false,
        wait_elapsed_ms: 0,
    };
}

pub const DEFAULT_STEP_LIMIT: u32 = 4096;

pub struct ProcedureExecutor<'db> {
    db: ProcedureDatabase<'db>,
    root: ProcedureIndex,
    frames: [Frame; format::MAX_CALL_DEPTH],
    depth: usize,
    stack: [Value; format::STACK_CAPACITY],
    sp: usize,
    state: ExecutionState,
    step_limit: u32,
}

impl<'db> ProcedureExecutor<'db> {
    pub fn new(procedure: Procedure<'db>) -> Self {
        let mut exec = ProcedureExecutor {
            db: procedure.database(),
            root: procedure.index,
            frames: [Frame::EMPTY; format::MAX_CALL_DEPTH],
            depth: 0,
            stack: [Value::Bool(false); format::STACK_CAPACITY],
            sp: 0,
            state: ExecutionState::Ready,
            step_limit: DEFAULT_STEP_LIMIT,
        };
        exec.reset();
        exec
    }

    /// override the per-tick instruction budget
    pub fn with_step_limit(mut self, limit: u32) -> Self {
        self.step_limit = limit.max(1);
        self
    }

    /// rewind to the start
    pub fn reset(&mut self) {
        self.frames = [Frame::EMPTY; format::MAX_CALL_DEPTH];
        self.frames[0] = Frame {
            procedure: self.root.0,
            ..Frame::EMPTY
        };
        self.depth = 0;
        self.sp = 0;
        self.state = ExecutionState::Ready;
    }

    pub fn state(&self) -> ExecutionState {
        self.state
    }

    pub fn is_finished(&self) -> bool {
        self.state.is_terminal()
    }

    /// the procedure this executor was created for
    pub fn procedure(&self) -> Option<Procedure<'db>> {
        self.db.procedure(self.root)
    }

    /// the procedure currently executing
    pub fn current_procedure(&self) -> Option<Procedure<'db>> {
        self.db
            .procedure(ProcedureIndex(self.frames[self.depth].procedure))
    }

    pub fn instruction_pointer(&self) -> u32 {
        self.frames[self.depth].ip
    }

    /// stop the procedure
    pub fn cancel<C: AircraftControls>(&mut self, controls: &mut C) {
        if self.state.is_terminal() {
            return;
        }
        self.state = ExecutionState::Cancelled;
        controls.on_event(ProcedureEvent::Failed {
            reason: FailReason::Cancelled,
        });
    }

    /// evaluate the procedures trigger condition, if it has one
    pub fn evaluate_trigger<S: AircraftState>(
        procedure: &Procedure<'db>,
        state: &S,
    ) -> Result<Option<bool>, RuntimeError> {
        if !procedure.has_trigger() {
            return Ok(None);
        }
        let db = procedure.database();
        let mut stack = [Value::Bool(false); format::STACK_CAPACITY];
        let mut sp = 0usize;
        let code = procedure.trigger_code();
        let mut at = 0usize;
        while at < code.len() {
            let (instr, len) = decode(code, at).map_err(map_decode)?;
            eval_expression(&db, instr, state, &mut stack, &mut sp)?;
            at += len;
        }
        if sp != 1 {
            return Err(RuntimeError::TypeMismatch);
        }
        match stack[0] {
            Value::Bool(b) => Ok(Some(b)),
            Value::F32(_) => Err(RuntimeError::TypeMismatch),
        }
    }

    /// advance the procedure by up to one tick worth of work
    pub fn tick<S: AircraftState, C: AircraftControls>(
        &mut self,
        state: &S,
        controls: &mut C,
        dt_ms: u32,
    ) -> Tick {
        if self.state.is_terminal() {
            return Tick::Idle;
        }
        if self.state == ExecutionState::Ready {
            self.state = ExecutionState::Running;
            if let Some(p) = self.current_procedure() {
                controls.on_event(ProcedureEvent::Started { procedure: p });
            }
        }
        match self.run(state, controls, dt_ms) {
            Ok(tick) => tick,
            Err(err) => {
                self.state = ExecutionState::Failed;
                controls.on_event(ProcedureEvent::Failed {
                    reason: FailReason::Runtime(err),
                });
                Tick::Failed
            }
        }
    }

    fn pop_bool(&mut self) -> Result<bool, RuntimeError> {
        if self.sp == 0 {
            return Err(RuntimeError::StackUnderflow);
        }
        self.sp -= 1;
        self.stack[self.sp]
            .as_bool()
            .ok_or(RuntimeError::TypeMismatch)
    }

    fn run<S: AircraftState, C: AircraftControls>(
        &mut self,
        state: &S,
        controls: &mut C,
        dt_ms: u32,
    ) -> Result<Tick, RuntimeError> {
        let mut budget = self.step_limit;
        loop {
            if budget == 0 {
                return Err(RuntimeError::StepLimitExceeded);
            }
            budget -= 1;

            let frame = self.frames[self.depth];
            let procedure = self
                .db
                .procedure(ProcedureIndex(frame.procedure))
                .ok_or(RuntimeError::InvalidReference)?;
            let code = procedure.body_code();
            let at = frame.ip as usize;
            if at >= code.len() {
                return Err(RuntimeError::UnexpectedEnd);
            }
            let (instr, len) = decode(code, at).map_err(map_decode)?;
            let next = (at + len) as u32;

            if let Some(()) =
                try_eval_expression(&self.db, instr, state, &mut self.stack, &mut self.sp)?
            {
                self.frames[self.depth].ip = next;
                continue;
            }

            match instr {
                Instr::Jump(target) => {
                    self.frames[self.depth].ip = target;
                }
                Instr::JumpIfFalse(target) => {
                    let cond = self.pop_bool()?;
                    self.frames[self.depth].ip = if cond { next } else { target };
                }
                Instr::SetPosition { control, position } => {
                    let c = self
                        .db
                        .control(control)
                        .ok_or(RuntimeError::InvalidReference)?;
                    if c.kind == ControlKind::Checklist {
                        return Err(RuntimeError::InvalidReference);
                    }
                    let name = self
                        .db
                        .position_name(&c, position)
                        .ok_or(RuntimeError::InvalidReference)?;
                    let action = Action::Set {
                        control: c,
                        value: ControlValue::Position {
                            index: position,
                            name,
                        },
                    };
                    self.frames[self.depth].ip = next;
                    if let Some(tick) = self.dispatch(controls, action, c.name) {
                        return Ok(tick);
                    }
                }
                Instr::SetAnalog { control, value } => {
                    let c = self
                        .db
                        .control(control)
                        .ok_or(RuntimeError::InvalidReference)?;
                    let action = Action::Set {
                        control: c,
                        value: ControlValue::Analog(value),
                    };
                    self.frames[self.depth].ip = next;
                    if let Some(tick) = self.dispatch(controls, action, c.name) {
                        return Ok(tick);
                    }
                }
                Instr::Check(control) => {
                    let c = self
                        .db
                        .control(control)
                        .ok_or(RuntimeError::InvalidReference)?;
                    let action = Action::Check { control: c };
                    self.frames[self.depth].ip = next;
                    if let Some(tick) = self.dispatch(controls, action, c.name) {
                        return Ok(tick);
                    }
                }
                Instr::Notify(id) => {
                    let message = self.db.string(id).ok_or(RuntimeError::InvalidReference)?;
                    controls.on_event(ProcedureEvent::Notification { message });
                    self.frames[self.depth].ip = next;
                }
                Instr::Call(index) => {
                    let callee = self
                        .db
                        .procedure(index)
                        .ok_or(RuntimeError::InvalidReference)?;
                    if self.depth + 1 >= format::MAX_CALL_DEPTH {
                        return Err(RuntimeError::CallDepthExceeded);
                    }
                    self.frames[self.depth].ip = next;
                    self.depth += 1;
                    self.frames[self.depth] = Frame {
                        procedure: index.0,
                        ..Frame::EMPTY
                    };
                    controls.on_event(ProcedureEvent::Entered { procedure: callee });
                }
                Instr::Await {
                    body_len,
                    timeout_ms,
                    on_timeout: mode,
                } => {
                    let d = self.depth;
                    let re_entry =
                        self.frames[d].wait_active && self.frames[d].wait_ip == at as u32;
                    if re_entry {
                        self.frames[d].wait_elapsed_ms =
                            self.frames[d].wait_elapsed_ms.saturating_add(dt_ms);
                    } else {
                        self.frames[d].wait_active = true;
                        self.frames[d].wait_ip = at as u32;
                        self.frames[d].wait_elapsed_ms = 0;
                    }
                    let elapsed = self.frames[d].wait_elapsed_ms;
                    if timeout_ms > 0 && elapsed >= timeout_ms {
                        self.frames[d].wait_active = false;
                        self.frames[d].ip = next + body_len as u32 + 1;
                        let failed = mode == on_timeout::FAIL;
                        controls.on_event(ProcedureEvent::Timeout { continued: !failed });
                        if failed {
                            self.state = ExecutionState::Failed;
                            controls.on_event(ProcedureEvent::Failed {
                                reason: FailReason::Timeout,
                            });
                            return Ok(Tick::Failed);
                        }
                    } else {
                        self.frames[d].ip = next;
                    }
                }
                Instr::AwaitTest => {
                    let satisfied = self.pop_bool()?;
                    let d = self.depth;
                    if !self.frames[d].wait_active {
                        return Err(RuntimeError::NoActiveWait);
                    }
                    if satisfied {
                        self.frames[d].wait_active = false;
                        self.frames[d].ip = next;
                        self.state = ExecutionState::Running;
                    } else {
                        let wait_ip = self.frames[d].wait_ip;
                        let elapsed = self.frames[d].wait_elapsed_ms;
                        self.frames[d].ip = wait_ip;
                        let timeout = decode_timeout(code, wait_ip as usize);
                        self.state = ExecutionState::Waiting;
                        controls.on_event(ProcedureEvent::Waiting {
                            elapsed_ms: elapsed,
                            timeout_ms: timeout,
                        });
                        return Ok(Tick::Waiting {
                            elapsed_ms: elapsed,
                            timeout_ms: timeout,
                        });
                    }
                }
                Instr::Require(message) => {
                    let ok = self.pop_bool()?;
                    self.frames[self.depth].ip = next;
                    if !ok {
                        self.state = ExecutionState::Failed;
                        let message = if message == format::NO_STRING {
                            None
                        } else {
                            self.db.string(message)
                        };
                        controls.on_event(ProcedureEvent::Failed {
                            reason: FailReason::Precondition { message },
                        });
                        return Ok(Tick::Failed);
                    }
                }
                Instr::Fail(message) => {
                    self.state = ExecutionState::Failed;
                    let message = if message == format::NO_STRING {
                        None
                    } else {
                        self.db.string(message)
                    };
                    controls.on_event(ProcedureEvent::Failed {
                        reason: FailReason::Explicit { message },
                    });
                    return Ok(Tick::Failed);
                }
                Instr::Complete | Instr::End => {
                    if self.depth == 0 {
                        self.state = ExecutionState::Completed;
                        controls.on_event(ProcedureEvent::Completed);
                        return Ok(Tick::Completed);
                    }
                    self.depth -= 1;
                    controls.on_event(ProcedureEvent::Returned { procedure });
                }
                _ => return Err(RuntimeError::InvalidOpcode(code[at])),
            }
        }
    }

    /// ferform an action and translate a rejection into a failure.
    fn dispatch<C: AircraftControls>(
        &mut self,
        controls: &mut C,
        action: Action<'db>,
        control_name: &'db str,
    ) -> Option<Tick> {
        controls.on_event(ProcedureEvent::ActionRequested { action });
        match controls.execute(action) {
            ActionResult::Accepted => None,
            ActionResult::Rejected(code) => {
                self.state = ExecutionState::Failed;
                controls.on_event(ProcedureEvent::Failed {
                    reason: FailReason::ActionRejected {
                        control: control_name,
                        code,
                    },
                });
                Some(Tick::Failed)
            }
        }
    }
}

fn decode_timeout(code: &[u8], await_at: usize) -> Option<u32> {
    match decode(code, await_at) {
        Ok((Instr::Await { timeout_ms, .. }, _)) if timeout_ms > 0 => Some(timeout_ms),
        _ => None,
    }
}

fn map_decode(err: crate::db::BytecodeError) -> RuntimeError {
    use crate::db::BytecodeError as B;
    match err {
        B::UnknownOpcode(op) => RuntimeError::InvalidOpcode(op),
        B::Truncated => RuntimeError::UnexpectedEnd,
        _ => RuntimeError::InvalidReference,
    }
}

fn try_eval_expression<S: AircraftState>(
    db: &ProcedureDatabase<'_>,
    instr: Instr,
    state: &S,
    stack: &mut [Value; format::STACK_CAPACITY],
    sp: &mut usize,
) -> Result<Option<()>, RuntimeError> {
    match instr {
        Instr::Nop
        | Instr::PushF32(_)
        | Instr::PushBool(_)
        | Instr::LoadF32(_)
        | Instr::LoadBool(_)
        | Instr::Not
        | Instr::And
        | Instr::Or
        | Instr::Compare(_) => {
            eval_expression(db, instr, state, stack, sp)?;
            Ok(Some(()))
        }
        _ => Ok(None),
    }
}

fn eval_expression<S: AircraftState>(
    db: &ProcedureDatabase<'_>,
    instr: Instr,
    state: &S,
    stack: &mut [Value; format::STACK_CAPACITY],
    sp: &mut usize,
) -> Result<(), RuntimeError> {
    match instr {
        Instr::Nop => Ok(()),
        Instr::PushF32(v) => push_value(stack, sp, Value::F32(v)),
        Instr::PushBool(b) => push_value(stack, sp, Value::Bool(b)),
        Instr::LoadF32(id) | Instr::LoadBool(id) => {
            let symbol = db.symbol(id).ok_or(RuntimeError::InvalidReference)?;
            let value = state.read(symbol);
            if value.ty() != symbol.ty {
                return Err(RuntimeError::TypeMismatch);
            }
            push_value(stack, sp, value)
        }
        Instr::Not => {
            let v = pop_bool(stack, sp)?;
            push_value(stack, sp, Value::Bool(!v))
        }
        Instr::And => {
            let b = pop_bool(stack, sp)?;
            let a = pop_bool(stack, sp)?;
            push_value(stack, sp, Value::Bool(a && b))
        }
        Instr::Or => {
            let b = pop_bool(stack, sp)?;
            let a = pop_bool(stack, sp)?;
            push_value(stack, sp, Value::Bool(a || b))
        }
        Instr::Compare(cmp) => {
            let result = match cmp {
                Cmp::EqBool | Cmp::NeBool => {
                    let b = pop_bool(stack, sp)?;
                    let a = pop_bool(stack, sp)?;
                    if cmp == Cmp::EqBool { a == b } else { a != b }
                }
                _ => {
                    let b = pop_f32(stack, sp)?;
                    let a = pop_f32(stack, sp)?;
                    match cmp {
                        Cmp::Lt => a < b,
                        Cmp::Le => a <= b,
                        Cmp::Gt => a > b,
                        Cmp::Ge => a >= b,
                        Cmp::EqF32 => a == b,
                        Cmp::NeF32 => a != b,
                        Cmp::EqBool | Cmp::NeBool => return Err(RuntimeError::TypeMismatch),
                    }
                }
            };
            push_value(stack, sp, Value::Bool(result))
        }
        _ => Err(RuntimeError::TypeMismatch),
    }
}

fn push_value(
    stack: &mut [Value; format::STACK_CAPACITY],
    sp: &mut usize,
    value: Value,
) -> Result<(), RuntimeError> {
    if *sp >= format::STACK_CAPACITY {
        return Err(RuntimeError::StackOverflow);
    }
    stack[*sp] = value;
    *sp += 1;
    Ok(())
}

fn pop_bool(stack: &[Value; format::STACK_CAPACITY], sp: &mut usize) -> Result<bool, RuntimeError> {
    if *sp == 0 {
        return Err(RuntimeError::StackUnderflow);
    }
    *sp -= 1;
    stack[*sp].as_bool().ok_or(RuntimeError::TypeMismatch)
}

fn pop_f32(stack: &[Value; format::STACK_CAPACITY], sp: &mut usize) -> Result<f32, RuntimeError> {
    if *sp == 0 {
        return Err(RuntimeError::StackUnderflow);
    }
    *sp -= 1;
    stack[*sp].as_f32().ok_or(RuntimeError::TypeMismatch)
}
