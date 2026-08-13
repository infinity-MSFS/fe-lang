use fe_runtime::format::{NO_STRING, STACK_CAPACITY, on_timeout, op};

use crate::ir::{IrCmp, IrExpr, IrProcedure, IrStep};

pub(crate) struct CodeBlob {
    pub body: Vec<u8>,
    pub trigger: Vec<u8>,
    pub max_stack: usize,
}

pub(crate) enum CodegenError {
    /// The expression stack would exceed the runtimes fixed depth
    StackTooDeep { needed: usize },
    /// The body is larger than the formats per-procedure limit
    BodyTooLarge { size: usize },
}

const MAX_BODY: usize = u32::MAX as usize / 2;

pub(crate) fn generate(procedure: &IrProcedure) -> Result<CodeBlob, CodegenError> {
    let mut body = Assembler::default();
    body.steps(&procedure.steps);
    body.opcode(op::END);

    let mut trigger = Assembler::default();
    if let Some(condition) = &procedure.trigger {
        trigger.expression(condition);
    }

    let max_stack = body.max_stack.max(trigger.max_stack).max(1);
    if max_stack > STACK_CAPACITY {
        return Err(CodegenError::StackTooDeep { needed: max_stack });
    }
    if body.code.len() > MAX_BODY {
        return Err(CodegenError::BodyTooLarge {
            size: body.code.len(),
        });
    }
    Ok(CodeBlob {
        body: body.code,
        trigger: trigger.code,
        max_stack,
    })
}

#[derive(Default)]
struct Assembler {
    code: Vec<u8>,
    max_stack: usize,
}

impl Assembler {
    fn offset(&self) -> u32 {
        self.code.len() as u32
    }

    fn opcode(&mut self, opcode: u8) {
        self.code.push(opcode);
    }

    fn u8(&mut self, value: u8) {
        self.code.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.code.extend_from_slice(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.code.extend_from_slice(&value.to_le_bytes());
    }

    fn f32(&mut self, value: f32) {
        self.code.extend_from_slice(&value.to_bits().to_le_bytes());
    }

    fn placeholder_u32(&mut self) -> usize {
        let at = self.code.len();
        self.u32(0);
        at
    }

    fn patch_u32(&mut self, at: usize, value: u32) {
        self.code[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn patch_u16(&mut self, at: usize, value: u16) {
        self.code[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn note_stack(&mut self, depth: usize) {
        self.max_stack = self.max_stack.max(depth);
    }

    fn expression(&mut self, expr: &IrExpr) {
        self.note_stack(expr.stack_depth());
        self.expression_inner(expr);
    }

    fn expression_inner(&mut self, expr: &IrExpr) {
        match expr {
            IrExpr::Bool(true) => self.opcode(op::PUSH_TRUE),
            IrExpr::Bool(false) => self.opcode(op::PUSH_FALSE),
            IrExpr::Number(value) => {
                self.opcode(op::PUSH_F32);
                self.f32(*value);
            }
            IrExpr::Load { symbol, ty } => {
                self.opcode(match ty {
                    fe_runtime::value::ValueType::Bool => op::LOAD_BOOL,
                    fe_runtime::value::ValueType::F32 => op::LOAD_F32,
                });
                self.u16(*symbol);
            }
            IrExpr::Not(inner) => {
                self.expression_inner(inner);
                self.opcode(op::NOT);
            }
            IrExpr::And(lhs, rhs) => {
                self.expression_inner(lhs);
                self.expression_inner(rhs);
                self.opcode(op::AND);
            }
            IrExpr::Or(lhs, rhs) => {
                self.expression_inner(lhs);
                self.expression_inner(rhs);
                self.opcode(op::OR);
            }
            IrExpr::Compare { op: cmp, lhs, rhs } => {
                self.expression_inner(lhs);
                self.expression_inner(rhs);
                self.opcode(match cmp {
                    IrCmp::Lt => op::LT,
                    IrCmp::Le => op::LE,
                    IrCmp::Gt => op::GT,
                    IrCmp::Ge => op::GE,
                    IrCmp::EqF32 => op::EQ_F32,
                    IrCmp::NeF32 => op::NE_F32,
                    IrCmp::EqBool => op::EQ_BOOL,
                    IrCmp::NeBool => op::NE_BOOL,
                });
            }
        }
    }

    fn steps(&mut self, steps: &[IrStep]) {
        for step in steps {
            self.step(step);
        }
    }

    fn step(&mut self, step: &IrStep) {
        match step {
            IrStep::SetPosition { control, position } => {
                self.opcode(op::SET_POSITION);
                self.u16(*control);
                self.u8(*position);
            }
            IrStep::SetAnalog { control, value } => {
                self.opcode(op::SET_ANALOG);
                self.u16(*control);
                self.f32(*value);
            }
            IrStep::Check { control } => {
                self.opcode(op::CHECK);
                self.u16(*control);
            }
            IrStep::Notify { message } => {
                self.opcode(op::NOTIFY);
                self.u32(*message);
            }
            IrStep::Call { procedure } => {
                self.opcode(op::CALL);
                self.u16(*procedure);
            }
            IrStep::Require { condition, message } => {
                self.expression(condition);
                self.opcode(op::REQUIRE);
                self.u32(message.unwrap_or(NO_STRING));
            }
            IrStep::Wait {
                condition,
                timeout_ms,
                fail_on_timeout,
            } => {
                self.opcode(op::AWAIT);
                let length_at = self.code.len();
                self.u16(0);
                self.u32(*timeout_ms);
                self.u8(if *fail_on_timeout {
                    on_timeout::FAIL
                } else {
                    on_timeout::CONTINUE
                });
                let body_start = self.offset();
                self.expression(condition);
                let body_len = self.offset() - body_start;
                self.patch_u16(length_at, body_len as u16);
                self.opcode(op::AWAIT_TEST);
            }
            IrStep::If {
                condition,
                then_steps,
                else_steps,
            } => {
                self.expression(condition);
                self.opcode(op::JUMP_IF_FALSE);
                let else_target = self.placeholder_u32();
                self.steps(then_steps);

                if else_steps.is_empty() {
                    let here = self.offset();
                    self.patch_u32(else_target, here);
                } else {
                    let then_falls_through =
                        !then_steps.last().map(IrStep::terminates).unwrap_or(false);
                    let end_target = if then_falls_through {
                        self.opcode(op::JUMP);
                        Some(self.placeholder_u32())
                    } else {
                        None
                    };
                    let here = self.offset();
                    self.patch_u32(else_target, here);
                    self.steps(else_steps);
                    if let Some(end_target) = end_target {
                        let here = self.offset();
                        self.patch_u32(end_target, here);
                    }
                }
            }
            IrStep::Complete => self.opcode(op::COMPLETE),
            IrStep::Fail { message } => {
                self.opcode(op::FAIL);
                self.u32(message.unwrap_or(NO_STRING));
            }
        }
    }
}
