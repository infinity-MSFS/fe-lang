# Diagnostics

Every diagnostic has a stable code, a primary span, and usually a note or a
suggestion. Codes never change meaning; a code is retired rather than reused.

```
error[E0201]: unknown aircraft symbol
 --> hydraulic.fe:4:11
  |
4 |     check HYD_2_ENGINE_PUM
  |           ^^^^^^^^^^^^^^^^ `HYD_2_ENGINE_PUM` is not registered
  |
  = help: did you mean `HYD_2_ENGINE_PUMP`?
```

`Diagnostic::render(&SourceMap)` produces this; `Diagnostics::render` does the
whole bag. Rendering is separate from producing, so a host can consume the
structured form instead — an editor extension wants spans and codes, not text.

## Lexical

| Code | Meaning |
| --- | --- |
| E0001 | unexpected character |
| E0002 | unterminated string |
| E0003 | invalid escape sequence |
| E0004 | malformed number |
| E0005 | unterminated block comment |
| E0006 | invalid duration |

## Syntax

| Code | Meaning |
| --- | --- |
| E0101 | expected a specific token |
| E0102 | expected a declaration |
| E0103 | expected a step or metadata entry |
| E0104 | expected an expression |
| E0105 | metadata after the first step |
| E0106 | duplicate metadata entry |
| E0107 | chained comparison (`a < b < c`) |
| E0108 | procedure has no steps |

## Semantic

| Code | Meaning |
| --- | --- |
| E0201 | unknown symbol |
| E0202 | not a control (attempt to write state) |
| E0203 | not readable (attempt to read a control) |
| E0204 | type mismatch |
| E0205 | invalid value for this control |
| E0206 | invalid action for this control kind |
| E0207 | value outside the registered range |
| E0208 | unknown procedure |
| E0209 | duplicate procedure identifier |
| E0210 | recursive `call` |
| E0211 | missing required metadata |
| E0212 | invalid metadata value |
| E0213 | invalid timeout |
| E0214 | `if` nesting too deep |
| E0215 | `call` chain deeper than the runtime stack |
| E0216 | procedure too complex |
| E0217 | database too large |

## Warnings

| Code | Meaning |
| --- | --- |
| W0001 | unreachable step |
| W0002 | float compared for equality |
| W0003 | `if` with empty branches |
| W0005 | condition reads no aircraft state |

W0004 is retired: it warned about a procedure with no explicit `complete`,
which turned out to describe every well-written checklist once falling off the
end of a body was defined to mean completion.

## Internal

`E0999` means the compiler produced a database it could not read back. It is a
compiler bug by definition, and it exists because the alternative — shipping
that database — is worse.

## Recovery

The parser recovers at two levels: to the next plausible step inside a body,
and to the next `procedure` declaration otherwise. A typo in the first
procedure of a file does not hide every other error in it, which matters when a
procedure library is edited as a whole.

Semantic analysis does not stop at the first error either. It reports
everything it can and returns `None` at the end if anything was fatal.
