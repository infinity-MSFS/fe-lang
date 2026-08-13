# Editor support

Two plugins and the grammar they share.

| | |
| --- | --- |
| [`vscode/`](vscode) | Visual Studio Code extension — highlighting, completion, outline, go-to-definition |
| [`zed/`](zed) | Zed extension — highlighting, completion, outline, brackets, indentation |
| [`tree-sitter-fe/`](tree-sitter-fe) | the tree-sitter grammar Zed uses (and Neovim or Helix can) |

Each directory has its own README with how to install it.

## What they know and what they don't

Neither plugin compiles anything, and that boundary is deliberate. Whether a
control exists, whether `TANK_3_TO_1` is a position that control has, and
whether `22.5` is inside its registered range are questions only `fe-compiler`
can answer, and only against a specific aircraft's `SymbolRegistry` — which
lives in that aircraft's build, not in this repository. An editor that guessed
at those answers would be wrong in the one direction that matters.

So the plugins do the part that needs no registry:

* **Two kinds of name, two colours.** A control is something a procedure
  *moves*; a state path is something it *reads*. `HYD_2_ELECTRIC_PUMP` and
  `hydraulic.2.pressure` are not the same kind of thing and do not look the
  same.
* **Contextual keywords, handled properly.** `name`, `check` and `set` are
  keywords at the start of a statement and ordinary path segments everywhere
  else, so `hydraulic.name.check` stays a path — in both plugins, and in the
  grammar rather than by luck.
* **Completion from what you have already written.** Control names, state
  paths, positions and procedure identifiers are gathered out of the `.fe` files
  themselves. It is a fast way to retype a name you have used before, not a
  claim that the name is real.

## Keeping the two in step

The two plugins have separate grammars for separate machinery — a TextMate
grammar for VS Code, tree-sitter for Zed — and they can drift. If you add a
statement to the language, the checklist is:

1. `fe-lang` — lexer, parser, AST ([`docs/extending.md`](../docs/extending.md)).
2. `editors/tree-sitter-fe/grammar.js`, then regenerate and run the corpus.
3. `editors/tree-sitter-fe/queries/highlights.scm` and
   `editors/zed/languages/fe/highlights.scm` — kept identical.
4. `editors/vscode/syntaxes/fe.tmLanguage.json`.
5. The snippets: `editors/zed/snippets/fe.json` and
   `editors/vscode/snippets/fe.json`.
6. `editors/vscode/src/analysis.js` if the statement introduces a new place the
   cursor can be, and a case in `editors/vscode/test/analysis.test.js`.

## Tests

```
cd editors/tree-sitter-fe && npx tree-sitter test     # grammar corpus
cd editors/vscode && node --test                      # contexts, indexing, and the providers
```

Both suites also run against `examples/dc10/*.fe`, so a change to the language
that the examples adopt will fail here first.
