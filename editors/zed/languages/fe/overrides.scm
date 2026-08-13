; Scopes referenced by `not_in` in config.toml, so that quote and brace
; auto-closing stops inside strings and comments.

(string) @string

[
  (line_comment)
  (block_comment)
] @comment
