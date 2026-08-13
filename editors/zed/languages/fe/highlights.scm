; Kept in step with `editors/tree-sitter-fe/queries/highlights.scm`.
;
; `control_path` and `path` are separate nodes in the grammar precisely so that
; the thing a procedure *moves* and the thing it *reads* can be coloured
; differently without two patterns fighting over the same node.

; ---------------------------------------------------------------- definitions

(procedure name: (identifier) @function)
(call_step target: (identifier) @function)

; ------------------------------------------------------------------- symbols

(control_path (identifier) @constant)

(path (identifier) @property)

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
