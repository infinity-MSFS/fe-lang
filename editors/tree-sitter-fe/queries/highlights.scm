; Highlights for tree-sitter's own tooling and for editors that read queries
; from the grammar repository (Neovim, Helix, the `tree-sitter highlight` CLI).
;
; Zed reads `editors/zed/languages/fe/highlights.scm` instead. The two files are
; kept in step with each other; if you change one, change the other.

; ---------------------------------------------------------------- definitions

(procedure name: (identifier) @function)
(call_step target: (identifier) @function)

; ------------------------------------------------------------------- symbols

; What a procedure moves.
(control_path (identifier) @constant)

; What a procedure reads.
(path (identifier) @property)

; The right-hand side of `set`.
(position (identifier) @constant)

(category_entry
  value: (identifier) @constant
  (#any-of? @constant "normal" "abnormal" "emergency" "reference"))

; ------------------------------------------------------------------ keywords

"procedure" @keyword

[
  "name"
  "description"
  "category"
  "priority"
  "revision"
  "trigger"
  "require"
] @keyword

[
  "check"
  "set"
  "start"
  "stop"
  "open"
  "close"
  "notify"
  "call"
  "wait"
  "timeout"
  "if"
  "else"
  "complete"
  "when"
  "fail"
] @keyword

; -------------------------------------------------------------------- values

(string) @string
(escape_sequence) @string.escape
(number) @number
(duration) @number
(boolean) @boolean
(line_comment) @comment
(block_comment) @comment

; ----------------------------------------------------------------- operators

[
  "&&"
  "||"
  "!"
  "<"
  "<="
  ">"
  ">="
  "=="
  "!="
  "="
  "-"
] @operator

["{" "}" "(" ")"] @punctuation.bracket
"." @punctuation.delimiter
