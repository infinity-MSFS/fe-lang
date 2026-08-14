//! The Zed extension.
//!
//! Its whole job is to find `fe-lsp` and hand Zed a command to run. Everything
//! about the language — what is wrong with a file, what could go where the
//! cursor is, what a control accepts — is the server's, so that Zed and Visual
//! Studio Code are answering with the same code rather than with two
//! implementations that agree until they do not.

use zed_extension_api::{
    self as zed, settings::LspSettings, Command, LanguageServerId, Result, Worktree,
};

const SERVER: &str = "fe-lsp";

const HOW_TO_INSTALL: &str = "\
`fe-lsp` was not found.

Install it with `cargo install --path fe-lsp` from a checkout of
https://github.com/infinity-MSFS/fe-lang, or point Zed at it:

    \"lsp\": {
      \"fe-lsp\": {
        \"binary\": { \"path\": \"/path/to/fe-lsp\" }
      }
    }
";

struct FeExtension;

impl zed::Extension for FeExtension {
    fn new() -> Self {
        FeExtension
    }

    fn language_server_command(
        &mut self,
        id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<Command> {
        let settings = LspSettings::for_worktree(id.as_ref(), worktree)
            .ok()
            .and_then(|settings| settings.binary);

        // What the user configured, then what is on their PATH. Nothing is
        // downloaded: the server is built from the same checkout as the
        // compiler it embeds, and a prebuilt binary from somewhere else could
        // disagree with it about the language.
        let path = settings
            .as_ref()
            .and_then(|binary| binary.path.clone())
            .or_else(|| worktree.which(SERVER))
            .ok_or_else(|| HOW_TO_INSTALL.to_string())?;

        let args = settings
            .and_then(|binary| binary.arguments)
            .unwrap_or_default();

        Ok(Command {
            command: path,
            args,
            env: worktree.shell_env(),
        })
    }

    fn language_server_initialization_options(
        &mut self,
        id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<Option<zed::serde_json::Value>> {
        Ok(LspSettings::for_worktree(id.as_ref(), worktree)
            .ok()
            .and_then(|settings| settings.initialization_options))
    }

    fn language_server_workspace_configuration(
        &mut self,
        id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<Option<zed::serde_json::Value>> {
        Ok(LspSettings::for_worktree(id.as_ref(), worktree)
            .ok()
            .and_then(|settings| settings.settings))
    }
}

zed::register_extension!(FeExtension);
