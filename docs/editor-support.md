# Editor support

There is one implementation of the language and the editor uses it.
[`fe-lsp`](../fe-lsp) is a language server that runs `fe-compiler` over the whole
project on every edit and forwards what it says; the Visual Studio Code and Zed
extensions are launchers for it.

```
   .fe files ──┐
               ├─► fe_compiler::check_full ──► diagnostics ──► the editor
   fe.toml ────┘                               syntax trees      features
```

## Why this needed a file format

`docs/architecture.md` explains why there is no compiler binary: reading files,
globbing directories and knowing what the aircraft's symbols are called all
belong to a specific aircraft's build. The same argument said an editor could
never check a procedure, because the `SymbolRegistry` is Rust code in that
build, and an editor cannot run it.

The way out is not to relax the constraint but to write the registry down.
[`fe-project`](../fe-project) reads an `fe.toml` into exactly the
`SymbolRegistry` that `docs/symbols.md` describes, without touching a filesystem
— the caller supplies the text — so the same function serves the editor, a
`build.rs`, and a test. One file, read by both, and the answer an author gets
while typing is the answer the build will give.

`fe-project/tests/manifest.rs` asserts that the example manifest and the
registry built in Rust are the same registry, symbol for symbol and tag for tag.

## What it can answer, and what it cannot

**With an `fe.toml`**, everything the compiler can: E0201 through E0217, the
warnings, and the two diagnostics that are only reachable once code has been
generated. The server calls `fe_compiler::check_full` rather than
`fe_compiler::check` for exactly that reason — `check` stops after analysis, so
it cannot report E0216, and an author told their procedure was fine whose build
then rejected it has been let down by the editor rather than helped by it.

**Without one**, syntax and lexical diagnostics only. Not "everything, against
an empty registry": that would make every name in every file E0201 and bury the
real errors under a wall of red about names that are perfectly fine. And not
silently, either — the server sends a `window/showMessage` and an `fe/status`
notification that both clients put in the status bar, because a file showing no
errors and a file having no errors must not look the same.

## What the server adds

The compiler answers "what is wrong with this". The server answers the questions
an editor asks that a compiler has no reason to:

|                                        | Where it comes from                                                                       |
| -------------------------------------- | ----------------------------------------------------------------------------------------- |
| what could go here                     | the token stream, classified by `fe-lsp/src/completion.rs`, then filtered by the registry |
| what does this name mean               | the registry, plus the syntax tree for procedures                                         |
| where is it defined, what else uses it | one walk over the trees, in `fe-lsp/src/locate.rs`                                        |
| how do I fix this                      | the diagnostic, plus `SymbolRegistry::suggest` and the control-kind table                 |
| how should this be laid out            | the token _and trivia_ stream, so comments survive                                        |

Two of those are worth spelling out.

**Completion is token-driven, never tree-driven.** Text being typed usually does
not parse, and that is exactly when completion is wanted. Working from tokens
also makes the language's contextual keywords fall out for free:
`hydraulic.name.check` is three path segments because each follows a `.`, and
`set complete.timeout = OPEN` parses because `complete` follows `set` — the same
predecessor rules the parser uses, not special cases.

The one thing tokens cannot decide is where a statement ends. The parser does not
care about newlines, so `check A check B` on one line is two steps; but `fail`
with no message and `complete` with no condition are both whole statements, so
after either of them nothing in the token stream says whether the next word
continues it. The author's line break does, and every procedure in this
repository is written one statement to a line — with a continuation line
announcing itself, because a line cannot begin with `&&`.

**Formatting works from tokens and trivia, not from the tree.** The lexer skips
comments and the AST has nowhere to put them, so a pretty-printer walking the
tree would produce a beautifully formatted file with every explanation of _why_ a
step exists deleted. `fe_lang::lexer::tokenize_with_trivia` keeps them.
Formatting also refuses a file that does not parse: the editor writes the result
straight to disk, without asking.

## Semantic tokens and the grammars

`editors/README.md` used to carry a warning that the TextMate grammar and the
tree-sitter grammar could drift, with a checklist for keeping them in step. The
server's semantic tokens are the structural answer to it, and they work by doing
less: they cover **names only** — control, state, procedure, position, category
— and leave keywords, strings, numbers and comments to the grammars, which
clients layer them over rather than replacing.

A name is then coloured by what the registry says it _is_ rather than by where it
appears, so a control written into a condition shows up as a control, which is
the mistake E0203 is about. An unregistered name is left uncoloured on purpose:
colour means "the aircraft has heard of this", which is worth seeing at a glance.

## Extending it

Adding a statement to the language means, on this side of the fence:

1. `fe-lsp/snippets/fe.json` — one copy, served to both editors.
2. `fe-lsp/src/completion.rs` if it introduces a new place the cursor can be.
3. `fe-lsp/src/locate.rs` if it introduces a new place a name can appear. Hover,
   go-to-definition, references, rename, semantic tokens and inlay hints all read
   from that one walk, so they follow from it.
4. `fe-lsp/src/features/format.rs` if it needs to be laid out in a way the
   statement-per-line rule does not already produce.

Adding a control kind means `fe-project` (the `kind` table), and the verb table
in `fe-lsp/src/locate.rs` that decides what completion offers and what a quick
fix suggests. `docs/extending.md` has the compiler-side list.
