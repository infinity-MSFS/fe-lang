//! Shared test scaffolding: the example DC-10 registry and a fake aircraft.
//!
//! This is what an aircraft's build tooling and systems code would provide for
//! real — the registry next to the build, the traits next to the simulation.

#![allow(dead_code)]

use std::collections::BTreeMap;

use fe_compiler::{ControlSpec, SourceUnit, SymbolRegistry, ValueType};
use fe_runtime::{
    Action, ActionResult, AircraftControls, AircraftState, Control, ControlValue, ProcedureEvent,
    Symbol, Value,
};

// Host tags. In a real aircraft these would be a generated enum shared with
// the systems code.
pub mod tag {
    pub const HYD1_PRESSURE: u32 = 10;
    pub const HYD2_PRESSURE: u32 = 11;
    pub const HYD3_PRESSURE: u32 = 12;
    pub const HYD2_ELECTRIC_PUMP_RUNNING: u32 = 13;
    pub const ENG2_RUNNING: u32 = 20;
    pub const AC_BUS1_POWERED: u32 = 30;
    pub const AC_BUS2_POWERED: u32 = 31;
    pub const GEN2_AVAILABLE: u32 = 32;
    pub const FUEL_IMBALANCE: u32 = 40;
    pub const FUEL_TANK1: u32 = 41;
    pub const FUEL_TANK3: u32 = 43;
    pub const FUEL_CROSSFEED_OPEN: u32 = 44;

    pub const HYD1_ENGINE_PUMP: u32 = 100;
    pub const HYD2_ENGINE_PUMP: u32 = 101;
    pub const HYD3_ENGINE_PUMP: u32 = 102;
    pub const HYD2_ELECTRIC_PUMP: u32 = 103;
    pub const HYD2_ISOLATION_VALVE: u32 = 104;
    pub const GEN2_FIELD: u32 = 110;
    pub const BUS_TIE_1_2: u32 = 111;
    pub const FUEL_XFEED_SELECTOR: u32 = 120;
    pub const FUEL_CROSSFEED_VALVE: u32 = 121;
    pub const FUEL_QUANTITY_INDICATION: u32 = 122;
    pub const FUEL_PUMP_PRESSURE_TARGET: u32 = 123;

    pub const CABIN_ALTITUDE: u32 = 50;
    pub const CABIN_RATE: u32 = 51;
    pub const ON_GROUND: u32 = 52;

    pub const OUTFLOW_FWD: u32 = 130;
    pub const OUTFLOW_AFT: u32 = 131;
    pub const PACK_FLOW: u32 = 132;
    pub const OXYGEN_INDICATION: u32 = 133;
}

