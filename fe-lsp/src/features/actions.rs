//! Quick fixes.
//!
//! Every one of these is derived from a diagnostic the compiler already
//! produced, and from the registry it produced it against. Nothing here decides
//! what is wrong; it only offers the edit that the diagnostic already implies.

use fe_project::{ControlSpec, SymbolRegistry};

use crate::locate::ControlVerb;

pub struct Fix {
    pub title: String,
    /// Replacement for the diagnostic's own range.
    pub replacement: String,
    /// Whether this is the obvious single answer, which clients may apply
    /// without asking.
    pub preferred: bool,
}

/// Fixes for a diagnostic, given the text it is complaining about.
pub fn fixes(
    code: &str,
    text: &str,
    registry: Option<&SymbolRegistry>,
    verb: Option<ControlVerb>,
) -> Vec<Fix> {
    let Some(registry) = registry else {
        return Vec::new();
    };

    match code {
        // The near miss. `SymbolRegistry::suggest` is the same edit-distance
        // search that produced the diagnostic's "did you mean"; this makes it
        // one keystroke instead of a retype.
        fe_compiler::codes::UNKNOWN_SYMBOL => registry
            .suggest(text)
            .map(|suggestion| {
                vec![Fix {
                    title: format!("Change to `{suggestion}`"),
                    replacement: suggestion.to_string(),
                    preferred: true,
                }]
            })
            .unwrap_or_default(),

        // An unlisted position: offer the ones the control has. The diagnostic
        // names the control, but the *span* is the position, so the control is
        // passed in by the caller.
        fe_compiler::codes::INVALID_CONTROL_VALUE => Vec::new(),

        // A verb the kind does not accept. There is exactly one verb per kind
        // that means what the author meant.
        fe_compiler::codes::INVALID_ACTION_FOR_CONTROL => {
            let Some(verb) = verb else {
                return Vec::new();
            };
            let Some(control) = registry.control(text) else {
                return Vec::new();
            };
            equivalent(verb, &control.spec)
                .map(|replacement| {
                    vec![Fix {
                        title: format!("Use `{replacement}` instead"),
                        replacement: replacement.to_string(),
                        preferred: true,
                    }]
                })
                .unwrap_or_default()
        }

        _ => Vec::new(),
    }
}

/// Positions of the control a `set` names, for E0205.
pub fn position_fixes(registry: Option<&SymbolRegistry>, control: &str) -> Vec<Fix> {
    let Some(control) = registry.and_then(|r| r.control(control)) else {
        return Vec::new();
    };
    control
        .spec
        .positions()
        .iter()
        .map(|position| Fix {
            title: format!("Change to `{position}`"),
            replacement: position.to_string(),
            preferred: false,
        })
        .collect()
}

/// The verb that means the same thing on a control of this kind.
///
/// `open` on a switch means "make it on", and a switch spells that `start`.
/// The pairing is by intent — energise or de-energise — not by position index.
fn equivalent(verb: ControlVerb, spec: &ControlSpec) -> Option<&'static str> {
    let energise = match verb {
        ControlVerb::Start | ControlVerb::Open => true,
        ControlVerb::Stop | ControlVerb::Close => false,
        ControlVerb::Check | ControlVerb::Set => return None,
    };
    match spec {
        ControlSpec::Switch => Some(if energise { "start" } else { "stop" }),
        ControlSpec::Valve => Some(if energise { "open" } else { "close" }),
        // A selector or an analog is set to a named value; a checklist has no
        // actuator at all. None of them has a verb that means this.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verb_maps_to_the_one_that_means_the_same_thing() {
        assert_eq!(
            equivalent(ControlVerb::Open, &ControlSpec::switch()),
            Some("start")
        );
        assert_eq!(
            equivalent(ControlVerb::Close, &ControlSpec::switch()),
            Some("stop")
        );
        assert_eq!(
            equivalent(ControlVerb::Start, &ControlSpec::valve()),
            Some("open")
        );
        assert_eq!(
            equivalent(ControlVerb::Stop, &ControlSpec::valve()),
            Some("close")
        );
    }

    /// A checklist has nothing to actuate and a selector has no on/off, so there
    /// is no edit to offer — and offering one would be inventing an intent.
    #[test]
    fn kinds_with_no_equivalent_verb_offer_nothing() {
        assert_eq!(
            equivalent(ControlVerb::Open, &ControlSpec::checklist()),
            None
        );
        assert_eq!(
            equivalent(ControlVerb::Start, &ControlSpec::selector(["A", "B"])),
            None
        );
        assert_eq!(
            equivalent(ControlVerb::Open, &ControlSpec::analog(0.0, 1.0)),
            None
        );
    }

    #[test]
    fn a_near_miss_is_offered_as_an_edit() {
        let mut registry = SymbolRegistry::new();
        registry
            .define_control("HYD_2_ENGINE_PUMP", ControlSpec::checklist(), 1)
            .unwrap();

        let fixes = fixes(
            fe_compiler::codes::UNKNOWN_SYMBOL,
            "HYD_2_ENGINE_PUM",
            Some(&registry),
            None,
        );
        assert_eq!(fixes.len(), 1);
        assert_eq!(fixes[0].replacement, "HYD_2_ENGINE_PUMP");
        assert!(fixes[0].preferred);
    }

    #[test]
    fn a_name_with_no_near_miss_offers_nothing() {
        let mut registry = SymbolRegistry::new();
        registry
            .define_control("HYD_2_ENGINE_PUMP", ControlSpec::checklist(), 1)
            .unwrap();
        assert!(
            fixes(
                fe_compiler::codes::UNKNOWN_SYMBOL,
                "COMPLETELY_DIFFERENT",
                Some(&registry),
                None
            )
            .is_empty()
        );
    }

    #[test]
    fn positions_come_from_the_control_that_was_named() {
        let mut registry = SymbolRegistry::new();
        registry
            .define_control(
                "FUEL_XFEED_SELECTOR",
                ControlSpec::selector(["OFF", "TANK_1_TO_3", "TANK_3_TO_1"]),
                1,
            )
            .unwrap();

        let fixes = position_fixes(Some(&registry), "FUEL_XFEED_SELECTOR");
        assert_eq!(
            fixes
                .iter()
                .map(|f| f.replacement.as_str())
                .collect::<Vec<_>>(),
            ["OFF", "TANK_1_TO_3", "TANK_3_TO_1"]
        );
    }

    /// Without a manifest the server has no grounds for any of this.
    #[test]
    fn nothing_is_offered_without_a_registry() {
        assert!(fixes(fe_compiler::codes::UNKNOWN_SYMBOL, "X", None, None).is_empty());
        assert!(position_fixes(None, "X").is_empty());
    }
}
