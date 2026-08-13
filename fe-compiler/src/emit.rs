use fe_lang::diagnostics::{Diagnostic, Label, codes};
use fe_runtime::format::{self, flags, header, proc_rec};

use crate::codegen::{self, CodegenError};
use crate::ir::IrModule;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub procedures: usize,
    pub symbols: usize,
    pub controls: usize,
    pub strings: usize,
    /// Total size of the code section in bytes
    pub code_bytes: usize,
    /// Total size of the database in bytes
    pub total_bytes: usize,
}

pub(crate) fn emit(module: &IrModule) -> Result<(Vec<u8>, Stats), Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();

    let mut code = Vec::new();
    let mut placement = Vec::with_capacity(module.procedures.len());
    for procedure in &module.procedures {
        match codegen::generate(procedure) {
            Ok(blob) => {
                let body_off = code.len() as u32;
                code.extend_from_slice(&blob.body);
                let trigger_off = code.len() as u32;
                code.extend_from_slice(&blob.trigger);
                placement.push(Placement {
                    body_off,
                    body_len: blob.body.len() as u32,
                    trigger_off,
                    trigger_len: blob.trigger.len() as u32,
                    max_stack: blob.max_stack as u8,
                });
            }
            Err(error) => {
                let name = module
                    .strings
                    .get(procedure.id as usize)
                    .cloned()
                    .unwrap_or_default();
                diagnostics.push(match error {
                    CodegenError::StackTooDeep { needed } => Diagnostic::error(
                        codes::PROCEDURE_TOO_COMPLEX,
                        format!(
                            "`{name}` needs {needed} expression stack slots; the runtime provides {}",
                            format::STACK_CAPACITY
                        ),
                        Label::bare(procedure.span),
                    )
                    .with_help("split the condition across nested `if` steps"),
                    CodegenError::BodyTooLarge { size } => Diagnostic::error(
                        codes::PROCEDURE_TOO_COMPLEX,
                        format!("`{name}` compiles to {size} bytes, which is too large"),
                        Label::bare(procedure.span),
                    ),
                });
            }
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    let mut positions: Vec<u32> = Vec::new();
    let mut control_positions = Vec::with_capacity(module.controls.len());
    for control in &module.controls {
        let first = positions.len();
        positions.extend(control.positions.iter().copied());
        control_positions.push((first as u16, control.positions.len() as u8));
    }

    let too_large = |what: &str, count: usize, limit: usize| {
        Diagnostic::error(
            codes::DATABASE_TOO_LARGE,
            format!("the procedure database has {count} {what}; the limit is {limit}"),
            Label::bare(
                module
                    .procedures
                    .first()
                    .map(|p| p.span)
                    .unwrap_or(fe_lang::span::Span::new(fe_lang::span::UnitId(0), 0, 0)),
            ),
        )
    };
    if module.procedures.len() > u16::MAX as usize {
        return Err(vec![too_large(
            "procedures",
            module.procedures.len(),
            u16::MAX as usize,
        )]);
    }
    if module.symbols.len() > u16::MAX as usize {
        return Err(vec![too_large(
            "symbols",
            module.symbols.len(),
            u16::MAX as usize,
        )]);
    }
    if module.controls.len() > u16::MAX as usize {
        return Err(vec![too_large(
            "controls",
            module.controls.len(),
            u16::MAX as usize,
        )]);
    }
    if positions.len() > u16::MAX as usize {
        return Err(vec![too_large(
            "control positions",
            positions.len(),
            u16::MAX as usize,
        )]);
    }

    let mut blob = Vec::new();
    let mut string_index = Vec::with_capacity(module.strings.len() + 1);
    for text in &module.strings {
        string_index.push(blob.len() as u32);
        blob.extend_from_slice(text.as_bytes());
    }
    string_index.push(blob.len() as u32);

    let mut cursor = format::HEADER_SIZE;
    let proc_offset = cursor;
    cursor += module.procedures.len() * format::PROC_RECORD_SIZE;
    let symbol_offset = align(&mut cursor);
    cursor += module.symbols.len() * format::SYMBOL_RECORD_SIZE;
    let control_offset = align(&mut cursor);
    cursor += module.controls.len() * format::CONTROL_RECORD_SIZE;
    let position_offset = align(&mut cursor);
    cursor += positions.len() * format::POSITION_RECORD_SIZE;
    let string_index_offset = align(&mut cursor);
    cursor += string_index.len() * 4;
    let string_blob_offset = align(&mut cursor);
    cursor += blob.len();
    let code_offset = align(&mut cursor);
    cursor += code.len();
    let total_size = cursor;

    let mut out = vec![0u8; total_size];

    out[header::MAGIC..header::MAGIC + 4].copy_from_slice(&format::MAGIC);
    put_u16(&mut out, header::FORMAT_VERSION, format::FORMAT_VERSION);
    put_u16(&mut out, header::HEADER_SIZE, format::HEADER_SIZE as u16);
    put_u32(&mut out, header::TOTAL_SIZE, total_size as u32);
    put_u32(&mut out, header::FLAGS, flags::CONTENT_HASH);
    put_u32(&mut out, header::PROC_COUNT, module.procedures.len() as u32);
    put_u32(&mut out, header::PROC_OFFSET, proc_offset as u32);
    put_u32(&mut out, header::SYMBOL_COUNT, module.symbols.len() as u32);
    put_u32(&mut out, header::SYMBOL_OFFSET, symbol_offset as u32);
    put_u32(
        &mut out,
        header::CONTROL_COUNT,
        module.controls.len() as u32,
    );
    put_u32(&mut out, header::CONTROL_OFFSET, control_offset as u32);
    put_u32(&mut out, header::POSITION_COUNT, positions.len() as u32);
    put_u32(&mut out, header::POSITION_OFFSET, position_offset as u32);
    put_u32(&mut out, header::STRING_COUNT, module.strings.len() as u32);
    put_u32(
        &mut out,
        header::STRING_INDEX_OFFSET,
        string_index_offset as u32,
    );
    put_u32(
        &mut out,
        header::STRING_BLOB_OFFSET,
        string_blob_offset as u32,
    );
    put_u32(&mut out, header::STRING_BLOB_LEN, blob.len() as u32);
    put_u32(&mut out, header::CODE_OFFSET, code_offset as u32);
    put_u32(&mut out, header::CODE_LEN, code.len() as u32);

    for (index, procedure) in module.procedures.iter().enumerate() {
        let at = proc_offset + index * format::PROC_RECORD_SIZE;
        let place = &placement[index];
        put_u32(&mut out, at + proc_rec::ID_STR, procedure.id);
        put_u32(&mut out, at + proc_rec::NAME_STR, procedure.name);
        put_u32(
            &mut out,
            at + proc_rec::DESC_STR,
            procedure.description.unwrap_or(format::NO_STRING),
        );
        put_u32(&mut out, at + proc_rec::CODE_OFF, place.body_off);
        put_u32(&mut out, at + proc_rec::CODE_LEN, place.body_len);
        put_u32(&mut out, at + proc_rec::TRIGGER_OFF, place.trigger_off);
        put_u32(&mut out, at + proc_rec::TRIGGER_LEN, place.trigger_len);
        out[at + proc_rec::CATEGORY] = procedure.category;
        out[at + proc_rec::PRIORITY] = procedure.priority;
        put_u16(&mut out, at + proc_rec::REVISION, procedure.revision);
        let _ = place.max_stack;
    }

    for (index, symbol) in module.symbols.iter().enumerate() {
        let at = symbol_offset + index * format::SYMBOL_RECORD_SIZE;
        put_u32(&mut out, at, symbol.name);
        put_u32(&mut out, at + 4, symbol.tag);
        out[at + 8] = symbol.ty as u8;
    }
    for (index, control) in module.controls.iter().enumerate() {
        let at = control_offset + index * format::CONTROL_RECORD_SIZE;
        let (first, count) = control_positions[index];
        put_u32(&mut out, at, control.name);
        put_u32(&mut out, at + 4, control.tag);
        out[at + 8] = control.kind as u8;
        out[at + 9] = count;
        put_u16(&mut out, at + 10, first);
    }
    for (index, string_id) in positions.iter().enumerate() {
        put_u32(&mut out, position_offset + index * 4, *string_id);
    }

    for (index, offset) in string_index.iter().enumerate() {
        put_u32(&mut out, string_index_offset + index * 4, *offset);
    }
    out[string_blob_offset..string_blob_offset + blob.len()].copy_from_slice(&blob);
    out[code_offset..code_offset + code.len()].copy_from_slice(&code);

    let hash = format::fnv1a32(&out[format::HEADER_SIZE..]);
    put_u32(&mut out, header::CONTENT_HASH, hash);

    let stats = Stats {
        procedures: module.procedures.len(),
        symbols: module.symbols.len(),
        controls: module.controls.len(),
        strings: module.strings.len(),
        code_bytes: code.len(),
        total_bytes: total_size,
    };
    Ok((out, stats))
}

struct Placement {
    body_off: u32,
    body_len: u32,
    trigger_off: u32,
    trigger_len: u32,
    max_stack: u8,
}

fn align(cursor: &mut usize) -> usize {
    let padded = (*cursor + 3) & !3;
    *cursor = padded;
    padded
}

fn put_u16(out: &mut [u8], at: usize, value: u16) {
    out[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(out: &mut [u8], at: usize, value: u32) {
    out[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
