# Flight Engineer for Visual Studio Code

A client for [`fe-lsp`](../../fe-lsp), the language server for `.fe` procedure
files.

The extension itself is a launcher of about eighty lines. Everything the editor
knows about the language — what is wrong with a file, what could go where the
cursor is, what a name means — comes from the server, which runs the real
compiler over your project on every edit. Two implementations of the language's
rules would agree right up until they did not, and the one in the editor would
be the wrong one.

## What it does

**Diagnostics** are `fe-compiler`'s, unmodified, with their stable codes and
their suggestions:

```
error[E0206]: `open` cannot be applied to `HYD_2_ELECTRIC_PUMP`
```

Everything a build reports, an editor reports — including E0216, which is only
reachable once code has been generated. Each code links to its entry in
[`docs/diagnostics.md`](../../docs/diagnostics.md), and the common mistakes come
with a one-click fix: a misspelled control becomes the right one, `open` on a
switch becomes `start`, an unlisted position becomes one the control has.

**Completion** knows the aircraft. `open ` offers valves and not switches;
`set FUEL_XFEED_SELECTOR = ` offers exactly the positions that selector has; a
condition offers state and never a control.

**Hover** reports a state path's type and a control's kind, positions and host
tag — the facts the source cannot show and that the type errors are about.

**Go to definition, references, rename and the outline** work across the whole
project, because procedure identifiers share one flat namespace over every file
compiled together.

**Formatting** reprints a file from its tokens, so comments survive. A file that
does not parse is left exactly as it is.

**Inlay hints** show what the source does not say: the position a verb moves a
control to, an analog control's registered range, a timeout in milliseconds.

## What it needs

An `fe.toml` at or above the project root, declaring the aircraft's symbols —
see [`examples/dc10/fe.toml`](../../examples/dc10/fe.toml) and
[`fe-project`](../../fe-project). Without one the server cannot know whether a
control exists, so it reports syntax only and says so in the status bar rather
than leaving you to assume a clean file is a correct one.

## Installing

**1. Build the server.** From a checkout of this repository:

```
cargo install --path fe-lsp
```

That puts `fe-lsp` in `~/.cargo/bin`, which is on your PATH if you installed
Rust with `rustup`. If it lives somewhere else, set `fe.server.path`.

**2. Install the extension.**

```
cd editors/vscode
npm install
npx --yes @vscode/vsce package
code --install-extension fe-lang-0.2.0.vsix
```

Or, for development, symlink this directory into your extensions folder:

|              |                                            |
| ------------ | ------------------------------------------ |
| Windows      | `%USERPROFILE%\.vscode\extensions\fe-lang` |
| macOS, Linux | `~/.vscode/extensions/fe-lang`             |

```powershell
# Windows, from the repository root
New-Item -ItemType SymbolicLink -Path "$env:USERPROFILE\.vscode\extensions\fe-lang" -Target "$PWD\editors\vscode"
```

## Settings

| Setting                    | Default |                                                                                                                |
| -------------------------- | ------- | -------------------------------------------------------------------------------------------------------------- |
| `fe.server.path`           | `""`    | where `fe-lsp` is; empty means look on PATH                                                                    |
| `fe.manifest`              | `""`    | where the aircraft's `fe.toml` is, relative to the workspace; empty means the nearest one at or above the root |
| `fe.inlayHints.enable`     | `true`  | show positions, ranges and resolved timeouts                                                                   |
| `fe.semanticTokens.enable` | `true`  | colour names by what the registry says they are                                                                |

## Developing

Open `editors/vscode` and press <kbd>F5</kbd> for an Extension Development Host
with `examples/dc10` open — which has an `fe.toml`, so the server runs in full.

```
cd editors/vscode
npm install
node --test
```

`test/vscode-stub.js` implements the handful of editor and language-client APIs
`extension.js` actually calls, so activation, server discovery, settings and the
status bar can be driven under plain node. Anything about the _language_ is
tested in `fe-lsp`, where it is implemented.

The TextMate grammar in `syntaxes/` stays: the server's semantic tokens layer
over it rather than replacing it, colouring only the names the registry knows.
