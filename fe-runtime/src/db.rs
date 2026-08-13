use core::fmt;

use crate::format::{self, Category, header, op};
use crate::value::{ControlKind, ValueType};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FormatError {
    /// Fewer bytes than a header.
    TooSmall,
    /// The first four bytes were not FEBC
    BadMagic,
    /// The file declares a format version this runtime does not implement
    UnsupportedVersion { found: u16, supported: u16 },
    /// header_size / total_size disagree with the slice we were given
    BadHeader,
    /// A section offset or count runs past the end of the file, overflows,
    /// or is misaligned
    BadSection { section: Section },
    /// The string index is not monotonic, or a string is not valid utf8
    BadStringTable,
    /// A record referenced a table entry that does not exist
    BadReference,
    /// Procedures are not in the canonical (sorted, unique) order
    BadProcedureOrder,
    /// content_hash did not match the payload
    ChecksumMismatch,
    /// The bytecode of a procedure failed verification
    BadBytecode {
        procedure: u32,
        offset: u32,
        kind: BytecodeError,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Procedures,
    Symbols,
    Controls,
    Positions,
    Strings,
    Code,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BytecodeError {
    /// Opcode is not in the instruction set
    UnknownOpcode(u8),
    /// The instructions operands run past the end of the procedure
    Truncated,
    /// Operand names a table entry that does not exist
    BadOperand,
    /// A jump target is backwards, out of range, or not an instruction start
    BadJumpTarget,
    /// Stack would overflow, underflow, or hold the wrong type
    StackDiscipline,
    /// AWAIT / AWAIT_TEST are not correctly paired
    BadWait,
    /// Procedure does not end with END, or execution can run off the end
    MissingEnd,
    /// Expression region left something other than a single bool on the stack
    BadExpression,
    /// Too many unresolved forward branches (nesting limit)
    TooComplex,
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::TooSmall => write!(f, "byte slice is smaller than a database header"),
            FormatError::BadMagic => write!(f, "not a .febin database (bad magic)"),
            FormatError::UnsupportedVersion { found, supported } => write!(
                f,
                "database format version {found} is not supported (this runtime reads {supported})"
            ),
            FormatError::BadHeader => write!(f, "malformed header"),
            FormatError::BadSection { section } => write!(f, "malformed {section:?} section"),
            FormatError::BadStringTable => write!(f, "malformed string table"),
            FormatError::BadReference => write!(f, "record references a missing table entry"),
            FormatError::BadProcedureOrder => {
                write!(f, "procedure table is unordered or has duplicates")
            }
            FormatError::ChecksumMismatch => write!(f, "content hash mismatch"),
            FormatError::BadBytecode {
                procedure,
                offset,
                kind,
            } => {
                write!(
                    f,
                    "invalid bytecode in procedure {procedure} at +{offset}: {kind:?}"
                )
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for FormatError {}

#[derive(Clone, Copy)]
pub(crate) struct Layout {
    pub total_size: usize,
    pub proc_count: u32,
    pub proc_offset: usize,
    pub symbol_count: u32,
    pub symbol_offset: usize,
    pub control_count: u32,
    pub control_offset: usize,
    pub position_count: u32,
    pub position_offset: usize,
    pub string_count: u32,
    pub string_index_offset: usize,
    pub string_blob_offset: usize,
    pub string_blob_len: usize,
    pub code_offset: usize,
    pub code_len: usize,
}

#[derive(Clone, Copy)]
pub struct ProcedureDatabase<'a> {
    bytes: &'a [u8],
    pub(crate) layout: Layout,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct SymbolId(pub u16);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct ControlId(pub u16);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct ProcedureIndex(pub u16);

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    let b = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn section(
    bytes_len: usize,
    offset: u32,
    count: u32,
    record: usize,
    which: Section,
) -> Result<usize, FormatError> {
    let err = FormatError::BadSection { section: which };
    let offset = offset as usize;
    if offset % 4 != 0 {
        return Err(err);
    }
    let span = (count as usize).checked_mul(record).ok_or(err)?;
    let end = offset.checked_add(span).ok_or(err)?;
    if end > bytes_len {
        return Err(err);
    }
    Ok(offset)
}

impl<'a> ProcedureDatabase<'a> {
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self, FormatError> {
        if bytes.len() < format::HEADER_SIZE {
            return Err(FormatError::TooSmall);
        }
        if bytes[0..4] != format::MAGIC {
            return Err(FormatError::BadMagic);
        }
        let version = read_u16(bytes, header::FORMAT_VERSION).ok_or(FormatError::BadHeader)?;
        if version != format::FORMAT_VERSION {
            return Err(FormatError::UnsupportedVersion {
                found: version,
                supported: format::FORMAT_VERSION,
            });
        }
        let header_size =
            read_u16(bytes, header::HEADER_SIZE).ok_or(FormatError::BadHeader)? as usize;
        if header_size < format::HEADER_SIZE || header_size % 4 != 0 || header_size > bytes.len() {
            return Err(FormatError::BadHeader);
        }
        let total_size =
            read_u32(bytes, header::TOTAL_SIZE).ok_or(FormatError::BadHeader)? as usize;
        if total_size < header_size || total_size > bytes.len() {
            return Err(FormatError::BadHeader);
        }
        let bytes = &bytes[..total_size];

        let flags = read_u32(bytes, header::FLAGS).ok_or(FormatError::BadHeader)?;
        let proc_count = read_u32(bytes, header::PROC_COUNT).ok_or(FormatError::BadHeader)?;
        let symbol_count = read_u32(bytes, header::SYMBOL_COUNT).ok_or(FormatError::BadHeader)?;
        let control_count = read_u32(bytes, header::CONTROL_COUNT).ok_or(FormatError::BadHeader)?;
        let position_count =
            read_u32(bytes, header::POSITION_COUNT).ok_or(FormatError::BadHeader)?;
        let string_count = read_u32(bytes, header::STRING_COUNT).ok_or(FormatError::BadHeader)?;

        if proc_count > u16::MAX as u32
            || symbol_count > u16::MAX as u32
            || control_count > u16::MAX as u32
        {
            return Err(FormatError::BadHeader);
        }

        let proc_offset = section(
            total_size,
            read_u32(bytes, header::PROC_OFFSET).ok_or(FormatError::BadHeader)?,
            proc_count,
            format::PROC_RECORD_SIZE,
            Section::Procedures,
        )?;
        let symbol_offset = section(
            total_size,
            read_u32(bytes, header::SYMBOL_OFFSET).ok_or(FormatError::BadHeader)?,
            symbol_count,
            format::SYMBOL_RECORD_SIZE,
            Section::Symbols,
        )?;
        let control_offset = section(
            total_size,
            read_u32(bytes, header::CONTROL_OFFSET).ok_or(FormatError::BadHeader)?,
            control_count,
            format::CONTROL_RECORD_SIZE,
            Section::Controls,
        )?;
        let position_offset = section(
            total_size,
            read_u32(bytes, header::POSITION_OFFSET).ok_or(FormatError::BadHeader)?,
            position_count,
            format::POSITION_RECORD_SIZE,
            Section::Positions,
        )?;
        let string_index_offset = section(
            total_size,
            read_u32(bytes, header::STRING_INDEX_OFFSET).ok_or(FormatError::BadHeader)?,
            string_count.checked_add(1).ok_or(FormatError::BadHeader)?,
            4,
            Section::Strings,
        )?;
        let string_blob_len =
            read_u32(bytes, header::STRING_BLOB_LEN).ok_or(FormatError::BadHeader)? as usize;
        let string_blob_offset = section(
            total_size,
            read_u32(bytes, header::STRING_BLOB_OFFSET).ok_or(FormatError::BadHeader)?,
            string_blob_len as u32,
            1,
            Section::Strings,
        )?;
        let code_len = read_u32(bytes, header::CODE_LEN).ok_or(FormatError::BadHeader)? as usize;
        let code_offset = section(
            total_size,
            read_u32(bytes, header::CODE_OFFSET).ok_or(FormatError::BadHeader)?,
            code_len as u32,
            1,
            Section::Code,
        )?;

        let layout = Layout {
            total_size,
            proc_count,
            proc_offset,
            symbol_count,
            symbol_offset,
            control_count,
            control_offset,
            position_count,
            position_offset,
            string_count,
            string_index_offset,
            string_blob_offset,
            string_blob_len,
            code_offset,
            code_len,
        };
        let db = ProcedureDatabase { bytes, layout };

        if flags & format::flags::CONTENT_HASH != 0 {
            let stored = read_u32(bytes, header::CONTENT_HASH).ok_or(FormatError::BadHeader)?;
            if format::fnv1a32(&bytes[header_size..]) != stored {
                return Err(FormatError::ChecksumMismatch);
            }
        }

        db.validate_strings()?;
        db.validate_tables()?;
        db.validate_procedures()?;
        Ok(db)
    }

    pub fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn size_bytes(&self) -> usize {
        self.layout.total_size
    }

    fn validate_strings(&self) -> Result<(), FormatError> {
        let l = &self.layout;
        let mut prev = 0u32;
        for i in 0..=l.string_count {
            let off = read_u32(self.bytes, l.string_index_offset + i as usize * 4)
                .ok_or(FormatError::BadStringTable)?;
            if off < prev || off as usize > l.string_blob_len {
                return Err(FormatError::BadStringTable);
            }
            prev = off;
        }
        if prev as usize != l.string_blob_len {
            return Err(FormatError::BadStringTable);
        }
        let blob = self
            .bytes
            .get(l.string_blob_offset..l.string_blob_offset + l.string_blob_len)
            .ok_or(FormatError::BadStringTable)?;
        core::str::from_utf8(blob).map_err(|_| FormatError::BadStringTable)?;
        for i in 0..l.string_count {
            self.string(i).ok_or(FormatError::BadStringTable)?;
        }
        Ok(())
    }

    pub fn string(&self, id: u32) -> Option<&'a str> {
        let l = &self.layout;
        if id >= l.string_count {
            return None;
        }
        let start = read_u32(self.bytes, l.string_index_offset + id as usize * 4)? as usize;
        let end = read_u32(self.bytes, l.string_index_offset + (id as usize + 1) * 4)? as usize;
        if end < start || end > l.string_blob_len {
            return None;
        }
        let blob = self
            .bytes
            .get(l.string_blob_offset + start..l.string_blob_offset + end)?;
        core::str::from_utf8(blob).ok()
    }

    fn checked_string(&self, id: u32) -> Result<(), FormatError> {
        if id == format::NO_STRING {
            return Ok(());
        }
        self.string(id).map(|_| ()).ok_or(FormatError::BadReference)
    }

    fn validate_tables(&self) -> Result<(), FormatError> {
        for i in 0..self.layout.symbol_count as u16 {
            let s = self.symbol(SymbolId(i)).ok_or(FormatError::BadReference)?;
            self.checked_string(s.name_str)?;
        }
        for i in 0..self.layout.control_count as u16 {
            let c = self
                .control(ControlId(i))
                .ok_or(FormatError::BadReference)?;
            self.checked_string(c.name_str)?;
            let last = (c.first_position as u32)
                .checked_add(c.position_count as u32)
                .ok_or(FormatError::BadReference)?;
            if last > self.layout.position_count {
                return Err(FormatError::BadReference);
            }
            for p in 0..c.position_count {
                let id = self
                    .position_string_id(c.first_position + p as u16)
                    .ok_or(FormatError::BadReference)?;
                self.checked_string(id)?;
            }
        }
        Ok(())
    }

    pub fn symbol_count(&self) -> u16 {
        self.layout.symbol_count as u16
    }

    pub fn control_count(&self) -> u16 {
        self.layout.control_count as u16
    }

    pub fn symbol(&self, id: SymbolId) -> Option<Symbol<'a>> {
        if id.0 as u32 >= self.layout.symbol_count {
            return None;
        }
        let at = self.layout.symbol_offset + id.0 as usize * format::SYMBOL_RECORD_SIZE;
        let name_str = read_u32(self.bytes, at)?;
        let tag = read_u32(self.bytes, at + 4)?;
        let ty = ValueType::from_u8(*self.bytes.get(at + 8)?)?;
        Some(Symbol {
            id,
            tag,
            ty,
            name_str,
            name: self.string(name_str).unwrap_or(""),
        })
    }

    pub fn control(&self, id: ControlId) -> Option<Control<'a>> {
        if id.0 as u32 >= self.layout.control_count {
            return None;
        }
        let at = self.layout.control_offset + id.0 as usize * format::CONTROL_RECORD_SIZE;
        let name_str = read_u32(self.bytes, at)?;
        let tag = read_u32(self.bytes, at + 4)?;
        let kind = ControlKind::from_u8(*self.bytes.get(at + 8)?);
        let position_count = *self.bytes.get(at + 9)?;
        let first_position = read_u16(self.bytes, at + 10)?;
        Some(Control {
            id,
            tag,
            kind,
            position_count,
            first_position,
            name_str,
            name: self.string(name_str).unwrap_or(""),
        })
    }

    fn position_string_id(&self, index: u16) -> Option<u32> {
        if index as u32 >= self.layout.position_count {
            return None;
        }
        read_u32(self.bytes, self.layout.position_offset + index as usize * 4)
    }

    pub fn position_name(&self, control: &Control<'a>, index: u8) -> Option<&'a str> {
        if index >= control.position_count {
            return None;
        }
        let id = self.position_string_id(control.first_position + index as u16)?;
        self.string(id)
    }

    pub fn procedure_count(&self) -> u16 {
        self.layout.proc_count as u16
    }

    pub fn procedure(&self, index: ProcedureIndex) -> Option<Procedure<'a>> {
        if index.0 as u32 >= self.layout.proc_count {
            return None;
        }
        let at = self.layout.proc_offset + index.0 as usize * format::PROC_RECORD_SIZE;
        use format::proc_rec as r;
        let id_str = read_u32(self.bytes, at + r::ID_STR)?;
        let name_str = read_u32(self.bytes, at + r::NAME_STR)?;
        let desc_str = read_u32(self.bytes, at + r::DESC_STR)?;
        Some(Procedure {
            db: *self,
            index,
            id: self.string(id_str).unwrap_or(""),
            name: self.string(name_str).unwrap_or(""),
            description: if desc_str == format::NO_STRING {
                None
            } else {
                self.string(desc_str)
            },
            code_off: read_u32(self.bytes, at + r::CODE_OFF)?,
            code_len: read_u32(self.bytes, at + r::CODE_LEN)?,
            trigger_off: read_u32(self.bytes, at + r::TRIGGER_OFF)?,
            trigger_len: read_u32(self.bytes, at + r::TRIGGER_LEN)?,
            category: Category::from_u8(*self.bytes.get(at + r::CATEGORY)?),
            priority: *self.bytes.get(at + r::PRIORITY)?,
            revision: read_u16(self.bytes, at + r::REVISION)?,
        })
    }

    pub fn get_procedure(&self, id: &str) -> Option<Procedure<'a>> {
        let mut lo = 0u32;
        let mut hi = self.layout.proc_count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let p = self.procedure(ProcedureIndex(mid as u16))?;
            match p.id.cmp(id) {
                core::cmp::Ordering::Less => lo = mid + 1,
                core::cmp::Ordering::Greater => hi = mid,
                core::cmp::Ordering::Equal => return Some(p),
            }
        }
        None
    }

    pub fn procedures(&self) -> impl Iterator<Item = Procedure<'a>> + '_ {
        let db = *self;
        (0..self.layout.proc_count as u16).filter_map(move |i| db.procedure(ProcedureIndex(i)))
    }

    pub fn symbols(&self) -> impl Iterator<Item = Symbol<'a>> + '_ {
        let db = *self;
        (0..self.layout.symbol_count as u16).filter_map(move |i| db.symbol(SymbolId(i)))
    }

    pub fn controls(&self) -> impl Iterator<Item = Control<'a>> + '_ {
        let db = *self;
        (0..self.layout.control_count as u16).filter_map(move |i| db.control(ControlId(i)))
    }

    pub(crate) fn code(&self) -> &'a [u8] {
        match self
            .bytes
            .get(self.layout.code_offset..self.layout.code_offset + self.layout.code_len)
        {
            Some(s) => s,
            None => &[],
        }
    }

    fn validate_procedures(&self) -> Result<(), FormatError> {
        let mut prev: Option<&str> = None;
        for i in 0..self.layout.proc_count as u16 {
            let at = self.layout.proc_offset + i as usize * format::PROC_RECORD_SIZE;
            use format::proc_rec as r;
            let id_str = read_u32(self.bytes, at + r::ID_STR).ok_or(FormatError::BadReference)?;
            if id_str == format::NO_STRING {
                return Err(FormatError::BadReference);
            }
            self.checked_string(id_str)?;
            self.checked_string(
                read_u32(self.bytes, at + r::NAME_STR).ok_or(FormatError::BadReference)?,
            )?;
            self.checked_string(
                read_u32(self.bytes, at + r::DESC_STR).ok_or(FormatError::BadReference)?,
            )?;

            let p = self
                .procedure(ProcedureIndex(i))
                .ok_or(FormatError::BadReference)?;
            if p.id.is_empty() {
                return Err(FormatError::BadReference);
            }
            if let Some(prev) = prev {
                if prev >= p.id {
                    return Err(FormatError::BadProcedureOrder);
                }
            }
            prev = Some(p.id);

            let end = (p.code_off as usize)
                .checked_add(p.code_len as usize)
                .ok_or(FormatError::BadSection {
                    section: Section::Code,
                })?;
            if end > self.layout.code_len || p.code_len == 0 {
                return Err(FormatError::BadSection {
                    section: Section::Code,
                });
            }
            let tend = (p.trigger_off as usize)
                .checked_add(p.trigger_len as usize)
                .ok_or(FormatError::BadSection {
                    section: Section::Code,
                })?;
            if tend > self.layout.code_len {
                return Err(FormatError::BadSection {
                    section: Section::Code,
                });
            }
            crate::verify::verify_body(self, &p).map_err(|(offset, kind)| {
                FormatError::BadBytecode {
                    procedure: i as u32,
                    offset,
                    kind,
                }
            })?;
            if p.trigger_len > 0 {
                crate::verify::verify_expression(self, p.trigger_code()).map_err(
                    |(offset, kind)| FormatError::BadBytecode {
                        procedure: i as u32,
                        offset,
                        kind,
                    },
                )?;
            }
        }
        Ok(())
    }
}

