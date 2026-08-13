use crate::db::{BytecodeError, Instr, Procedure, ProcedureDatabase, decode};
use crate::format::{self, op};
use crate::value::{ControlKind, ValueType};

const MAX_PENDING: usize = 64;

type VerifyResult = Result<(), (u32, BytecodeError)>;

struct Stack {
    types: [ValueType; format::STACK_CAPACITY],
    len: usize,
}

impl Stack {
    fn new() -> Stack {
        Stack {
            types: [ValueType::Bool; format::STACK_CAPACITY],
            len: 0,
        }
    }

    fn push(&mut self, ty: ValueType) -> Result<(), BytecodeError> {
        if self.len >= format::STACK_CAPACITY {
            return Err(BytecodeError::StackDiscipline);
        }
        self.types[self.len] = ty;
        self.len += 1;
        Ok(())
    }

    fn pop(&mut self, expect: ValueType) -> Result<(), BytecodeError> {
        if self.len == 0 {
            return Err(BytecodeError::StackDiscipline);
        }
        self.len -= 1;
        if self.types[self.len] != expect {
            return Err(BytecodeError::StackDiscipline);
        }
        Ok(())
    }
}

fn is_expression_op(opcode: u8) -> bool {
    matches!(
        opcode,
        op::NOP
            | op::PUSH_F32
            | op::PUSH_TRUE
            | op::PUSH_FALSE
            | op::LOAD_F32
            | op::LOAD_BOOL
            | op::NOT
            | op::AND
            | op::OR
            | op::LT
            | op::LE
            | op::GT
            | op::GE
            | op::EQ_F32
            | op::NE_F32
            | op::EQ_BOOL
            | op::NE_BOOL
    )
}

fn step_expression(
    db: &ProcedureDatabase<'_>,
    instr: Instr,
    stack: &mut Stack,
) -> Result<(), BytecodeError> {
    use crate::db::Cmp;
    match instr {
        Instr::Nop => {}
        Instr::PushF32(_) => stack.push(ValueType::F32)?,
        Instr::PushBool(_) => stack.push(ValueType::Bool)?,
        Instr::LoadF32(id) => {
            let s = db.symbol(id).ok_or(BytecodeError::BadOperand)?;
            if s.ty != ValueType::F32 {
                return Err(BytecodeError::StackDiscipline);
            }
            stack.push(ValueType::F32)?;
        }
        Instr::LoadBool(id) => {
            let s = db.symbol(id).ok_or(BytecodeError::BadOperand)?;
            if s.ty != ValueType::Bool {
                return Err(BytecodeError::StackDiscipline);
            }
            stack.push(ValueType::Bool)?;
        }
        Instr::Not => {
            stack.pop(ValueType::Bool)?;
            stack.push(ValueType::Bool)?;
        }
        Instr::And | Instr::Or => {
            stack.pop(ValueType::Bool)?;
            stack.pop(ValueType::Bool)?;
            stack.push(ValueType::Bool)?;
        }
        Instr::Compare(cmp) => {
            let operand = match cmp {
                Cmp::EqBool | Cmp::NeBool => ValueType::Bool,
                _ => ValueType::F32,
            };
            stack.pop(operand)?;
            stack.pop(operand)?;
            stack.push(ValueType::Bool)?;
        }
        _ => return Err(BytecodeError::BadExpression),
    }
    Ok(())
}

pub(crate) fn verify_expression(db: &ProcedureDatabase<'_>, code: &[u8]) -> VerifyResult {
    let mut stack = Stack::new();
    let mut at = 0usize;
    while at < code.len() {
        let (instr, len) = decode(code, at).map_err(|e| (at as u32, e))?;
        if !is_expression_op(code[at]) {
            return Err((at as u32, BytecodeError::BadExpression));
        }
        step_expression(db, instr, &mut stack).map_err(|e| (at as u32, e))?;
        at += len;
    }
    if stack.len != 1 || stack.types[0] != ValueType::Bool {
        return Err((at as u32, BytecodeError::BadExpression));
    }
    Ok(())
}

