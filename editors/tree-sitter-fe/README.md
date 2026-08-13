# tree-sitter-fe

A tree-sitter grammar for the flight-engineer procedure language, mirroring
[`docs/language.md`](../../docs/language.md).

```sh
npx tree-sitter generate --abi 14
npx tree-sitter test
npx tree-sitter parse ../../examples/dc10/*.fe
```

`src/parser.c` is committed: Zed compiles it directly and never runs
`tree-sitter generate`. Regenerate it whenever `grammar.js` changes, and commit
the result. ABI 14 is what current Zed builds can load.

Two things are worth knowing before editing it:

**Keywords are contextual**, as in `fe-lang`'s lexer. `name`, `check`, `set` and
the rest are keywords only where a keyword is expected, so `hydraulic.name.check`
is a legal path. That comes from `word: $ => $.identifier` — tree-sitter only
lexes a keyword where the parser could accept one. Removing that line breaks
every path that happens to contain an engineer's choice of word.

**`path` and `control_path` are the same shape on purpose.** One is state a
procedure reads, the other is a control it moves. They are separate nodes so a
highlight query can colour them differently without two patterns overlapping on
one node and leaving the winner to the editor.

This grammar is a little more permissive than `fe-compiler`, because an editor
has to keep working on a file that is halfway through being typed. Anything it
accepts that the compiler rejects is the compiler's to report.
