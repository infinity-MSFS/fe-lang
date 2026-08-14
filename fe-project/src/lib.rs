use std::collections::BTreeMap;
use std::ops::Range;

use serde::Deserialize;

pub use fe_compiler::{ControlSpec, SymbolRegistry, ValueType};

pub const DEFAULT_SOURCE: &str = ".";
pub const MANIFEST_NAME: &str = "fe.toml";

#[derive(Debug, Clone)]
pub struct Manifest {
    pub sources: Vec<String>,
    pub registry: SymbolRegistry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestError {
    pub message: String,
    pub span: Option<Range<usize>>,
}

impl ManifestError {
    fn at(span: Range<usize>, message: impl Into<String>) -> ManifestError {
        ManifestError {
            message: message.into(),
            span: Some(span),
        }
    }
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ManifestError {}

pub fn parse(text: &str) -> Result<Manifest, Vec<ManifestError>> {
    let raw: RawManifest = match toml::from_str(text) {
        Ok(raw) => raw,
        Err(error) => {
            return Err(vec![ManifestError {
                message: error.message().to_string(),
                span: error.span(),
            }]);
        }
    };

    let mut errors = Vec::new();
    let mut registry = SymbolRegistry::new();

    for (name, entry) in &raw.state {
        let span = name.span();
        let ty = match entry.get_ref().ty.get_ref().as_str() {
            "bool" => ValueType::Bool,
            "f32" | "number" => ValueType::F32,
            other => {
                errors.push(ManifestError::at(
                    entry.get_ref().ty.span(),
                    format!("unknown state type `{other}`, expected `bool` or `f32`"),
                ));
                continue;
            }
        };
        if let Err(error) = registry.define_state(name.get_ref(), ty, entry.get_ref().tag) {
            errors.push(ManifestError::at(span, error.to_string()));
        }
    }

    for (name, entry) in &raw.controls {
        match entry.get_ref().spec() {
            Ok(spec) => {
                if let Err(error) =
                    registry.define_control(name.get_ref(), spec, entry.get_ref().tag)
                {
                    errors.push(ManifestError::at(name.span(), error.to_string()));
                }
            }
            Err(error) => errors.push(error),
        }
    }

    if errors.is_empty() {
        Ok(Manifest {
            sources: raw
                .project
                .sources
                .unwrap_or_else(|| vec![DEFAULT_SOURCE.to_string()]),
            registry,
        })
    } else {
        Err(errors)
    }
}

type Spanned<T> = toml::Spanned<T>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    #[serde(default)]
    project: RawProject,
    #[serde(default)]
    state: BTreeMap<Spanned<String>, Spanned<RawState>>,
    #[serde(default)]
    controls: BTreeMap<Spanned<String>, Spanned<RawControl>>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawProject {
    sources: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawState {
    #[serde(rename = "type")]
    ty: Spanned<String>,
    tag: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawControl {
    kind: Spanned<String>,
    tag: u32,
    positions: Option<Spanned<Vec<String>>>,
    min: Option<Spanned<f32>>,
    max: Option<Spanned<f32>>,
}

impl RawControl {
    fn spec(&self) -> Result<ControlSpec, ManifestError> {
        fn reject(field: &str, span: Range<usize>, kind: &str) -> ManifestError {
            ManifestError::at(
                span,
                format!("`{field}` does not apply to a {kind} control"),
            )
        }
        let kind = self.kind.get_ref().as_str();

        if kind != "selector" {
            if let Some(positions) = &self.positions {
                return Err(reject("positions", positions.span(), kind));
            }
        }
        if kind != "analog" {
            if let Some(min) = &self.min {
                return Err(reject("min", min.span(), kind));
            }
            if let Some(max) = &self.max {
                return Err(reject("max", max.span(), kind));
            }
        }

        Ok(match kind {
            "switch" => ControlSpec::switch(),
            "valve" => ControlSpec::valve(),
            "checklist" => ControlSpec::checklist(),
            "selector" => match &self.positions {
                Some(positions) => ControlSpec::selector(positions.get_ref().clone()),
                None => {
                    return Err(ManifestError::at(
                        self.kind.span(),
                        "a selector needs `positions`",
                    ));
                }
            },
            "analog" => match (&self.min, &self.max) {
                (Some(min), Some(max)) => ControlSpec::analog(*min.get_ref(), *max.get_ref()),
                _ => {
                    return Err(ManifestError::at(
                        self.kind.span(),
                        "an analog control needs `min` and `max`",
                    ));
                }
            },
            other => {
                return Err(ManifestError::at(
                    self.kind.span(),
                    format!(
                        "unknown control kind `{other}`, expected one of \
                         `switch`, `valve`, `selector`, `analog`, `checklist`"
                    ),
                ));
            }
        })
    }
}
