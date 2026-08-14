//! Reprinting a procedure file.
//!
//! This works from the token stream rather than the syntax tree, for one
//! reason: the AST has nowhere to put a comment. `fe-lang`'s lexer skips
//! comments as trivia, so a pretty-printer walking the tree would produce a
//! beautifully formatted file with every explanation of *why* a step exists
//! deleted — which, in a document describing what to do when a hydraulic system
//! fails, is the worst possible thing to lose.
//!
//! `tokenize_with_trivia` keeps them, and this merges the two streams back into
//! source order.

use fe_lang::diagnostics::Diagnostics;
use fe_lang::lexer::{Trivia, TriviaKind, tokenize_with_trivia};
use fe_lang::span::{Span, UnitId};
use fe_lang::token::{Keyword, Token, TokenKind};

const INDENT: &str = "    ";

/// Format `source`, or `None` if it should be left alone.
///
/// A file that does not lex or parse is returned untouched. Reflowing text the
/// server does not understand is how a half-written procedure becomes a
/// scrambled one, and the editor offers no warning before writing the result to
/// disk.
pub fn format(source: &str) -> Option<String> {
    let mut diagnostics = Diagnostics::new();
    let (tokens, trivia) = tokenize_with_trivia(UnitId(0), source, &mut diagnostics);
    if diagnostics.has_errors() {
        return None;
    }
    // Parsed for its verdict only; the reprint works from the tokens.
    let _ = fe_lang::parser::parse(UnitId(0), &tokens, &mut diagnostics);
    if diagnostics.has_errors() {
        return None;
    }

    let out = print(source, &tokens, &trivia);
    (out != source).then_some(out)
}

/// One item of the merged stream.
enum Item<'a> {
    Token(&'a Token<'a>),
    Comment(&'a Trivia),
}

impl Item<'_> {
    fn span(&self) -> Span {
        match self {
            Item::Token(token) => token.span,
            Item::Comment(trivia) => trivia.span,
        }
    }
}

fn merge<'a>(tokens: &'a [Token<'a>], trivia: &'a [Trivia]) -> Vec<Item<'a>> {
    let mut items: Vec<Item> = tokens
        .iter()
        .filter(|token| token.kind != TokenKind::Eof)
        .map(Item::Token)
        .chain(trivia.iter().map(Item::Comment))
        .collect();
    items.sort_by_key(|item| item.span().start);
    items
}

struct Printer {
    out: String,
    depth: usize,
    /// Nothing has been written to the current line yet.
    fresh: bool,
}

impl Printer {
    fn newline(&mut self) {
        if !self.fresh {
            self.out.push('\n');
            self.fresh = true;
        }
    }

    fn blank_line(&mut self) {
        self.newline();
        if !self.out.ends_with("\n\n") && !self.out.is_empty() {
            self.out.push('\n');
        }
    }

    fn word(&mut self, text: &str) {
        if self.fresh {
            self.out.push_str(&INDENT.repeat(self.depth));
            self.fresh = false;
        } else if !self.out.ends_with(' ') {
            self.out.push(' ');
        }
        self.out.push_str(text);
    }

    /// Written tight against what precedes it: `.`, and a `(`'s contents.
    fn tight(&mut self, text: &str) {
        if self.fresh {
            self.out.push_str(&INDENT.repeat(self.depth));
            self.fresh = false;
        }
        self.out.push_str(text);
    }
}

