pub const MAGIC: [u8; 4] = *b"FEBC";
pub const FORMAT_VERSION: u16 = 1;
pub const HEADER_SIZE: usize = 80;
pub const PROC_RECORD_SIZE: usize = 32;
pub const SYMBOL_RECORD_SIZE: usize = 12;
pub const CONTROL_RECORD_SIZE: usize = 16;
pub const POSITION_RECORD_SIZE: usize = 4;
pub const STACK_CAPACITY: usize = 32;
pub const MAX_CALL_DEPTH: usize = 8;
pub const NO_STRING: u32 = u32::MAX;

pub mod header {
    pub const MAGIC: usize = 0;
    pub const FORMAT_VERSION: usize = 4;
    pub const HEADER_SIZE: usize = 6;
    pub const TOTAL_SIZE: usize = 8;
    pub const CONTENT_HASH: usize = 12;
    pub const FLAGS: usize = 16;
    pub const PROC_COUNT: usize = 20;
    pub const PROC_OFFSET: usize = 24;
    pub const SYMBOL_COUNT: usize = 28;
    pub const SYMBOL_OFFSET: usize = 32;
    pub const CONTROL_COUNT: usize = 36;
    pub const CONTROL_OFFSET: usize = 40;
    pub const POSITION_COUNT: usize = 44;
    pub const POSITION_OFFSET: usize = 48;
    pub const STRING_COUNT: usize = 52;
    pub const STRING_INDEX_OFFSET: usize = 56;
    pub const STRING_BLOB_OFFSET: usize = 60;
    pub const STRING_BLOB_LEN: usize = 64;
    pub const CODE_OFFSET: usize = 68;
    pub const CODE_LEN: usize = 72;
    pub const RESERVED: usize = 76;
}

pub mod flags {
    pub const CONTENT_HASH: u32 = 1 << 0;
}

pub mod proc_rec {
    pub const ID_STR: usize = 0;
    pub const NAME_STR: usize = 4;
    pub const DESC_STR: usize = 8;
    pub const CODE_OFF: usize = 12;
    pub const CODE_LEN: usize = 16;
    pub const TRIGGER_OFF: usize = 20;
    pub const TRIGGER_LEN: usize = 24;
    pub const CATEGORY: usize = 28;
    pub const PRIORITY: usize = 29;
    pub const REVISION: usize = 30;
}

pub mod op {
    // expression: push
    pub const NOP: u8 = 0x00;
    pub const PUSH_F32: u8 = 0x01;
    pub const PUSH_TRUE: u8 = 0x02;
    pub const PUSH_FALSE: u8 = 0x03;
    pub const LOAD_F32: u8 = 0x04;
    pub const LOAD_BOOL: u8 = 0x05;

    // expression: logic
    pub const NOT: u8 = 0x10;
    pub const AND: u8 = 0x11;
    pub const OR: u8 = 0x12;

    // expression: comparison
    pub const LT: u8 = 0x18;
    pub const LE: u8 = 0x19;
    pub const GT: u8 = 0x1A;
    pub const GE: u8 = 0x1B;
    pub const EQ_F32: u8 = 0x1C;
    pub const NE_F32: u8 = 0x1D;
    pub const EQ_BOOL: u8 = 0x1E;
    pub const NE_BOOL: u8 = 0x1F;

    // control flow
    pub const JUMP: u8 = 0x20;
    pub const JUMP_IF_FALSE: u8 = 0x21;

    // actions
    pub const SET_POSITION: u8 = 0x30;
    pub const SET_ANALOG: u8 = 0x31;
    pub const CHECK: u8 = 0x32;
    pub const NOTIFY: u8 = 0x33;
    pub const CALL: u8 = 0x34;

    //  waiting
    pub const AWAIT: u8 = 0x40;
    pub const AWAIT_TEST: u8 = 0x41;

    // termination
    pub const REQUIRE: u8 = 0x50;
    pub const COMPLETE: u8 = 0x60;
    pub const FAIL: u8 = 0x61;
    pub const END: u8 = 0x62;
}

pub const fn instruction_len(opcode: u8) -> Option<usize> {
    use op::*;
    Some(match opcode {
        NOP | PUSH_TRUE | PUSH_FALSE => 1,
        PUSH_F32 => 5,
        LOAD_F32 | LOAD_BOOL => 3,
        NOT | AND | OR => 1,
        LT | LE | GT | GE | EQ_F32 | NE_F32 | EQ_BOOL | NE_BOOL => 1,
        JUMP | JUMP_IF_FALSE => 5,
        SET_POSITION => 4,
        SET_ANALOG => 7,
        CHECK => 3,
        NOTIFY => 5,
        CALL => 3,
        AWAIT => 8,
        AWAIT_TEST => 1,
        REQUIRE => 5,
        COMPLETE | END => 1,
        FAIL => 5,
        _ => return None,
    })
}

pub mod on_timeout {
    pub const CONTINUE: u8 = 0;
    pub const FAIL: u8 = 1;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Category {
    Normal = 0,
    Abnormal = 1,
    Emergency = 2,
    Reference = 3,
    Other = 255,
}

impl Category {
    pub const fn from_u8(v: u8) -> Category {
        match v {
            0 => Category::Normal,
            1 => Category::Abnormal,
            2 => Category::Emergency,
            3 => Category::Reference,
            _ => Category::Other,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Category::Normal => "normal",
            Category::Abnormal => "abnormal",
            Category::Emergency => "emergency",
            Category::Reference => "reference",
            Category::Other => "other",
        }
    }
}

pub fn fnv1a32(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for b in bytes {
        hash ^= *b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}
