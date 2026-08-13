# Flight Engineer for Visual Studio Code

Editing support for `.fe` procedure files: highlighting, completion, an
outline, and go-to-definition on `call`.

## What it does

**Highlighting** distinguishes the two kinds of name in the language, because
confusing them is the mistake that matters: a **control** (`HYD_2_ELECTRIC_PUMP`
— something a procedure *moves*) is coloured as a constant, and a **state path**
(`hydraulic.2.pressure` — something a procedure *reads*) as a property. A
`category` that is not one of the four the compiler accepts is marked as an
error while you type.

**Completion** is context-aware, and knows nothing it did not read out of your
own files:

| Where the cursor is | What is offered |
| --- | --- |
| between procedures | the `procedure` skeleton |
| in a procedure that has no steps yet | metadata entries first, then steps |
| in a body or an `if` block | steps |
| after `category` | `normal`, `abnormal`, `emergency`, `reference` |
| after `check`, `set`, `start`, `stop`, `open`, `close` | every control named anywhere in the workspace |
| after `set CONTROL =` | positions this workspace uses, then the usual ones |
| after `call` | every procedure in the workspace, with its crew-facing title |
| after `timeout` | `500ms`, `1s`, `5s`, `10s`, `30s`, `1m`, `5m` |
| in a condition | every state path in the workspace, `true`, `false`, `timeout` |
| inside a string or comment | nothing |

**Outline and breadcrumbs** list the procedures in a file by identifier, with
the `name` string as the detail.

**Go to definition** on the target of a `call` jumps to that procedure, in
whichever file declares it — procedure identifiers share one flat namespace
across everything compiled together.

## What it does not do

It does not compile anything, so it cannot tell you that a control exists, that
a position is one the host registered, or that a value is in range. Only
`fe-compiler` knows that, and only against a specific aircraft's
`SymbolRegistry`. The completion list is a reflection of what your `.fe` files
already say — a fast way to type a name you have used before, not an assurance
that the name is real.

## Installing

Copy or symlink this directory into your extensions folder and restart:

| | |
| --- | --- |
| Windows | `%USERPROFILE%\.vscode\extensions\fe-lang` |
| macOS, Linux | `~/.vscode/extensions/fe-lang` |

```powershell
# Windows, from the repository root
New-Item -ItemType SymbolicLink -Path "$env:USERPROFILE\.vscode\extensions\fe-lang" -Target "$PWD\editors\vscode"
```

Or build a `.vsix` and install it like any other extension:

```
cd editors/vscode
npx --yes @vscode/vsce package
code --install-extension fe-lang-0.1.0.vsix
```

## Developing

There is no build step and there are no dependencies — it is plain JavaScript
against the editor API. Open `editors/vscode` in VS Code and press <kbd>F5</kbd>
to launch an Extension Development Host with `examples/dc10` open.

The half of the extension with opinions — where the cursor is, and what the
files say — is in `src/analysis.js`, which never imports `vscode` so that it can
be tested with plain node. The other half is tested too: `test/vscode-stub.js`
implements the handful of editor APIs `extension.js` actually calls, so the
providers can be driven end to end against `examples/dc10` without launching an
editor.

```
cd editors/vscode
node --test
```

## Settings

| Setting | Default | |
| --- | --- | --- |
| `fe.completion.enabled` | `true` | suggest anything at all |
| `fe.completion.scanWorkspace` | `true` | gather names from every `.fe` file in the workspace, not just the open ones |