fn print(source: &str, tokens: &[Token], trivia: &[Trivia]) -> String {
    let items = merge(tokens, trivia);
    let mut printer = Printer {
        out: String::new(),
        depth: 0,
        fresh: true,
    };

    for (index, item) in items.iter().enumerate() {
        let previous = index.checked_sub(1).map(|i| &items[i]);
        let same_line_as_previous = previous.is_some_and(|previous| {
            !source[previous.span().end as usize..item.span().start as usize].contains('\n')
        });
        // At most one blank line survives: an author's paragraph break is
        // meaningful, six of them are not.
        let blank_before = previous.is_some_and(|previous| {
            source[previous.span().end as usize..item.span().start as usize]
                .matches('\n')
                .count()
                > 1
        });

        match item {
            Item::Comment(trivia) => {
                let text = &source[trivia.span.start as usize..trivia.span.end as usize];
                if same_line_as_previous && !printer.fresh {
                    // A trailing comment explains the line it is on; keep it
                    // there.
                    printer.word(text);
                } else {
                    if blank_before {
                        printer.blank_line();
                    } else {
                        printer.newline();
                    }
                    for (offset, line) in text.lines().enumerate() {
                        if offset > 0 {
                            printer.newline();
                        }
                        // A block comment's continuation lines are the author's
                        // to align; only the first gets our indent.
                        printer.tight(if offset == 0 { line } else { line.trim_start() });
                    }
                    if trivia.kind == TriviaKind::Line {
                        printer.newline();
                    }
                }
            }

            Item::Token(token) => match token.kind {
                TokenKind::LBrace => {
                    printer.word("{");
                    printer.depth += 1;
                    printer.newline();
                }
                TokenKind::RBrace => {
                    printer.depth = printer.depth.saturating_sub(1);
                    printer.newline();
                    printer.word("}");
                    if printer.depth == 0 {
                        printer.newline();
                    }
                }
                TokenKind::Dot => printer.tight("."),
                TokenKind::LParen => printer.word("("),
                TokenKind::RParen => printer.tight(")"),
                TokenKind::Minus => printer.word("-"),
                TokenKind::Str(_) => {
                    printer.word(&source[token.span.start as usize..token.span.end as usize])
                }
                _ => {
                    let text = &source[token.span.start as usize..token.span.end as usize];
                    if starts_a_line(token, previous, source) {
                        printer.newline();
                    }
                    // A segment after `.` belongs to the path, and a number
                    // after `-` belongs to the number.
                    if matches!(
                        previous,
                        Some(Item::Token(t)) if matches!(t.kind, TokenKind::Dot | TokenKind::Minus)
                    ) {
                        printer.tight(text);
                    } else {
                        printer.word(text);
                    }
                }
            },
        }
    }

    printer.newline();
    // Exactly one trailing newline.
    while printer.out.ends_with("\n\n") {
        printer.out.pop();
    }
    printer.out
}

