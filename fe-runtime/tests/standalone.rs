use fe_runtime::format::{self, flags, header, op, proc_rec};
use fe_runtime::host::{Action, ActionResult, AircraftControls, AircraftState, ProcedureEvent};
use fe_runtime::{
    BytecodeError, ControlKind, FormatError, ProcedureDatabase, ProcedureExecutor, Symbol, Tick,
    Value, ValueType,
};

// ---------------------------------------------------------------------------
// A hand-rolled database builder
// ---------------------------------------------------------------------------

/// Everything the builder needs to know. Kept deliberately dumb: it writes
/// exactly what it is told, including nonsense, so tests can produce files the
/// compiler would never emit.
struct Builder {
    strings: Vec<&'static str>,
    /// (name string, type, tag)
    symbols: Vec<(u32, ValueType, u32)>,
    /// (name string, kind, first position, position count, tag)
    controls: Vec<(u32, ControlKind, u16, u8, u32)>,
    /// String ids, in control order.
    positions: Vec<u32>,
    /// (id string, name string, body, trigger)
    procedures: Vec<(u32, u32, Vec<u8>, Vec<u8>)>,
}

fn align(n: usize) -> usize {
    (n + 3) & !3
}

fn put_u16(out: &mut [u8], at: usize, v: u16) {
    out[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

fn put_u32(out: &mut [u8], at: usize, v: u32) {
    out[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

impl Builder {
    fn build(&self) -> Vec<u8> {
        let mut code = Vec::new();
        let mut placement = Vec::new();
        for (_, _, body, trigger) in &self.procedures {
            let body_off = code.len() as u32;
            code.extend_from_slice(body);
            let trigger_off = code.len() as u32;
            code.extend_from_slice(trigger);
            placement.push((
                body_off,
                body.len() as u32,
                trigger_off,
                trigger.len() as u32,
            ));
        }

        let mut blob = Vec::new();
        let mut index = Vec::new();
        for s in &self.strings {
            index.push(blob.len() as u32);
            blob.extend_from_slice(s.as_bytes());
        }
        index.push(blob.len() as u32);

        let mut at = format::HEADER_SIZE;
        let proc_offset = at;
        at += self.procedures.len() * format::PROC_RECORD_SIZE;
        let symbol_offset = align(at);
        at = symbol_offset + self.symbols.len() * format::SYMBOL_RECORD_SIZE;
        let control_offset = align(at);
        at = control_offset + self.controls.len() * format::CONTROL_RECORD_SIZE;
        let position_offset = align(at);
        at = position_offset + self.positions.len() * 4;
        let index_offset = align(at);
        at = index_offset + index.len() * 4;
        let blob_offset = align(at);
        at = blob_offset + blob.len();
        let code_offset = align(at);
        let total = code_offset + code.len();

        let mut out = vec![0u8; total];
        out[0..4].copy_from_slice(&format::MAGIC);
        put_u16(&mut out, header::FORMAT_VERSION, format::FORMAT_VERSION);
        put_u16(&mut out, header::HEADER_SIZE, format::HEADER_SIZE as u16);
        put_u32(&mut out, header::TOTAL_SIZE, total as u32);
        put_u32(&mut out, header::FLAGS, flags::CONTENT_HASH);
        put_u32(&mut out, header::PROC_COUNT, self.procedures.len() as u32);
        put_u32(&mut out, header::PROC_OFFSET, proc_offset as u32);
        put_u32(&mut out, header::SYMBOL_COUNT, self.symbols.len() as u32);
        put_u32(&mut out, header::SYMBOL_OFFSET, symbol_offset as u32);
        put_u32(&mut out, header::CONTROL_COUNT, self.controls.len() as u32);
        put_u32(&mut out, header::CONTROL_OFFSET, control_offset as u32);
        put_u32(
            &mut out,
            header::POSITION_COUNT,
            self.positions.len() as u32,
        );
        put_u32(&mut out, header::POSITION_OFFSET, position_offset as u32);
        put_u32(&mut out, header::STRING_COUNT, self.strings.len() as u32);
        put_u32(&mut out, header::STRING_INDEX_OFFSET, index_offset as u32);
        put_u32(&mut out, header::STRING_BLOB_OFFSET, blob_offset as u32);
        put_u32(&mut out, header::STRING_BLOB_LEN, blob.len() as u32);
        put_u32(&mut out, header::CODE_OFFSET, code_offset as u32);
        put_u32(&mut out, header::CODE_LEN, code.len() as u32);

        for (i, (id, name, _, _)) in self.procedures.iter().enumerate() {
            let at = proc_offset + i * format::PROC_RECORD_SIZE;
            let (body_off, body_len, trigger_off, trigger_len) = placement[i];
            put_u32(&mut out, at + proc_rec::ID_STR, *id);
            put_u32(&mut out, at + proc_rec::NAME_STR, *name);
            put_u32(&mut out, at + proc_rec::DESC_STR, format::NO_STRING);
            put_u32(&mut out, at + proc_rec::CODE_OFF, body_off);
            put_u32(&mut out, at + proc_rec::CODE_LEN, body_len);
            put_u32(&mut out, at + proc_rec::TRIGGER_OFF, trigger_off);
            put_u32(&mut out, at + proc_rec::TRIGGER_LEN, trigger_len);
            out[at + proc_rec::CATEGORY] = 1;
            out[at + proc_rec::PRIORITY] = 50;
            put_u16(&mut out, at + proc_rec::REVISION, 7);
        }
        for (i, (name, ty, tag)) in self.symbols.iter().enumerate() {
            let at = symbol_offset + i * format::SYMBOL_RECORD_SIZE;
            put_u32(&mut out, at, *name);
            put_u32(&mut out, at + 4, *tag);
            out[at + 8] = *ty as u8;
        }
        for (i, (name, kind, first, count, tag)) in self.controls.iter().enumerate() {
            let at = control_offset + i * format::CONTROL_RECORD_SIZE;
            put_u32(&mut out, at, *name);
            put_u32(&mut out, at + 4, *tag);
            out[at + 8] = *kind as u8;
            out[at + 9] = *count;
            put_u16(&mut out, at + 10, *first);
        }
        for (i, s) in self.positions.iter().enumerate() {
            put_u32(&mut out, position_offset + i * 4, *s);
        }
        for (i, o) in index.iter().enumerate() {
            put_u32(&mut out, index_offset + i * 4, *o);
        }
        out[blob_offset..blob_offset + blob.len()].copy_from_slice(&blob);
        out[code_offset..code_offset + code.len()].copy_from_slice(&code);

        let hash = format::fnv1a32(&out[format::HEADER_SIZE..]);
        put_u32(&mut out, header::CONTENT_HASH, hash);
        out
    }
}

/// A database with one bool symbol, one switch, and one procedure that turns
/// the switch on when the symbol is true.
///
/// Body:
/// ```text
/// 0000  LOAD_BOOL #0
/// 0003  JUMP_IF_FALSE @0013
/// 0008  SET_POSITION #0 1
/// 0012  COMPLETE
/// 0013  END
/// ```
fn minimal() -> Builder {
    let body = {
        let mut c = Vec::new();
        c.push(op::LOAD_BOOL);
        c.extend_from_slice(&0u16.to_le_bytes());
        c.push(op::JUMP_IF_FALSE);
        c.extend_from_slice(&13u32.to_le_bytes());
        c.push(op::SET_POSITION);
        c.extend_from_slice(&0u16.to_le_bytes());
        c.push(1);
        c.push(op::COMPLETE);
        c.push(op::END);
        assert_eq!(c.len(), 14);
        c
    };
    let trigger = {
        let mut c = Vec::new();
        c.push(op::LOAD_BOOL);
        c.extend_from_slice(&0u16.to_le_bytes());
        c
    };
    Builder {
        strings: vec!["PROC", "A Procedure", "armed", "MASTER", "OFF", "ON"],
        symbols: vec![(2, ValueType::Bool, 900)],
        controls: vec![(3, ControlKind::Switch, 0, 2, 901)],
        positions: vec![4, 5],
        procedures: vec![(0, 1, body, trigger)],
    }
}

// ---------------------------------------------------------------------------
// A host
// ---------------------------------------------------------------------------

struct Host {
    armed: bool,
    actions: Vec<String>,
    completed: bool,
}

impl AircraftState for Host {
    fn read(&self, symbol: Symbol<'_>) -> Value {
        assert_eq!(symbol.tag, 900);
        Value::Bool(self.armed)
    }
}

impl AircraftControls for Host {
    fn execute(&mut self, action: Action<'_>) -> ActionResult {
        self.actions.push(format!("{:?}", action.control().name));
        ActionResult::Accepted
    }

    fn on_event(&mut self, event: ProcedureEvent<'_>) {
        if matches!(event, ProcedureEvent::Completed) {
            self.completed = true;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn a_hand_built_database_loads() {
    let bytes = minimal().build();
    let db = ProcedureDatabase::from_bytes(&bytes).expect("hand-built database should load");
    assert_eq!(db.procedure_count(), 1);
    assert_eq!(db.symbol_count(), 1);
    assert_eq!(db.control_count(), 1);

    let p = db.get_procedure("PROC").unwrap();
    assert_eq!(p.name, "A Procedure");
    assert_eq!(p.priority, 50);
    assert_eq!(p.revision, 7);
    assert_eq!(p.category, fe_runtime::Category::Abnormal);
    assert!(p.has_trigger());

    let control = db.controls().next().unwrap();
    assert_eq!(control.name, "MASTER");
    assert_eq!(db.position_name(&control, 1), Some("ON"));
}

#[test]
fn a_hand_built_database_executes() {
    let bytes = minimal().build();
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let p = db.get_procedure("PROC").unwrap();

    let mut host = Host {
        armed: true,
        actions: Vec::new(),
        completed: false,
    };
    assert_eq!(
        ProcedureExecutor::evaluate_trigger(&p, &host).unwrap(),
        Some(true)
    );

    let mut exec = ProcedureExecutor::new(p);
    let mut controls = Host {
        armed: true,
        actions: Vec::new(),
        completed: false,
    };
    assert_eq!(exec.tick(&host, &mut controls, 50), Tick::Completed);
    assert_eq!(controls.actions, vec!["\"MASTER\"".to_string()]);
    assert!(controls.completed);

    // The false branch skips the action entirely.
    host.armed = false;
    let mut exec = ProcedureExecutor::new(db.get_procedure("PROC").unwrap());
    let mut controls = Host {
        armed: false,
        actions: Vec::new(),
        completed: false,
    };
    assert_eq!(exec.tick(&host, &mut controls, 50), Tick::Completed);
    assert!(controls.actions.is_empty());
}

#[test]
fn a_backward_jump_is_refused_at_load() {
    let mut builder = minimal();
    // Point the conditional jump at itself.
    let body = &mut builder.procedures[0].2;
    body[4..8].copy_from_slice(&3u32.to_le_bytes());
    let bytes = builder.build();
    match ProcedureDatabase::from_bytes(&bytes) {
        Err(FormatError::BadBytecode { kind, .. }) => {
            assert_eq!(kind, BytecodeError::BadJumpTarget)
        }
        other => panic!("expected a jump-target error, got {other:?}"),
    }
}

#[test]
fn a_body_that_does_not_end_with_end_is_refused() {
    let mut builder = minimal();
    let body = &mut builder.procedures[0].2;
    let last = body.len() - 1;
    body[last] = op::NOP;
    let bytes = builder.build();
    match ProcedureDatabase::from_bytes(&bytes) {
        Err(FormatError::BadBytecode { kind, .. }) => {
            assert_eq!(kind, BytecodeError::MissingEnd)
        }
        other => panic!("expected a missing-END error, got {other:?}"),
    }
}

#[test]
fn an_unbalanced_stack_is_refused() {
    // A LOAD with nothing consuming it: the stack is not empty at the next
    // statement boundary.
    let mut builder = minimal();
    let mut body = Vec::new();
    body.push(op::LOAD_BOOL);
    body.extend_from_slice(&0u16.to_le_bytes());
    body.push(op::COMPLETE);
    body.push(op::END);
    builder.procedures[0].2 = body;
    let bytes = builder.build();
    match ProcedureDatabase::from_bytes(&bytes) {
        Err(FormatError::BadBytecode { kind, .. }) => {
            assert_eq!(kind, BytecodeError::StackDiscipline)
        }
        other => panic!("expected a stack error, got {other:?}"),
    }
}

#[test]
fn a_type_confused_comparison_is_refused() {
    // Compare a bool against a float. The compiler would never emit this; a
    // hand-edited file might.
    let mut builder = minimal();
    let mut body = Vec::new();
    body.push(op::LOAD_BOOL);
    body.extend_from_slice(&0u16.to_le_bytes());
    body.push(op::PUSH_F32);
    body.extend_from_slice(&1.0f32.to_bits().to_le_bytes());
    body.push(op::LT);
    body.push(op::REQUIRE);
    body.extend_from_slice(&format::NO_STRING.to_le_bytes());
    body.push(op::END);
    builder.procedures[0].2 = body;
    let bytes = builder.build();
    assert!(matches!(
        ProcedureDatabase::from_bytes(&bytes),
        Err(FormatError::BadBytecode { .. })
    ));
}

#[test]
fn an_await_without_its_test_is_refused() {
    let mut builder = minimal();
    let mut body = Vec::new();
    body.push(op::AWAIT);
    body.extend_from_slice(&3u16.to_le_bytes());
    body.extend_from_slice(&1000u32.to_le_bytes());
    body.push(format::on_timeout::CONTINUE);
    body.push(op::LOAD_BOOL);
    body.extend_from_slice(&0u16.to_le_bytes());
    // AWAIT_TEST omitted.
    body.push(op::END);
    builder.procedures[0].2 = body;
    let bytes = builder.build();
    match ProcedureDatabase::from_bytes(&bytes) {
        Err(FormatError::BadBytecode { kind, .. }) => {
            assert!(
                matches!(
                    kind,
                    BytecodeError::BadWait | BytecodeError::StackDiscipline
                ),
                "unexpected kind {kind:?}"
            )
        }
        other => panic!("expected a wait error, got {other:?}"),
    }
}

#[test]
fn a_wait_round_trips_through_a_hand_built_database() {
    // AWAIT / condition / AWAIT_TEST, assembled by hand and executed.
    let mut builder = minimal();
    let mut body = Vec::new();
    body.push(op::AWAIT);
    body.extend_from_slice(&3u16.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.push(format::on_timeout::CONTINUE);
    body.push(op::LOAD_BOOL);
    body.extend_from_slice(&0u16.to_le_bytes());
    body.push(op::AWAIT_TEST);
    body.push(op::END);
    builder.procedures[0].2 = body;
    let bytes = builder.build();
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();

    let mut host = Host {
        armed: false,
        actions: Vec::new(),
        completed: false,
    };
    let mut controls = Host {
        armed: false,
        actions: Vec::new(),
        completed: false,
    };
    let mut exec = ProcedureExecutor::new(db.get_procedure("PROC").unwrap());
    for _ in 0..3 {
        assert!(matches!(
            exec.tick(&host, &mut controls, 100),
            Tick::Waiting { .. }
        ));
    }
    host.armed = true;
    assert_eq!(exec.tick(&host, &mut controls, 100), Tick::Completed);
}

#[test]
fn unknown_control_kinds_pass_through_rather_than_being_guessed() {
    // Forward compatibility: a newer compiler may introduce a control kind
    // this runtime does not know. It must not invent semantics for it — but a
    // `check` on it is still meaningful, so the database should load.
    let mut builder = minimal();
    builder.controls[0].1 = ControlKind::Unknown;
    let mut body = Vec::new();
    body.push(op::CHECK);
    body.extend_from_slice(&0u16.to_le_bytes());
    body.push(op::END);
    builder.procedures[0].2 = body;
    let bytes = builder.build();
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    assert_eq!(db.controls().next().unwrap().kind, ControlKind::Unknown);
}

#[test]
fn disassembly_of_a_hand_built_database_is_readable() {
    let bytes = minimal().build();
    let db = ProcedureDatabase::from_bytes(&bytes).unwrap();
    let p = db.get_procedure("PROC").unwrap();
    let mut text = String::new();
    fe_runtime::disassemble(&p, &mut text).unwrap();
    assert!(text.contains("procedure PROC"), "{text}");
    assert!(text.contains(".trigger"), "{text}");
    assert!(text.contains("JUMP_IF_FALSE  @0013"), "{text}");
    assert!(text.contains("MASTER = ON"), "{text}");
}