/// The registry the example procedures are written against.
pub fn registry() -> SymbolRegistry {
    let mut r = SymbolRegistry::new();

    r.define_state("hydraulic.1.pressure", ValueType::F32, tag::HYD1_PRESSURE)
        .unwrap();
    r.define_state("hydraulic.2.pressure", ValueType::F32, tag::HYD2_PRESSURE)
        .unwrap();
    r.define_state("hydraulic.3.pressure", ValueType::F32, tag::HYD3_PRESSURE)
        .unwrap();
    r.define_state(
        "hydraulic.2.electric_pump_running",
        ValueType::Bool,
        tag::HYD2_ELECTRIC_PUMP_RUNNING,
    )
    .unwrap();
    r.define_state("engine.2.running", ValueType::Bool, tag::ENG2_RUNNING)
        .unwrap();
    r.define_state(
        "electrical.ac_bus.1.powered",
        ValueType::Bool,
        tag::AC_BUS1_POWERED,
    )
    .unwrap();
    r.define_state(
        "electrical.ac_bus.2.powered",
        ValueType::Bool,
        tag::AC_BUS2_POWERED,
    )
    .unwrap();
    r.define_state(
        "generator.2.available",
        ValueType::Bool,
        tag::GEN2_AVAILABLE,
    )
    .unwrap();
    r.define_state("fuel.imbalance_1_3", ValueType::F32, tag::FUEL_IMBALANCE)
        .unwrap();
    r.define_state("fuel.tank.1.quantity", ValueType::F32, tag::FUEL_TANK1)
        .unwrap();
    r.define_state("fuel.tank.3.quantity", ValueType::F32, tag::FUEL_TANK3)
        .unwrap();
    r.define_state(
        "fuel.crossfeed_open",
        ValueType::Bool,
        tag::FUEL_CROSSFEED_OPEN,
    )
    .unwrap();

    r.define_state("cabin.altitude", ValueType::F32, tag::CABIN_ALTITUDE)
        .unwrap();
    r.define_state("cabin.rate_of_climb", ValueType::F32, tag::CABIN_RATE)
        .unwrap();
    r.define_state("aircraft.on_ground", ValueType::Bool, tag::ON_GROUND)
        .unwrap();

    r.define_control(
        "HYD_1_ENGINE_PUMP",
        ControlSpec::checklist(),
        tag::HYD1_ENGINE_PUMP,
    )
    .unwrap();
    r.define_control(
        "HYD_2_ENGINE_PUMP",
        ControlSpec::checklist(),
        tag::HYD2_ENGINE_PUMP,
    )
    .unwrap();
    r.define_control(
        "HYD_3_ENGINE_PUMP",
        ControlSpec::checklist(),
        tag::HYD3_ENGINE_PUMP,
    )
    .unwrap();
    r.define_control(
        "HYD_2_ELECTRIC_PUMP",
        ControlSpec::switch(),
        tag::HYD2_ELECTRIC_PUMP,
    )
    .unwrap();
    r.define_control(
        "HYD_2_ISOLATION_VALVE",
        ControlSpec::valve(),
        tag::HYD2_ISOLATION_VALVE,
    )
    .unwrap();
    r.define_control("GEN_2_FIELD", ControlSpec::switch(), tag::GEN2_FIELD)
        .unwrap();
    r.define_control("BUS_TIE_1_2", ControlSpec::valve(), tag::BUS_TIE_1_2)
        .unwrap();
    r.define_control(
        "FUEL_XFEED_SELECTOR",
        ControlSpec::selector(["OFF", "TANK_1_TO_3", "TANK_3_TO_1"]),
        tag::FUEL_XFEED_SELECTOR,
    )
    .unwrap();
    r.define_control(
        "FUEL_CROSSFEED_VALVE",
        ControlSpec::valve(),
        tag::FUEL_CROSSFEED_VALVE,
    )
    .unwrap();
    r.define_control(
        "FUEL_QUANTITY_INDICATION",
        ControlSpec::checklist(),
        tag::FUEL_QUANTITY_INDICATION,
    )
    .unwrap();
    r.define_control(
        "FUEL_PUMP_PRESSURE_TARGET",
        ControlSpec::analog(0.0, 50.0),
        tag::FUEL_PUMP_PRESSURE_TARGET,
    )
    .unwrap();

    r.define_control(
        "OUTFLOW_VALVE_FORWARD",
        ControlSpec::valve(),
        tag::OUTFLOW_FWD,
    )
    .unwrap();
    r.define_control("OUTFLOW_VALVE_AFT", ControlSpec::valve(), tag::OUTFLOW_AFT)
        .unwrap();
    r.define_control(
        "PACK_FLOW_SELECTOR",
        ControlSpec::selector(["OFF", "NORMAL", "HIGH"]),
        tag::PACK_FLOW,
    )
    .unwrap();
    r.define_control(
        "OXYGEN_PRESSURE_INDICATION",
        ControlSpec::checklist(),
        tag::OXYGEN_INDICATION,
    )
    .unwrap();

    r
}

/// The example sources, as the build tooling would load them.
pub fn example_units() -> Vec<SourceUnit> {
    vec![
        SourceUnit::new(
            "hydraulic.fe",
            include_str!("../../../examples/dc10/hydraulic.fe"),
        ),
        SourceUnit::new(
            "electrical.fe",
            include_str!("../../../examples/dc10/electrical.fe"),
        ),
        SourceUnit::new("fuel.fe", include_str!("../../../examples/dc10/fuel.fe")),
        SourceUnit::new(
            "pressurization.fe",
            include_str!("../../../examples/dc10/pressurization.fe"),
        ),
    ]
}

/// Compile the examples or panic with rendered diagnostics.
pub fn compile_examples() -> Vec<u8> {
    let units = example_units();
    match fe_compiler::compile(&units, &registry()) {
        Ok(compiled) => compiled.into_bytes(),
        Err(error) => panic!("{}", error.render(&units)),
    }
}

/// Compile arbitrary source against the example registry.
pub fn compile_source(source: &str) -> Result<fe_compiler::Compiled, String> {
    let units = vec![SourceUnit::new("test.fe", source)];
    fe_compiler::compile(&units, &registry()).map_err(|e| e.render(&units))
}

/// Compile and expect failure, returning the rendered diagnostics.
pub fn expect_errors(source: &str) -> String {
    match compile_source(source) {
        Ok(_) => panic!("expected compilation to fail:\n{source}"),
        Err(rendered) => rendered,
    }
}

// ---------------------------------------------------------------------------
// A fake aircraft
// ---------------------------------------------------------------------------

/// Stand-in for the aircraft simulation. State is whatever the test says it
/// is; the procedure engine has no opinion about how it got that way.
#[derive(Clone, Debug, Default)]
pub struct FakeAircraft {
    values: BTreeMap<u32, Value>,
}

