//! Binary format tests: structure, determinism, and what happens when the
//! bytes are wrong.
//!
//! The corruption matrix is the important half. A `.febin` will be shipped
//! inside an aircraft package, decompressed by someone else's installer, and
//! possibly edited by a curious user. Every one of these mutations must
//! produce a `FormatError` — never a panic, never a partially-usable database.

mod support;

use fe_runtime::format::{self, header};
use fe_runtime::{FormatError, ProcedureDatabase};

fn compiled() -> Vec<u8> {
    support::compile_examples()
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// Recompute the content hash so a mutation is tested against the *verifier*
/// rather than short-circuiting on the integrity check.
fn reseal(bytes: &mut [u8]) {
    let hash = format::fnv1a32(&bytes[format::HEADER_SIZE..]);
    put_u32(bytes, header::CONTENT_HASH, hash);
}

/// Apply a mutation, reseal, and assert the result is rejected.
#[track_caller]
fn rejects(what: &str, mutate: impl FnOnce(&mut Vec<u8>)) -> FormatError {
    let mut bytes = compiled();
    mutate(&mut bytes);
    if bytes.len() >= format::HEADER_SIZE {
        reseal(&mut bytes);
    }
    match ProcedureDatabase::from_bytes(&bytes) {
        Ok(_) => panic!("{what}: corrupt database was accepted"),
        Err(error) => error,
    }
}

// ---------------------------------------------------------------------------
// Structure
// ---------------------------------------------------------------------------

#[test]
fn header_is_well_formed() {
    let bytes = compiled();
    assert_eq!(&bytes[0..4], b"FEBC");
    assert_eq!(
        u16::from_le_bytes([
            bytes[header::FORMAT_VERSION],
            bytes[header::FORMAT_VERSION + 1]
        ]),
        format::FORMAT_VERSION
    );
    assert_eq!(u32_at(&bytes, header::TOTAL_SIZE) as usize, bytes.len());
}

#[test]
fn every_section_is_four_byte_aligned() {
    let bytes = compiled();
    for offset in [
        header::PROC_OFFSET,
        header::SYMBOL_OFFSET,
        header::CONTROL_OFFSET,
        header::POSITION_OFFSET,
        header::STRING_INDEX_OFFSET,
        header::STRING_BLOB_OFFSET,
        header::CODE_OFFSET,
    ] {
        assert_eq!(
            u32_at(&bytes, offset) % 4,
            0,
            "section at {offset} misaligned"
        );
    }
}

#[test]
fn the_database_round_trips() {
    let bytes = compiled();
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    assert_eq!(db.as_bytes(), &bytes[..]);
    assert_eq!(db.size_bytes(), bytes.len());
    assert!(db.procedure_count() >= 5);
}

#[test]
fn procedures_are_sorted_and_reachable_by_id() {
    let bytes = compiled();
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let ids: Vec<&str> = db.procedures().map(|p| p.id).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted, "procedure table must be sorted by id");

    // The binary search must find every one of them, and nothing else.
    for id in &ids {
        assert_eq!(db.get_procedure(id).unwrap().id, *id);
    }
    assert!(db.get_procedure("NOT_A_PROCEDURE").is_none());
    assert!(db.get_procedure("").is_none());
}

#[test]
fn metadata_survives_the_round_trip() {
    let bytes = compiled();
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let p = db.get_procedure("HYD_2_LOW_PRESSURE").unwrap();
    assert_eq!(p.name, "Hydraulic System 2 Low Pressure");
    assert_eq!(p.category, fe_runtime::Category::Abnormal);
    assert_eq!(p.priority, 80);
    assert_eq!(p.revision, 3);
    assert!(p.description.is_some());
    assert!(p.has_trigger());

    let plain = db.get_procedure("HYD_ALL_SYSTEMS_CHECK").unwrap();
    assert_eq!(plain.category, fe_runtime::Category::Normal);
    assert!(!plain.has_trigger());
    assert!(plain.description.is_none());
}

#[test]
fn symbols_and_controls_carry_their_host_tags() {
    let bytes = compiled();
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();

    let pressure = db
        .symbols()
        .find(|s| s.name == "hydraulic.2.pressure")
        .expect("symbol missing");
    assert_eq!(pressure.tag, support::tag::HYD2_PRESSURE);
    assert_eq!(pressure.ty, fe_runtime::ValueType::F32);

    let selector = db
        .controls()
        .find(|c| c.name == "FUEL_XFEED_SELECTOR")
        .expect("control missing");
    assert_eq!(selector.tag, support::tag::FUEL_XFEED_SELECTOR);
    assert_eq!(selector.kind, fe_runtime::ControlKind::Selector);
    assert_eq!(selector.position_count, 3);
    assert_eq!(db.position_name(&selector, 0), Some("OFF"));
    assert_eq!(db.position_name(&selector, 2), Some("TANK_3_TO_1"));
    assert_eq!(db.position_name(&selector, 3), None);
}