impl fmt::Debug for ProcedureDatabase<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcedureDatabase")
            .field("procedures", &self.layout.proc_count)
            .field("symbols", &self.layout.symbol_count)
            .field("controls", &self.layout.control_count)
            .field("strings", &self.layout.string_count)
            .field("bytes", &self.layout.total_size)
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Symbol<'a> {
    pub id: SymbolId,
    pub tag: u32,
    pub ty: ValueType,
    pub name: &'a str,
    pub(crate) name_str: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Control<'a> {
    pub id: ControlId,
    pub tag: u32,
    pub kind: ControlKind,
    pub position_count: u8,
    pub name: &'a str,
    pub(crate) first_position: u16,
    pub(crate) name_str: u32,
}

#[derive(Clone, Copy)]
pub struct Procedure<'a> {
    pub(crate) db: ProcedureDatabase<'a>,
    pub index: ProcedureIndex,
    pub id: &'a str,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub category: Category,
    pub priority: u8,
    pub revision: u16,
    pub(crate) code_off: u32,
    pub(crate) code_len: u32,
    pub(crate) trigger_off: u32,
    pub(crate) trigger_len: u32,
}

impl<'a> Procedure<'a> {
    pub fn body_code(&self) -> &'a [u8] {
        let code = self.db.code();
        let start = self.code_off as usize;
        let end = start + self.code_len as usize;
        code.get(start..end).unwrap_or(&[])
    }

    pub fn trigger_code(&self) -> &'a [u8] {
        let code = self.db.code();
        let start = self.trigger_off as usize;
        let end = start + self.trigger_len as usize;
        code.get(start..end).unwrap_or(&[])
    }

    pub fn has_trigger(&self) -> bool {
        self.trigger_len > 0
    }

    pub fn code_size(&self) -> u32 {
        self.code_len
    }

    pub fn database(&self) -> ProcedureDatabase<'a> {
        self.db
    }
}

