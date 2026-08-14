//! Settings, from either of the two places a client may send them.

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// An explicit manifest, when it is not the `fe.toml` above the workspace.
    pub manifest: Option<PathBuf>,
    pub inlay_hints: bool,
    pub semantic_tokens: bool,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            manifest: None,
            // Hints are shown by default: an analog control's registered range
            // is exactly the thing E0207 is about, and it is invisible in the
            // source.
            inlay_hints: true,
            semantic_tokens: true,
        }
    }
}

impl Config {
    pub fn from_initialization(value: Option<&serde_json::Value>) -> Config {
        value.map(Config::read).unwrap_or_default()
    }

    /// `workspace/didChangeConfiguration` sends the whole settings tree, so the
    /// server's own section has to be found inside it. Clients disagree about
    /// whether that section is nested under `fe`.
    pub fn from_settings(value: &serde_json::Value) -> Config {
        Config::read(value.get("fe").unwrap_or(value))
    }

    fn read(value: &serde_json::Value) -> Config {
        let default = Config::default();
        let bool_at = |key: &str, fallback: bool| {
            value
                .pointer(key)
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(fallback)
        };
        Config {
            manifest: value
                .pointer("/manifest")
                .and_then(serde_json::Value::as_str)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
            inlay_hints: bool_at("/inlayHints/enable", default.inlay_hints),
            semantic_tokens: bool_at("/semanticTokens/enable", default.semantic_tokens),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_when_a_client_sends_nothing() {
        let config = Config::from_initialization(None);
        assert_eq!(config, Config::default());
        assert!(config.inlay_hints);
    }

    #[test]
    fn settings_are_read_with_or_without_the_section() {
        let settings = json!({ "inlayHints": { "enable": false } });
        assert!(!Config::from_settings(&settings).inlay_hints);

        let nested = json!({ "fe": { "inlayHints": { "enable": false } } });
        assert!(!Config::from_settings(&nested).inlay_hints);
    }

    #[test]
    fn an_explicit_manifest_overrides_the_search() {
        let config = Config::from_initialization(Some(&json!({ "manifest": "aircraft/fe.toml" })));
        assert_eq!(config.manifest, Some(PathBuf::from("aircraft/fe.toml")));
    }

    /// An empty string is a client clearing the setting, not a path.
    #[test]
    fn an_empty_manifest_setting_is_no_setting() {
        let config = Config::from_initialization(Some(&json!({ "manifest": "" })));
        assert_eq!(config.manifest, None);
    }
}