#[test]
fn only_referenced_symbols_are_in_the_table() {
    // The registry is the aircraft's whole vocabulary; the database should
    // carry only what the procedures actually touch, because the table is what
    // the host has to bind at load time.
    let compiled = support::compile_source(
        "procedure P { name \"P\" category normal wait hydraulic.2.pressure > 100 }",
    )
    .unwrap();
    let db = compiled.database();
    assert_eq!(db.symbol_count(), 1);
    assert_eq!(db.control_count(), 0);
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

#[test]
fn compilation_is_byte_for_byte_reproducible() {
    assert_eq!(compiled(), compiled());
}

#[test]
fn source_file_order_does_not_change_the_output() {
    // Build systems glob directories, and glob order is not a contract. If it
    // leaked into the output, every rebuild would look like a content change
    // to whatever ships the file.
    let registry = support::registry();
    let mut units = support::example_units();
    let first = fe_compiler::compile(&units, &registry)
        .unwrap()
        .into_bytes();
    units.reverse();
    let second = fe_compiler::compile(&units, &registry)
        .unwrap()
        .into_bytes();
    assert_eq!(first, second);
}

#[test]
fn unit_names_do_not_reach_the_output() {
    // A path is a property of somebody's build machine, not of the procedures.
    use fe_compiler::SourceUnit;
    let text = "procedure P { name \"P\" category normal complete }";
    let registry = support::registry();
    let a = fe_compiler::compile(&[SourceUnit::new("a.fe", text)], &registry)
        .unwrap()
        .into_bytes();
    let b = fe_compiler::compile(
        &[SourceUnit::new("/home/someone/build/b.fe", text)],
        &registry,
    )
    .unwrap()
    .into_bytes();
    assert_eq!(a, b);
}

#[test]
fn comments_and_whitespace_do_not_reach_the_output() {
    let bare = support::compile_source("procedure P { name \"P\" category normal complete }")
        .unwrap()
        .into_bytes();
    let decorated = support::compile_source(
        "// a comment\nprocedure P {\n\n    name \"P\"\n    /* block */ category normal\n    complete\n}\n",
    )
    .unwrap()
    .into_bytes();
    assert_eq!(bare, decorated);
}

#[test]
fn registry_tags_do_reach_the_output() {
    // The converse check: the registry is an input, so changing it must change
    // the bytes. Otherwise a stale database would silently outlive a renumbered
    // aircraft.
    use fe_compiler::{ControlSpec, SourceUnit, SymbolRegistry, ValueType};
    let source = "procedure P { name \"P\" category normal start PUMP }";
    let build = |tag: u32| {
        let mut r = SymbolRegistry::new();
        r.define_state("x", ValueType::Bool, 1).unwrap();
        r.define_control("PUMP", ControlSpec::switch(), tag)
            .unwrap();
        fe_compiler::compile(&[SourceUnit::new("p.fe", source)], &r)
            .unwrap()
            .into_bytes()
    };
    assert_ne!(build(1), build(2));
}

// ---------------------------------------------------------------------------
// Integrity
// ---------------------------------------------------------------------------

#[test]
fn a_flipped_payload_byte_is_caught_by_the_content_hash() {
    let mut bytes = compiled();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    assert_eq!(
        ProcedureDatabase::from_bytes(&bytes).unwrap_err(),
        FormatError::ChecksumMismatch
    );
}

#[test]
fn trailing_host_data_is_ignored() {
    // `total_size` bounds the database, so a host may concatenate its own
    // payload after it and hand over the whole slice.
    let mut bytes = compiled();
    let original = bytes.len();
    bytes.extend_from_slice(b"host payload follows here");
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    assert_eq!(db.size_bytes(), original);
    assert_eq!(db.as_bytes().len(), original);
}

// ---------------------------------------------------------------------------
// Corruption matrix
// ---------------------------------------------------------------------------

#[test]
fn empty_and_tiny_inputs() {
    assert_eq!(
        ProcedureDatabase::from_bytes(&[]).unwrap_err(),
        FormatError::TooSmall
    );
    assert_eq!(
        ProcedureDatabase::from_bytes(b"FEBC").unwrap_err(),
        FormatError::TooSmall
    );
    let bytes = compiled();
    assert_eq!(
        ProcedureDatabase::from_bytes(&bytes[..format::HEADER_SIZE - 1]).unwrap_err(),
        FormatError::TooSmall
    );
}

#[test]
fn bad_magic() {
    let error = rejects("magic", |b| b[0] = b'X');
    assert_eq!(error, FormatError::BadMagic);
}

#[test]
fn unsupported_version() {
    let error = rejects("version", |b| {
        b[header::FORMAT_VERSION..header::FORMAT_VERSION + 2].copy_from_slice(&99u16.to_le_bytes());
    });
    assert!(matches!(error, FormatError::UnsupportedVersion { .. }));
}

#[test]
fn truncation_at_every_length_is_rejected_not_panicked() {
    // Whole-file truncation is what a failed download looks like.
    let bytes = compiled();
    for len in 0..bytes.len() {
        let result = ProcedureDatabase::from_bytes(&bytes[..len]);
        assert!(result.is_err(), "truncation to {len} bytes was accepted");
    }
    assert!(ProcedureDatabase::from_bytes(&bytes).is_ok());
}

#[test]
fn total_size_larger_than_the_slice() {
    let error = rejects("total_size", |b| {
        let len = b.len() as u32;
        put_u32(b, header::TOTAL_SIZE, len + 4096);
    });
    assert_eq!(error, FormatError::BadHeader);
}

#[test]
fn header_size_smaller_than_the_header() {
    let error = rejects("header_size", |b| {
        b[header::HEADER_SIZE..header::HEADER_SIZE + 2].copy_from_slice(&8u16.to_le_bytes());
    });
    assert_eq!(error, FormatError::BadHeader);
}

#[test]
fn counts_beyond_what_the_bytecode_can_address() {
    let error = rejects("proc_count", |b| {
        put_u32(b, header::PROC_COUNT, u32::MAX);
    });
    assert_eq!(error, FormatError::BadHeader);
}

#[test]
fn a_section_that_runs_past_the_end() {
    let error = rejects("proc_offset", |b| {
        let len = b.len() as u32;
        put_u32(b, header::PROC_OFFSET, len - 8);
    });
    assert!(matches!(error, FormatError::BadSection { .. }));
}

#[test]
fn a_misaligned_section_offset() {
    let error = rejects("alignment", |b| {
        let offset = u32_at(b, header::SYMBOL_OFFSET);
        put_u32(b, header::SYMBOL_OFFSET, offset + 1);
    });
    assert!(matches!(error, FormatError::BadSection { .. }));
}

#[test]
fn an_offset_that_would_overflow_when_added_to_its_count() {
    let error = rejects("overflow", |b| {
        put_u32(b, header::CONTROL_OFFSET, u32::MAX - 3);
        put_u32(b, header::CONTROL_COUNT, 1000);
    });
    assert!(matches!(error, FormatError::BadSection { .. }));
}

#[test]
fn a_non_monotonic_string_index() {
    let error = rejects("string index", |b| {
        let index = u32_at(b, header::STRING_INDEX_OFFSET) as usize;
        // Second entry points before the first.
        put_u32(b, index + 4, 0);
        put_u32(b, index, 8);
    });
    assert_eq!(error, FormatError::BadStringTable);
}

#[test]
fn a_string_index_that_leaves_the_blob() {
    let error = rejects("string bounds", |b| {
        let index = u32_at(b, header::STRING_INDEX_OFFSET) as usize;
        put_u32(b, index + 4, u32::MAX);
    });
    assert_eq!(error, FormatError::BadStringTable);
}

#[test]
fn invalid_utf8_in_the_string_blob() {
    let error = rejects("utf-8", |b| {
        let blob = u32_at(b, header::STRING_BLOB_OFFSET) as usize;
        b[blob] = 0xFF;
    });
    assert_eq!(error, FormatError::BadStringTable);
}

#[test]
fn a_record_that_names_a_missing_string() {
    let error = rejects("string reference", |b| {
        let proc_offset = u32_at(b, header::PROC_OFFSET) as usize;
        put_u32(b, proc_offset + format::proc_rec::NAME_STR, 100_000);
    });
    assert_eq!(error, FormatError::BadReference);
}

#[test]
fn procedures_out_of_order_are_rejected() {
    // The reader binary-searches, so an unsorted table would make lookups
    // silently wrong rather than merely slow.
    let error = rejects("order", |b| {
        let proc_offset = u32_at(b, header::PROC_OFFSET) as usize;
        let size = format::PROC_RECORD_SIZE;
        let (a, c) = (proc_offset, proc_offset + size);
        for i in 0..size {
            b.swap(a + i, c + i);
        }
    });
    assert_eq!(error, FormatError::BadProcedureOrder);
}

#[test]
fn a_procedure_whose_code_runs_past_the_code_section() {
    let error = rejects("code bounds", |b| {
        let proc_offset = u32_at(b, header::PROC_OFFSET) as usize;
        put_u32(b, proc_offset + format::proc_rec::CODE_LEN, 1_000_000);
    });
    assert!(matches!(error, FormatError::BadSection { .. }));
}

#[test]
fn an_unknown_opcode() {
    let error = rejects("opcode", |b| {
        let code = u32_at(b, header::CODE_OFFSET) as usize;
        b[code] = 0xEE;
    });
    assert!(
        matches!(
            error,
            FormatError::BadBytecode {
                kind: fe_runtime::BytecodeError::UnknownOpcode(0xEE),
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn a_backward_jump_is_rejected() {
    // This is the property the whole design leans on: if a jump could go
    // backwards, a tick could loop, and a looping tick is a frozen aircraft.
    let error = rejects("backward jump", |b| {
        let code = u32_at(b, header::CODE_OFFSET) as usize;
        let len = u32_at(b, header::CODE_LEN) as usize;
        let at = find_opcode(&b[code..code + len], format::op::JUMP_IF_FALSE)
            .expect("examples contain a conditional jump");
        put_u32(b, code + at + 1, 0);
    });
    assert!(
        matches!(
            error,
            FormatError::BadBytecode {
                kind: fe_runtime::BytecodeError::BadJumpTarget,
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn a_jump_into_the_middle_of_an_instruction_is_rejected() {
    let error = rejects("misaligned jump", |b| {
        let code = u32_at(b, header::CODE_OFFSET) as usize;
        let len = u32_at(b, header::CODE_LEN) as usize;
        let at = find_opcode(&b[code..code + len], format::op::JUMP_IF_FALSE).unwrap();
        let target = u32_at(b, code + at + 1);
        put_u32(b, code + at + 1, target + 1);
    });
    assert!(
        matches!(error, FormatError::BadBytecode { .. }),
        "{error:?}"
    );
}

#[test]
fn an_await_without_its_test_is_rejected() {
    let error = rejects("await pairing", |b| {
        let code = u32_at(b, header::CODE_OFFSET) as usize;
        let len = u32_at(b, header::CODE_LEN) as usize;
        let at = find_opcode(&b[code..code + len], format::op::AWAIT).unwrap();
        // Claim a different body length, so AWAIT_TEST is no longer where the
        // instruction says it is.
        b[code + at + 1] = b[code + at + 1].wrapping_add(1);
    });
    assert!(
        matches!(error, FormatError::BadBytecode { .. }),
        "{error:?}"
    );
}

#[test]
fn an_operand_naming_a_missing_control_is_rejected() {
    let error = rejects("control operand", |b| {
        let code = u32_at(b, header::CODE_OFFSET) as usize;
        let len = u32_at(b, header::CODE_LEN) as usize;
        let at = find_opcode(&b[code..code + len], format::op::CHECK).unwrap();
        b[code + at + 1..code + at + 3].copy_from_slice(&9999u16.to_le_bytes());
    });
    assert!(
        matches!(
            error,
            FormatError::BadBytecode {
                kind: fe_runtime::BytecodeError::BadOperand,
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn a_body_that_does_not_end_with_end_is_rejected() {
    let error = rejects("missing end", |b| {
        let proc_offset = u32_at(b, header::PROC_OFFSET) as usize;
        let code_len = u32_at(b, proc_offset + format::proc_rec::CODE_LEN);
        put_u32(b, proc_offset + format::proc_rec::CODE_LEN, code_len - 1);
    });
    assert!(
        matches!(error, FormatError::BadBytecode { .. }),
        "{error:?}"
    );
}

/// Find the first offset holding `opcode` while decoding properly, so we do not
/// accidentally match an immediate operand byte.
fn find_opcode(code: &[u8], opcode: u8) -> Option<usize> {
    let mut at = 0usize;
    while at < code.len() {
        if code[at] == opcode {
            return Some(at);
        }
        at += format::instruction_len(code[at])?;
    }
    None
}