pub(crate) fn verify_body(db: &ProcedureDatabase<'_>, procedure: &Procedure<'_>) -> VerifyResult {
    let code = procedure.body_code();
    if code.is_empty() {
        return Err((0, BytecodeError::MissingEnd));
    }

    let mut stack = Stack::new();
    let mut pending = [0u32; MAX_PENDING];
    let mut pending_len = 0usize;
    let mut expect_await_test: Option<usize> = None;
    let mut reachable = true;
    let mut at = 0usize;
    let mut last_opcode = op::NOP;

    while at < code.len() {
        let mut is_target = false;
        let mut i = 0;
        while i < pending_len {
            if pending[i] as usize == at {
                pending[i] = pending[pending_len - 1];
                pending_len -= 1;
                is_target = true;
            } else {
                i += 1;
            }
        }
        if is_target {
            if reachable && stack.len != 0 {
                return Err((at as u32, BytecodeError::StackDiscipline));
            }
            stack.len = 0;
            reachable = true;
        } else if !reachable {
            stack.len = 0;
        }

        let opcode = code[at];
        let (instr, len) = decode(code, at).map_err(|e| (at as u32, e))?;
        let next = at + len;

        if expect_await_test.is_some() && opcode != op::AWAIT_TEST && !is_expression_op(opcode) {
            return Err((at as u32, BytecodeError::BadWait));
        }

        if is_expression_op(opcode) {
            step_expression(db, instr, &mut stack).map_err(|e| (at as u32, e))?;
            last_opcode = opcode;
            at = next;
            continue;
        }

        match instr {
            Instr::Jump(target) | Instr::JumpIfFalse(target) => {
                if matches!(instr, Instr::JumpIfFalse(_)) {
                    stack.pop(ValueType::Bool).map_err(|e| (at as u32, e))?;
                }
                if stack.len != 0 {
                    return Err((at as u32, BytecodeError::StackDiscipline));
                }
                let target = target as usize;
                // forward only: this is what makes a tick provably finite.
                if target <= at || target >= code.len() {
                    return Err((at as u32, BytecodeError::BadJumpTarget));
                }
                if pending_len >= MAX_PENDING {
                    return Err((at as u32, BytecodeError::TooComplex));
                }
                pending[pending_len] = target as u32;
                pending_len += 1;
                if matches!(instr, Instr::Jump(_)) {
                    reachable = false;
                }
            }
            Instr::SetPosition { control, position } => {
                let c = db
                    .control(control)
                    .ok_or((at as u32, BytecodeError::BadOperand))?;
                if c.kind == ControlKind::Checklist || position >= c.position_count {
                    return Err((at as u32, BytecodeError::BadOperand));
                }
                if stack.len != 0 {
                    return Err((at as u32, BytecodeError::StackDiscipline));
                }
            }
            Instr::SetAnalog { control, value } => {
                let c = db
                    .control(control)
                    .ok_or((at as u32, BytecodeError::BadOperand))?;
                if c.kind != ControlKind::Analog || !value.is_finite() {
                    return Err((at as u32, BytecodeError::BadOperand));
                }
                if stack.len != 0 {
                    return Err((at as u32, BytecodeError::StackDiscipline));
                }
            }
            Instr::Check(control) => {
                db.control(control)
                    .ok_or((at as u32, BytecodeError::BadOperand))?;
                if stack.len != 0 {
                    return Err((at as u32, BytecodeError::StackDiscipline));
                }
            }
            Instr::Notify(s) => {
                db.string(s).ok_or((at as u32, BytecodeError::BadOperand))?;
                if stack.len != 0 {
                    return Err((at as u32, BytecodeError::StackDiscipline));
                }
            }
            Instr::Call(index) => {
                db.procedure(index)
                    .ok_or((at as u32, BytecodeError::BadOperand))?;
                if stack.len != 0 {
                    return Err((at as u32, BytecodeError::StackDiscipline));
                }
            }
            Instr::Await {
                body_len,
                on_timeout,
                ..
            } => {
                if stack.len != 0 {
                    return Err((at as u32, BytecodeError::StackDiscipline));
                }
                if on_timeout > format::on_timeout::FAIL || expect_await_test.is_some() {
                    return Err((at as u32, BytecodeError::BadWait));
                }
                let test_at = next
                    .checked_add(body_len as usize)
                    .ok_or((at as u32, BytecodeError::BadWait))?;
                if body_len == 0 || test_at >= code.len() {
                    return Err((at as u32, BytecodeError::BadWait));
                }
                expect_await_test = Some(test_at);
            }
            Instr::AwaitTest => {
                match expect_await_test.take() {
                    Some(expected) if expected == at => {}
                    _ => return Err((at as u32, BytecodeError::BadWait)),
                }
                stack.pop(ValueType::Bool).map_err(|e| (at as u32, e))?;
                if stack.len != 0 {
                    return Err((at as u32, BytecodeError::StackDiscipline));
                }
            }
            Instr::Require(s) => {
                stack.pop(ValueType::Bool).map_err(|e| (at as u32, e))?;
                if stack.len != 0 {
                    return Err((at as u32, BytecodeError::StackDiscipline));
                }
                if s != format::NO_STRING {
                    db.string(s).ok_or((at as u32, BytecodeError::BadOperand))?;
                }
            }
            Instr::Fail(s) => {
                if s != format::NO_STRING {
                    db.string(s).ok_or((at as u32, BytecodeError::BadOperand))?;
                }
                if stack.len != 0 {
                    return Err((at as u32, BytecodeError::StackDiscipline));
                }
                reachable = false;
            }
            Instr::Complete | Instr::End => {
                if stack.len != 0 {
                    return Err((at as u32, BytecodeError::StackDiscipline));
                }
                reachable = false;
            }
            _ => return Err((at as u32, BytecodeError::UnknownOpcode(opcode))),
        }

        last_opcode = opcode;
        at = next;
    }

    if pending_len != 0 || expect_await_test.is_some() {
        return Err((at as u32, BytecodeError::BadJumpTarget));
    }
    if last_opcode != op::END {
        return Err((at as u32, BytecodeError::MissingEnd));
    }
    Ok(())
}
