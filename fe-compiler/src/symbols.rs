use std::collections::BTreeMap;

pub use fe_runtime::value::{ControlKind, ValueType};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryError {
    Duplicate { name: String },
    InvalidName { name: String, reason: &'static str },
    InvalidControl { name: String, reason: &'static str },
    TooMany { limit: usize },
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::Duplicate { name } => write!(f, "`{name}` is already registered"),
            RegistryError::InvalidName { name, reason } => {
                write!(f, "`{name}` is not a valid symbol name: {reason}")
            }
            RegistryError::InvalidControl { name, reason } => {
                write!(f, "control `{name}` is invalid: {reason}")
            }
            RegistryError::TooMany { limit } => {
                write!(f, "too many registered symbols (limit {limit})")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

#[derive(Clone, Debug, PartialEq)]
pub enum ControlSpec {
    Switch,
    Valve,
    Selector(Vec<String>),
    Analog { min: f32, max: f32 },
    Checklist,
}

impl ControlSpec {
    pub fn switch() -> ControlSpec {
        ControlSpec::Switch
    }

    pub fn valve() -> ControlSpec {
        ControlSpec::Valve
    }

    pub fn selector<I, S>(positions: I) -> ControlSpec
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        ControlSpec::Selector(positions.into_iter().map(Into::into).collect())
    }

    pub fn analog(min: f32, max: f32) -> ControlSpec {
        ControlSpec::Analog { min, max }
    }

    pub fn checklist() -> ControlSpec {
        ControlSpec::Checklist
    }

    pub fn positions(&self) -> Vec<&str> {
        match self {
            ControlSpec::Switch => vec!["OFF", "ON"],
            ControlSpec::Valve => vec!["CLOSED", "OPEN"],
            ControlSpec::Selector(list) => list.iter().map(String::as_str).collect(),
            ControlSpec::Analog { .. } | ControlSpec::Checklist => Vec::new(),
        }
    }

    pub fn position_index(&self, name: &str) -> Option<u8> {
        self.positions()
            .iter()
            .position(|p| p.eq_ignore_ascii_case(name))
            .map(|i| i as u8)
    }

    pub fn kind(&self) -> ControlKind {
        match self {
            ControlSpec::Switch => ControlKind::Switch,
            ControlSpec::Valve => ControlKind::Valve,
            ControlSpec::Selector(_) => ControlKind::Selector,
            ControlSpec::Analog { .. } => ControlKind::Analog,
            ControlSpec::Checklist => ControlKind::Checklist,
        }
    }

    fn validate(&self, name: &str) -> Result<(), RegistryError> {
        let invalid = |reason| RegistryError::InvalidControl {
            name: name.to_string(),
            reason,
        };
        match self {
            ControlSpec::Selector(positions) => {
                if positions.is_empty() {
                    return Err(invalid("a selector needs at least one position"));
                }
                if positions.len() > u8::MAX as usize {
                    return Err(invalid("a selector may have at most 255 positions"));
                }
                for (i, p) in positions.iter().enumerate() {
                    if p.is_empty() {
                        return Err(invalid("position names may not be empty"));
                    }
                    if positions[..i].iter().any(|q| q.eq_ignore_ascii_case(p)) {
                        return Err(invalid("position names must be unique"));
                    }
                }
                Ok(())
            }
            ControlSpec::Analog { min, max } => {
                if !min.is_finite() || !max.is_finite() {
                    return Err(invalid("analog limits must be finite"));
                }
                if min > max {
                    return Err(invalid("analog minimum exceeds maximum"));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct StateSymbol {
    pub name: String,
    pub ty: ValueType,
    pub tag: u32,
}

#[derive(Clone, Debug)]
pub struct ControlSymbol {
    pub name: String,
    pub spec: ControlSpec,
    pub tag: u32,
}

#[derive(Clone, Copy, Debug)]
pub enum Resolved<'a> {
    State(&'a StateSymbol),
    Control(&'a ControlSymbol),
}

#[derive(Clone, Debug, Default)]
pub struct SymbolRegistry {
    states: BTreeMap<String, StateSymbol>,
    controls: BTreeMap<String, ControlSymbol>,
}

const MAX_SYMBOLS: usize = u16::MAX as usize;

impl SymbolRegistry {
    pub fn new() -> SymbolRegistry {
        SymbolRegistry::default()
    }

    pub fn define_state(
        &mut self,
        name: impl Into<String>,
        ty: ValueType,
        tag: u32,
    ) -> Result<(), RegistryError> {
        let name = name.into();
        validate_name(&name)?;
        if self.states.len() >= MAX_SYMBOLS {
            return Err(RegistryError::TooMany { limit: MAX_SYMBOLS });
        }
        if self.contains(&name) {
            return Err(RegistryError::Duplicate { name });
        }
        self.states
            .insert(name.clone(), StateSymbol { name, ty, tag });
        Ok(())
    }

    pub fn define_control(
        &mut self,
        name: impl Into<String>,
        spec: ControlSpec,
        tag: u32,
    ) -> Result<(), RegistryError> {
        let name = name.into();
        validate_name(&name)?;
        spec.validate(&name)?;
        if self.controls.len() >= MAX_SYMBOLS {
            return Err(RegistryError::TooMany { limit: MAX_SYMBOLS });
        }
        if self.contains(&name) {
            return Err(RegistryError::Duplicate { name });
        }
        self.controls
            .insert(name.clone(), ControlSymbol { name, spec, tag });
        Ok(())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.states.contains_key(name) || self.controls.contains_key(name)
    }

    pub fn state(&self, name: &str) -> Option<&StateSymbol> {
        self.states.get(name)
    }

    pub fn control(&self, name: &str) -> Option<&ControlSymbol> {
        self.controls.get(name)
    }

    pub fn resolve(&self, name: &str) -> Option<Resolved<'_>> {
        if let Some(state) = self.states.get(name) {
            return Some(Resolved::State(state));
        }
        self.controls.get(name).map(Resolved::Control)
    }

    pub fn states(&self) -> impl Iterator<Item = &StateSymbol> {
        self.states.values()
    }

    pub fn controls(&self) -> impl Iterator<Item = &ControlSymbol> {
        self.controls.values()
    }

    pub fn len(&self) -> usize {
        self.states.len() + self.controls.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn suggest(&self, name: &str) -> Option<&str> {
        let budget = (name.len() / 3).max(2);
        let mut best: Option<(usize, &str)> = None;
        for candidate in self.states.keys().chain(self.controls.keys()) {
            let distance = edit_distance(name, candidate);
            if distance <= budget && best.map(|(d, _)| distance < d).unwrap_or(true) {
                best = Some((distance, candidate.as_str()));
            }
        }
        best.map(|(_, name)| name)
    }
}

fn validate_name(name: &str) -> Result<(), RegistryError> {
    let invalid = |reason| {
        Err(RegistryError::InvalidName {
            name: name.to_string(),
            reason,
        })
    };
    if name.is_empty() {
        return invalid("name is empty");
    }
    if name.len() > 255 {
        return invalid("name is longer than 255 bytes");
    }
    for segment in name.split('.') {
        if segment.is_empty() {
            return invalid("path segments may not be empty");
        }
        if !segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return invalid("path segments may only contain letters, digits and `_`");
        }
    }
    let first = name.as_bytes()[0];
    if !first.is_ascii_alphabetic() && first != b'_' {
        return invalid("names must start with a letter or `_`");
    }
    Ok(())
}

pub(crate) fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().flat_map(char::to_lowercase).collect();
    let b: Vec<char> = b.chars().flat_map(char::to_lowercase).collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}