impl PartialEq for Procedure<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
            && core::ptr::eq(self.db.as_bytes().as_ptr(), other.db.as_bytes().as_ptr())
    }
}

impl fmt::Debug for Procedure<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Procedure")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("category", &self.category)
            .field("priority", &self.priority)
            .field("revision", &self.revision)
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Instr {
    Nop,
    PushF32(f32),
    PushBool(bool),
    LoadF32(SymbolId),
    LoadBool(SymbolId),
    Not,
    And,
    Or,
    Compare(Cmp),
    Jump(u32),
    JumpIfFalse(u32),
    SetPosition {
        control: ControlId,
        position: u8,
    },
    SetAnalog {
        control: ControlId,
        value: f32,
    },
    Check(ControlId),
    Notify(u32),
    Call(ProcedureIndex),
    Await {
        body_len: u16,
        timeout_ms: u32,
        on_timeout: u8,
    },
    AwaitTest,
    Require(u32),
    Complete,
    Fail(u32),
    End,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    EqF32,
    NeF32,
    EqBool,
    NeBool,
}

pub fn decode(code: &[u8], at: usize) -> Result<(Instr, usize), BytecodeError> {
    let opcode = *code.get(at).ok_or(BytecodeError::Truncated)?;
    let len = format::instruction_len(opcode).ok_or(BytecodeError::UnknownOpcode(opcode))?;
    let end = at.checked_add(len).ok_or(BytecodeError::Truncated)?;
    if end > code.len() {
        return Err(BytecodeError::Truncated);
    }
    let b = &code[at..end];
    let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let f32_at = |i: usize| f32::from_bits(u32_at(i));
    let instr = match opcode {
        op::NOP => Instr::Nop,
        op::PUSH_F32 => Instr::PushF32(f32_at(1)),
        op::PUSH_TRUE => Instr::PushBool(true),
        op::PUSH_FALSE => Instr::PushBool(false),
        op::LOAD_F32 => Instr::LoadF32(SymbolId(u16_at(1))),
        op::LOAD_BOOL => Instr::LoadBool(SymbolId(u16_at(1))),
        op::NOT => Instr::Not,
        op::AND => Instr::And,
        op::OR => Instr::Or,
        op::LT => Instr::Compare(Cmp::Lt),
        op::LE => Instr::Compare(Cmp::Le),
        op::GT => Instr::Compare(Cmp::Gt),
        op::GE => Instr::Compare(Cmp::Ge),
        op::EQ_F32 => Instr::Compare(Cmp::EqF32),
        op::NE_F32 => Instr::Compare(Cmp::NeF32),
        op::EQ_BOOL => Instr::Compare(Cmp::EqBool),
        op::NE_BOOL => Instr::Compare(Cmp::NeBool),
        op::JUMP => Instr::Jump(u32_at(1)),
        op::JUMP_IF_FALSE => Instr::JumpIfFalse(u32_at(1)),
        op::SET_POSITION => Instr::SetPosition {
            control: ControlId(u16_at(1)),
            position: b[3],
        },
        op::SET_ANALOG => Instr::SetAnalog {
            control: ControlId(u16_at(1)),
            value: f32_at(3),
        },
        op::CHECK => Instr::Check(ControlId(u16_at(1))),
        op::NOTIFY => Instr::Notify(u32_at(1)),
        op::CALL => Instr::Call(ProcedureIndex(u16_at(1))),
        op::AWAIT => Instr::Await {
            body_len: u16_at(1),
            timeout_ms: u32_at(3),
            on_timeout: b[7],
        },
        op::AWAIT_TEST => Instr::AwaitTest,
        op::REQUIRE => Instr::Require(u32_at(1)),
        op::COMPLETE => Instr::Complete,
        op::FAIL => Instr::Fail(u32_at(1)),
        op::END => Instr::End,
        other => return Err(BytecodeError::UnknownOpcode(other)),
    };
    Ok((instr, len))
}
