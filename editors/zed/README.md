# Flight Engineer for Zed

Language support for `.fe` procedure files: a tree-sitter grammar for
highlighting, outline, brackets and indentation, and
[`fe-lsp`](../../fe-lsp) for everything that needs to know what the aircraft
actually has.

## What it does

The grammar does what a grammar can. **Highlighting** distinguishes the two
kinds of name in the language, because confusing them is the mistake that
matters: a **control** (`HYD_2_ELECTRIC_PUMP` — something a procedure _moves_)
and a **state path** (`hydraulic.2.pressure` — something it _reads_) are
separate nodes in the grammar rather than two highlight patterns competing over
one node. Keywords are contextual exactly as they are in `fe-lang`'s lexer, so
`hydraulic.name.check` stays a path and `set complete.timeout = OPEN` parses.
**Outline** (`cmd-shift-o`) lists procedures by the identifier `call` uses.

The server does the rest, by running the real compiler over your project:
diagnostics with the compiler's own codes and quick fixes, completion filtered
by what a control actually accepts, hover, go-to-definition and references
across files, rename, formatting, and inlay hints. See
[`fe-lsp`](../../fe-lsp) for the whole list.

## Installing

**1. Build the server.**

```
cargo install --path fe-lsp
```

The extension looks for `fe-lsp` on your PATH, which `~/.cargo/bin` is if you
installed Rust with `rustup`. To point it somewhere else, in `settings.json`:

```json
{
  "lsp": {
    "fe-lsp": {
      "binary": { "path": "/path/to/fe-lsp" }
    }
  }
}
```

**2. Commit the grammar.** Zed builds a grammar by fetching it with git, so it
has to live in a commit before the extension can be installed — a placeholder
`rev` cannot work. The generated `src/parser.c` is committed for the same
reason: Zed compiles that file directly and never runs `tree-sitter generate`.

**3. Point `[grammars.fe]` at that commit.** From Git Bash or any POSIX shell:

```sh
editors/zed/pin-grammar.sh            # a commit pushed to GitHub
editors/zed/pin-grammar.sh --local    # a commit in this checkout only
```

`--local` rewrites `repository` to `file://<this checkout>`, which is the way to
work on the grammar without pushing.

**4. Install it.** In Zed, run `zed: install dev extension` from the command
palette and choose the `editors/zed` directory. Open a `.fe` file.

Re-run `zed: install dev extension` after changing the extension: it is compiled
to WebAssembly, and Zed only rebuilds it when reinstalled. The grammar is only
rebuilt when `rev` changes.

### When the install says `failed to compile grammar 'fe'`

Zed clones the grammar into `editors/zed/grammars/fe` and reuses it, but refuses
a clone whose origin is not the `repository` currently in `extension.toml` —
so switching between `--local` and GitHub strands it. `pin-grammar.sh` now
removes a stranded checkout, but if one is already there:

```sh
rm -rf editors/zed/grammars
```

It is build output, and the next install re-clones it. The full reason is in
Zed's log (`zed: open log`), which is worth reading before guessing: the same
message covers a `rev` that was never pushed and a parser that will not compile.

## Settings

The server's own options go under `initialization_options`:

```json
{
  "lsp": {
    "fe-lsp": {
      "initialization_options": {
        "manifest": "aircraft/fe.toml",
        "inlayHints": { "enable": true },
        "semanticTokens": { "enable": true }
      }
    }
  }
}
```

`manifest` is where the aircraft's `fe.toml` is. Left unset, the server uses the
nearest one at or above the project root; without one at all it reports syntax
only, and says so.

## Building the extension

It is a Rust crate compiled to a WebAssembly component, which is why
`editors/zed` is in the root manifest's `exclude` list — a `cargo build` at the
top of the repository must not try to compile it for the host.

```sh
cd editors/zed
cargo build --release --target wasm32-wasip1
```

## Working on the grammar

The grammar itself is in [`../tree-sitter-fe`](../tree-sitter-fe). After editing
`grammar.js`:

```sh
cd editors/tree-sitter-fe
npx tree-sitter generate --abi 14   # regenerate src/parser.c
npx tree-sitter test                # the corpus in test/corpus
npx tree-sitter parse ../../examples/dc10/*.fe   # the real examples
```

ABI 14 is deliberate: it is what every current Zed build can load.

The query files are checked the same way, against a real procedure:

```sh
npx tree-sitter query ../zed/languages/fe/highlights.scm ../../examples/dc10/hydraulic.fe
```

| File                          |                                                           |
| ----------------------------- | --------------------------------------------------------- |
| `src/fe.rs`                   | the extension: finds `fe-lsp` and passes settings through |
| `languages/fe/config.toml`    | file suffixes, comments, brackets, indent width           |
| `languages/fe/highlights.scm` | colour                                                    |
| `languages/fe/brackets.scm`   | bracket matching                                          |
| `languages/fe/indents.scm`    | auto-indent                                               |
| `languages/fe/outline.scm`    | the outline and breadcrumbs                               |
| `languages/fe/overrides.scm`  | where auto-closing stops: strings and comments            |

Snippets are no longer contributed here. The server serves them through
completion so that they can be filtered by where the cursor is — `category` is
not a step, and `check` is not metadata — and contributing them as well would
offer every one of them twice.