impl FakeAircraft {
    pub fn new() -> FakeAircraft {
        let mut aircraft = FakeAircraft::default();
        aircraft.set(tag::HYD1_PRESSURE, Value::F32(3000.0));
        aircraft.set(tag::HYD2_PRESSURE, Value::F32(3000.0));
        aircraft.set(tag::HYD3_PRESSURE, Value::F32(3000.0));
        aircraft.set(tag::HYD2_ELECTRIC_PUMP_RUNNING, Value::Bool(false));
        aircraft.set(tag::ENG2_RUNNING, Value::Bool(true));
        aircraft.set(tag::AC_BUS1_POWERED, Value::Bool(true));
        aircraft.set(tag::AC_BUS2_POWERED, Value::Bool(true));
        aircraft.set(tag::GEN2_AVAILABLE, Value::Bool(true));
        aircraft.set(tag::FUEL_IMBALANCE, Value::F32(0.0));
        aircraft.set(tag::FUEL_TANK1, Value::F32(20000.0));
        aircraft.set(tag::FUEL_TANK3, Value::F32(20000.0));
        aircraft.set(tag::FUEL_CROSSFEED_OPEN, Value::Bool(false));
        aircraft.set(tag::CABIN_ALTITUDE, Value::F32(6500.0));
        aircraft.set(tag::CABIN_RATE, Value::F32(0.0));
        aircraft.set(tag::ON_GROUND, Value::Bool(false));
        aircraft
    }

    pub fn set(&mut self, tag: u32, value: Value) {
        self.values.insert(tag, value);
    }

    pub fn set_f32(&mut self, tag: u32, value: f32) {
        self.set(tag, Value::F32(value));
    }

    pub fn set_bool(&mut self, tag: u32, value: bool) {
        self.set(tag, Value::Bool(value));
    }
}

impl AircraftState for FakeAircraft {
    fn read(&self, symbol: Symbol<'_>) -> Value {
        match self.values.get(&symbol.tag) {
            Some(value) => *value,
            // A real aircraft would never be missing a registered symbol; the
            // fake returns a typed default rather than panicking.
            None => match symbol.ty {
                fe_runtime::ValueType::Bool => Value::Bool(false),
                fe_runtime::ValueType::F32 => Value::F32(0.0),
            },
        }
    }
}

/// Records everything the procedure asked for.
#[derive(Debug, Default)]
pub struct Recorder {
    pub actions: Vec<String>,
    pub notifications: Vec<String>,
    pub events: Vec<String>,
    /// Controls the host refuses, by name.
    pub reject: Vec<String>,
    pub completed: bool,
    pub failed: Option<String>,
}

impl Recorder {
    pub fn new() -> Recorder {
        Recorder::default()
    }

    pub fn reject(mut self, control: &str) -> Recorder {
        self.reject.push(control.to_string());
        self
    }

    fn describe(action: &Action<'_>) -> String {
        match action {
            Action::Set { control, value } => match value {
                ControlValue::Position { name, .. } => format!("set {} = {}", control.name, name),
                ControlValue::Analog(v) => format!("set {} = {}", control.name, v),
            },
            Action::Check { control } => format!("check {}", control.name),
        }
    }

    pub fn last_action(&self) -> Option<&str> {
        self.actions.last().map(String::as_str)
    }
}

impl AircraftControls for Recorder {
    fn execute(&mut self, action: Action<'_>) -> ActionResult {
        self.actions.push(Recorder::describe(&action));
        let control: Control<'_> = action.control();
        if self.reject.iter().any(|name| name == control.name) {
            return ActionResult::Rejected(7);
        }
        ActionResult::Accepted
    }

    fn on_event(&mut self, event: ProcedureEvent<'_>) {
        match event {
            ProcedureEvent::Notification { message } => {
                self.notifications.push(message.to_string());
                self.events.push(format!("notify: {message}"));
            }
            ProcedureEvent::Completed => {
                self.completed = true;
                self.events.push("completed".to_string());
            }
            ProcedureEvent::Failed { reason } => {
                self.failed = Some(format!("{reason:?}"));
                self.events.push(format!("failed: {reason:?}"));
            }
            ProcedureEvent::Waiting { elapsed_ms, .. } => {
                self.events.push(format!("waiting {elapsed_ms}ms"));
            }
            ProcedureEvent::Timeout { continued } => {
                self.events.push(format!("timeout (continued={continued})"));
            }
            ProcedureEvent::Started { procedure } => {
                self.events.push(format!("started {}", procedure.id));
            }
            ProcedureEvent::Entered { procedure } => {
                self.events.push(format!("entered {}", procedure.id));
            }
            ProcedureEvent::Returned { procedure } => {
                self.events.push(format!("returned {}", procedure.id));
            }
            ProcedureEvent::ActionRequested { .. } => {}
        }
    }
}
