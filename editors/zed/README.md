# Flight Engineer for Zed

Language support for `.fe` procedure files: a tree-sitter grammar, highlighting,
outline, bracket matching, indentation and snippet completions.

## What it does

**Highlighting** distinguishes the two kinds of name in the language, because
confusing them is the mistake that matters: a **control**
(`HYD_2_ELECTRIC_PUMP` — something a procedure *moves*) is a constant, and a
**state path** (`hydraulic.2.pressure` — something a procedure *reads*) is a
property. They are separate nodes in the grammar rather than two highlight
patterns competing over one node.

Keywords are contextual here exactly as they are in `fe-lang`'s lexer:
`hydraulic.name.check` stays a path, and `set complete.timeout = OPEN` parses.

**Snippets** cover every statement in the language, so typing `wa` offers
`wait`, `waitt` (with a timeout) and `waitf` (timeout that fails), and typing
`proc` offers the whole procedure skeleton. Zed's word completion fills in the
control and state names already used in the buffer.

**Outline** (`cmd-shift-o`) lists procedures by identifier — the name `call` and
the host use.

It does not compile anything. Only `fe-compiler`, against a specific aircraft's
`SymbolRegistry`, knows whether a control exists or a value is in range.

## Installing

Zed builds a grammar by fetching it with git, so the grammar has to live in a
commit before the extension can be installed — a placeholder `rev` cannot work.
The generated `src/parser.c` is committed for the same reason: Zed compiles that
file directly and never runs `tree-sitter generate`.

**1. Commit the grammar.** Anything under `editors/tree-sitter-fe` that is not
committed is invisible to Zed.

**2. Point `[grammars.fe]` at that commit.** From Git Bash or any POSIX shell:

```sh
editors/zed/pin-grammar.sh            # a commit pushed to GitHub
editors/zed/pin-grammar.sh --local    # a commit in this checkout only
```

`--local` rewrites `repository` to `file://<this checkout>`, which is the way to
work on the grammar without pushing. Either way it is two lines in
`extension.toml` if you would rather edit them by hand:

```toml
[grammars.fe]
repository = "https://github.com/infinity-MSFS/fe-lang"
rev = "the commit sha"
path = "editors/tree-sitter-fe"
```

**3. Install it.** In Zed, run `zed: install dev extension` from the command
palette and choose the `editors/zed` directory. Open a `.fe` file.

Re-run `zed: install dev extension` after changing the grammar — queries and
snippets are re-read, but the grammar is only rebuilt when the extension is
reinstalled, and only if `rev` changed.

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

| File | |
| --- | --- |
| `languages/fe/config.toml` | file suffixes, comments, brackets, indent width |
| `languages/fe/highlights.scm` | colour |
| `languages/fe/brackets.scm` | bracket matching |
| `languages/fe/indents.scm` | auto-indent |
| `languages/fe/outline.scm` | the outline and breadcrumbs |
| `languages/fe/overrides.scm` | where auto-closing stops: strings and comments |
| `snippets/fe.json` | completions; the file name must be the language name in lower case |