/// Whether this token begins a new line of output.
///
/// One statement per line, which is how every procedure in this repository is
/// written and what the snippets produce. `else` is the exception: it belongs
/// with the `}` before it.
fn starts_a_line(token: &Token, previous: Option<&Item>, source: &str) -> bool {
    let Some(keyword) = token.keyword() else {
        return false;
    };
    if keyword == Keyword::Else {
        return false;
    }
    if keyword == Keyword::Procedure {
        return true;
    }
    if !keyword.starts_step() {
        return false;
    }
    match previous {
        // A keyword after `.` is a path segment; after a verb it is the operand.
        Some(Item::Token(previous)) => {
            !matches!(
                previous.kind,
                TokenKind::Dot
                    | TokenKind::Assign
                    | TokenKind::Bang
                    | TokenKind::LParen
                    | TokenKind::EqEq
                    | TokenKind::BangEq
                    | TokenKind::Lt
                    | TokenKind::Le
                    | TokenKind::Gt
                    | TokenKind::Ge
                    | TokenKind::AndAnd
                    | TokenKind::OrOr
                    | TokenKind::Minus
            ) && !matches!(
                previous.keyword(),
                Some(
                    Keyword::Check
                        | Keyword::Set
                        | Keyword::Start
                        | Keyword::Stop
                        | Keyword::Open
                        | Keyword::Close
                        | Keyword::Call
                        | Keyword::Category
                        | Keyword::Timeout
                        | Keyword::When
                        | Keyword::Procedure
                )
            )
        }
        // After a comment, only if the author had it on its own line.
        Some(Item::Comment(trivia)) => {
            source[trivia.span.end as usize..token.span.start as usize].contains('\n')
                || trivia.kind == TriviaKind::Line
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLES: &[(&str, &str)] = &[
        (
            "hydraulic.fe",
            include_str!("../../../examples/dc10/hydraulic.fe"),
        ),
        (
            "electrical.fe",
            include_str!("../../../examples/dc10/electrical.fe"),
        ),
        ("fuel.fe", include_str!("../../../examples/dc10/fuel.fe")),
        (
            "pressurization.fe",
            include_str!("../../../examples/dc10/pressurization.fe"),
        ),
    ];

    /// Formatting twice has to be the same as formatting once, or every save
    /// produces a diff and the file never settles.
    #[test]
    fn formatting_is_idempotent() {
        for (name, source) in EXAMPLES {
            let once = format(source).unwrap_or_else(|| source.to_string());
            let twice = format(&once).unwrap_or_else(|| once.clone());
            assert_eq!(once, twice, "{name} does not settle");
        }
    }

    /// The formatter's output has to be the same procedures it was given.
    #[test]
    fn formatting_preserves_the_tokens() {
        fn tokens(source: &str) -> Vec<String> {
            let mut diagnostics = fe_lang::diagnostics::Diagnostics::new();
            fe_lang::lexer::tokenize(UnitId(0), source, &mut diagnostics)
                .iter()
                .map(|token| token.text.to_string())
                .collect()
        }

        for (name, source) in EXAMPLES {
            let formatted = format(source).unwrap_or_else(|| source.to_string());
            assert_eq!(tokens(source), tokens(&formatted), "{name} changed meaning");
        }
    }

    #[test]
    fn every_comment_survives() {
        for (name, source) in EXAMPLES {
            let formatted = format(source).unwrap_or_else(|| source.to_string());
            let mut diagnostics = fe_lang::diagnostics::Diagnostics::new();
            let (_, before) = tokenize_with_trivia(UnitId(0), source, &mut diagnostics);
            let (_, after) = tokenize_with_trivia(UnitId(0), &formatted, &mut diagnostics);

            let text = |source: &str, list: &[Trivia]| -> Vec<String> {
                list.iter()
                    .map(|t| {
                        source[t.span.start as usize..t.span.end as usize]
                            .trim()
                            .to_string()
                    })
                    .collect()
            };
            assert_eq!(text(source, &before), text(&formatted, &after), "{name}");
        }
    }

    #[test]
    fn a_squashed_procedure_is_laid_out() {
        let formatted = format("procedure P{name \"P\" category normal check A complete}").unwrap();
        assert_eq!(
            formatted,
            "procedure P {\n    name \"P\"\n    category normal\n    check A\n    complete\n}\n"
        );
    }

    #[test]
    fn nesting_indents() {
        let formatted =
            format("procedure P{name \"P\" category normal if a {complete} else {fail}}").unwrap();
        assert_eq!(
            formatted,
            "procedure P {\n    name \"P\"\n    category normal\n    if a {\n        complete\n    } else {\n        fail\n    }\n}\n"
        );
    }

    /// A path is one thing, and the keywords inside it are path segments.
    #[test]
    fn a_path_is_not_broken_up() {
        let formatted =
            format("procedure P{name \"P\" category normal wait hydraulic.name.check > 1}")
                .unwrap();
        assert!(
            formatted.contains("wait hydraulic.name.check > 1"),
            "{formatted}"
        );
    }

    #[test]
    fn nothing_to_do_is_no_edit() {
        let tidy = "procedure P {\n    name \"P\"\n    category normal\n    complete\n}\n";
        assert_eq!(format(tidy), None);
    }

    #[test]
    fn a_file_that_does_not_parse_is_left_alone() {
        assert_eq!(format("procedure P {\n    wait a < b < c\n"), None);
        assert_eq!(format("procedure P { notify \"unterminated"), None);
    }

    /// At most one blank line: a paragraph break is the author's, a screenful
    /// of them is not.
    #[test]
    fn blank_lines_are_kept_but_collapsed() {
        let formatted = format(
            "procedure P {\n    name \"P\"\n    category normal\n\n\n\n    // a note\n    complete\n}\n",
        )
        .unwrap();
        assert!(
            formatted.contains("category normal\n\n    // a note"),
            "{formatted:?}"
        );
    }
}
