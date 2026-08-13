use core::fmt::{self, Write};

use crate::db::{Cmp, Instr, Procedure, ProcedureDatabase, ProcedureIndex, decode};
use crate::format::{self, on_timeout};

pub fn disassemble(procedure: &Procedure<'_>, out: &mut dyn Write) -> fmt::Result {
    let db = procedure.database();
    writeln!(
        out,
        "procedure {} ; \"{}\" [{} priority={} rev={}]",
        procedure.id,
        procedure.name,
        procedure.category.as_str(),
        procedure.priority,
        procedure.revision
    )?;
    if procedure.has_trigger() {
        writeln!(out, ".trigger")?;
        disassemble_code(&db, procedure.trigger_code(), out)?;
    }
    writeln!(out, ".body")?;
    disassemble_code(&db, procedure.body_code(), out)
}

pub fn disassemble_code(
    db: &ProcedureDatabase<'_>,
    code: &[u8],
    out: &mut dyn Write,
) -> fmt::Result {
    let mut at = 0usize;
    while at < code.len() {
        let (instr, len) = match decode(code, at) {
            Ok(v) => v,
            Err(err) => {
                writeln!(out, "{at:04}  <invalid: {err:?}>")?;
                return Ok(());
            }
        };
        write!(out, "{at:04}  ")?;
        write_instruction(db, instr, out)?;
        writeln!(out)?;
        at += len;
    }
    Ok(())
}

fn write_instruction(db: &ProcedureDatabase<'_>, instr: Instr, out: &mut dyn Write) -> fmt::Result {
    match instr {
        Instr::Nop => write!(out, "NOP"),
        Instr::PushF32(v) => write!(out, "PUSH_F32       {v}"),
        Instr::PushBool(b) => write!(out, "PUSH_{}", if b { "TRUE" } else { "FALSE" }),
        Instr::LoadF32(id) => write!(
            out,
            "LOAD_F32       #{} ; {}",
            id.0,
            db.symbol(id).map(|s| s.name).unwrap_or("?")
        ),
        Instr::LoadBool(id) => write!(
            out,
            "LOAD_BOOL      #{} ; {}",
            id.0,
            db.symbol(id).map(|s| s.name).unwrap_or("?")
        ),
        Instr::Not => write!(out, "NOT"),
        Instr::And => write!(out, "AND"),
        Instr::Or => write!(out, "OR"),
        Instr::Compare(cmp) => write!(
            out,
            "{}",
            match cmp {
                Cmp::Lt => "LT",
                Cmp::Le => "LE",
                Cmp::Gt => "GT",
                Cmp::Ge => "GE",
                Cmp::EqF32 => "EQ_F32",
                Cmp::NeF32 => "NE_F32",
                Cmp::EqBool => "EQ_BOOL",
                Cmp::NeBool => "NE_BOOL",
            }
        ),
        Instr::Jump(t) => write!(out, "JUMP           @{t:04}"),
        Instr::JumpIfFalse(t) => write!(out, "JUMP_IF_FALSE  @{t:04}"),
        Instr::SetPosition { control, position } => {
            let c = db.control(control);
            write!(
                out,
                "SET_POSITION   #{} {} ; {} = {}",
                control.0,
                position,
                c.map(|c| c.name).unwrap_or("?"),
                c.and_then(|c| db.position_name(&c, position))
                    .unwrap_or("?")
            )
        }
        Instr::SetAnalog { control, value } => write!(
            out,
            "SET_ANALOG     #{} {} ; {}",
            control.0,
            value,
            db.control(control).map(|c| c.name).unwrap_or("?")
        ),
        Instr::Check(control) => write!(
            out,
            "CHECK          #{} ; {}",
            control.0,
            db.control(control).map(|c| c.name).unwrap_or("?")
        ),
        Instr::Notify(s) => write!(
            out,
            "NOTIFY         ${} ; \"{}\"",
            s,
            db.string(s).unwrap_or("?")
        ),
        Instr::Call(p) => write!(
            out,
            "CALL           #{} ; {}",
            p.0,
            db.procedure(ProcedureIndex(p.0))
                .map(|p| p.id)
                .unwrap_or("?")
        ),
        Instr::Await {
            body_len,
            timeout_ms,
            on_timeout: mode,
        } => write!(
            out,
            "AWAIT          body={body_len} timeout={}ms on_timeout={}",
            timeout_ms,
            if mode == on_timeout::FAIL {
                "fail"
            } else {
                "continue"
            }
        ),
        Instr::AwaitTest => write!(out, "AWAIT_TEST"),
        Instr::Require(s) => write!(
            out,
            "REQUIRE        ${} ; \"{}\"",
            s,
            if s == format::NO_STRING {
                ""
            } else {
                db.string(s).unwrap_or("?")
            }
        ),
        Instr::Complete => write!(out, "COMPLETE"),
        Instr::Fail(s) => write!(
            out,
            "FAIL           ${} ; \"{}\"",
            s,
            if s == format::NO_STRING {
                ""
            } else {
                db.string(s).unwrap_or("?")
            }
        ),
        Instr::End => write!(out, "END"),
    }
}
