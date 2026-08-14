//! What the cursor is in the middle of writing, and what could go there.
//!
//! Completion cannot be driven by the syntax tree: text being typed usually
//! does not parse, and that is exactly when it is wanted. So this works from
//! the token stream, which the lexer produces for any input at all.
//!
//! Working from tokens rather than from a regular expression over the line is
//! what makes the language's contextual keywords fall out for free.
//! `hydraulic.name.check` is three path segments because each follows a `.`,
//! and `set complete.timeout = OPEN` parses because `complete` follows `set`.
//! Neither needs a special case here; both are decided by the same predecessor
//! rules the parser uses.

use fe_lang::diagnostics::{Diagnostics, codes};
use fe_lang::span::UnitId;
use fe_lang::token::{Keyword, Token, TokenKind};

pub const CATEGORIES: &[&str] = &["normal", "abnormal", "emergency", "reference"];

/// Offered when the file has not used anything better yet.
pub const COMMON_DURATIONS: &[&str] = &["500ms", "1s", "5s", "10s", "30s", "1m", "5m"];

#[derive(Clone, Debug, PartialEq)]
pub enum Context {
    /// Inside a string or a comment. Suggesting anything here is noise.
    None,
    /// Between procedures.
    Top,
    /// Inside a procedure that has not started its steps.
    Metadata,
    /// Inside a procedure body or an `if` block.
    Step,
    /// After `category`.
    Category,
    /// After a verb that names a control.
    Control { verb: crate::locate::ControlVerb },
    /// After `set CONTROL =`, where the valid set depends on the control.
    Position { control: String },
    /// After `timeout`.
    Duration,
    /// After `call`.
    Procedure,
    /// Inside a condition.
    Expression,
}

/// Classify the cursor from everything before it.
pub fn context_at(prefix: &str) -> Context {
    let mut diagnostics = Diagnostics::new();
    let (tokens, trivia) =
        fe_lang::lexer::tokenize_with_trivia(UnitId(0), prefix, &mut diagnostics);
    let end = prefix.len() as u32;

    // A comment running to the cursor, or a string the lexer never saw closed,
    // means the cursor is inside one of them.
    if trivia.last().is_some_and(|t| t.span.end >= end) {
        return Context::None;
    }
    if diagnostics.iter().any(|d| {
        matches!(
            d.code,
            codes::UNTERMINATED_STRING | codes::UNTERMINATED_COMMENT
        ) && d.primary.span.end >= end
    }) {
        return Context::None;
    }

    let tokens: Vec<&Token> = tokens.iter().filter(|t| t.kind != TokenKind::Eof).collect();
    let depth = tokens.iter().fold(0i32, |depth, token| match token.kind {
        TokenKind::LBrace => depth + 1,
        TokenKind::RBrace => (depth - 1).max(0),
        _ => depth,
    });

    // `procedure ` wants a new identifier; nothing known can be offered.
    if matches!(tokens.last(), Some(t) if t.is_keyword(Keyword::Procedure)) {
        return Context::None;
    }

    if depth == 0 {
        return Context::Top;
    }

    // Only the statement being written matters, and its extent is decided by
    // newlines.
    //
    // The parser does not care about newlines — `check A check B` on one line
    // is two steps — so nothing in the grammar says where a statement stops. But
    // `fail` with no message and `complete` with no condition are both whole
    // statements, so after either of them the tokens alone cannot say whether
    // the next word continues it or begins something new. The author's line
    // break can, and does: every snippet, every example and every procedure in
    // this repository is written one statement to a line.
    let window = current_statement(prefix, &tokens);
    let head = statement_head(window);
    let statement = &window[head..];

    match statement.first().and_then(|t| t.keyword()) {
        Some(Keyword::Category) => Context::Category,
        Some(Keyword::Call) => Context::Procedure,
        Some(Keyword::Check) => Context::Control {
            verb: crate::locate::ControlVerb::Check,
        },
        Some(Keyword::Start) => Context::Control {
            verb: crate::locate::ControlVerb::Start,
        },
        Some(Keyword::Stop) => Context::Control {
            verb: crate::locate::ControlVerb::Stop,
        },
        Some(Keyword::Open) => Context::Control {
            verb: crate::locate::ControlVerb::Open,
        },
        Some(Keyword::Close) => Context::Control {
            verb: crate::locate::ControlVerb::Close,
        },
        Some(Keyword::Set) => match statement.iter().position(|t| t.kind == TokenKind::Assign) {
            Some(assign) => Context::Position {
                control: joined(&statement[1..assign]),
            },
            None => Context::Control {
                verb: crate::locate::ControlVerb::Set,
            },
        },
        Some(
            Keyword::Trigger | Keyword::Require | Keyword::Wait | Keyword::If | Keyword::Complete,
        ) => {
            // `timeout` ends a condition and starts a duration.
            match statement
                .iter()
                .rposition(|t| t.is_keyword(Keyword::Timeout))
            {
                Some(index) if statement.len() - index <= 2 => Context::Duration,
                Some(_) => Context::None,
                None => Context::Expression,
            }
        }
        // A string or a number is expected; the editor has nothing to add.
        Some(
            Keyword::Name
            | Keyword::Description
            | Keyword::Notify
            | Keyword::Fail
            | Keyword::Priority
            | Keyword::Revision,
        ) => Context::None,
        // At the start of a statement: metadata until the first step, then steps.
        _ => {
            if depth > 1 || has_started_steps(&tokens) {
                Context::Step
            } else {
                Context::Metadata
            }
        }
    }
}

/// The tokens of the statement the cursor is in: those on its line, plus
/// earlier lines while the line is plainly a continuation of one.
///
/// A condition broken across lines is the case that matters —
///
/// ```text
/// trigger hydraulic.2.pressure < 1800
///      && engine.2.running
/// ```
///
/// — and it announces itself, because a line beginning with an operator, `=`,
/// `timeout`, `when` or `else` cannot be the start of anything.
fn current_statement<'a, 'b>(prefix: &str, tokens: &'b [&'a Token<'a>]) -> &'b [&'a Token<'a>] {
    let mut line_start = prefix.rfind('\n').map(|index| index + 1).unwrap_or(0) as u32;

    loop {
        let first = tokens.iter().position(|t| t.span.start >= line_start);
        let Some(first) = first else {
            // Nothing on this line: the cursor is starting a fresh statement.
            return &tokens[tokens.len()..];
        };
        if line_start == 0 || !is_continuation(tokens[first]) {
            return &tokens[first..];
        }
        line_start = prefix[..line_start as usize - 1]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0) as u32;
    }
}

fn is_continuation(token: &Token) -> bool {
    matches!(
        token.kind,
        TokenKind::AndAnd
            | TokenKind::OrOr
            | TokenKind::EqEq
            | TokenKind::BangEq
            | TokenKind::Lt
            | TokenKind::Le
            | TokenKind::Gt
            | TokenKind::Ge
            | TokenKind::Assign
            | TokenKind::Dot
    ) || matches!(
        token.keyword(),
        Some(Keyword::Timeout | Keyword::When | Keyword::Else)
    )
}

/// Index of the token that starts the statement the cursor is in.
///
/// Scanning backwards, the first statement keyword found is the head — as long
/// as it really is one. A keyword after `.` is a path segment, and a keyword
/// after something that expects an operand is that operand.
fn statement_head(tokens: &[&Token]) -> usize {
    for index in (0..tokens.len()).rev() {
        let token = &tokens[index];
        if matches!(token.kind, TokenKind::LBrace | TokenKind::RBrace) {
            return index + 1;
        }
        let Some(keyword) = token.keyword() else {
            continue;
        };
        if !keyword.starts_step() {
            continue;
        }
        if index > 0 && !starts_statement_after(&tokens[index - 1]) {
            continue;
        }
        return index;
    }
    0
}

/// Whether a statement keyword following `previous` is really a statement
/// keyword, rather than a path segment or an operand.
fn starts_statement_after(previous: &Token) -> bool {
    if matches!(
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
    ) {
        return false;
    }
    !matches!(
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

/// Whether the innermost procedure has moved past its metadata.
///
/// Metadata after the first step is E0105, so once a step has been written the
/// metadata entries are no longer worth offering.
fn has_started_steps(tokens: &[&Token]) -> bool {
    let body = tokens
        .iter()
        .rposition(|t| t.kind == TokenKind::LBrace)
        .map(|index| index + 1)
        .unwrap_or(0);

    tokens[body..].iter().enumerate().any(|(offset, token)| {
        let index = body + offset;
        token.keyword().is_some_and(|keyword| {
            matches!(
                keyword,
                Keyword::Check
                    | Keyword::Set
                    | Keyword::Start
                    | Keyword::Stop
                    | Keyword::Open
                    | Keyword::Close
                    | Keyword::Notify
                    | Keyword::Call
                    | Keyword::Wait
                    | Keyword::If
                    | Keyword::Complete
                    | Keyword::Fail
            )
        }) && (index == body || starts_statement_after(&tokens[index - 1]))
    })
}

fn joined(tokens: &[&Token]) -> String {
    tokens.iter().map(|t| t.text).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locate::ControlVerb;

    /// `|` marks the cursor.
    #[track_caller]
    fn at(source: &str) -> Context {
        let (before, _) = source.split_once('|').expect("mark the cursor with |");
        context_at(before)
    }

    fn wrap(body: &str) -> String {
        format!("procedure P {{\n    name \"P\"\n    category normal\n    {body}")
    }

    #[test]
    fn between_procedures() {
        assert_eq!(at("|"), Context::Top);
        assert_eq!(at("procedure P {\n}\n|"), Context::Top);
        assert_eq!(at("proc|"), Context::Top);
    }

    #[test]
    fn a_new_procedure_name_is_the_authors_to_invent() {
        assert_eq!(at("procedure |"), Context::None);
    }

    #[test]
    fn metadata_until_the_first_step() {
        assert_eq!(at("procedure P {\n    |"), Context::Metadata);
        assert_eq!(
            at("procedure P {\n    name \"P\"\n    |"),
            Context::Metadata
        );
        assert_eq!(
            at("procedure P {\n    name \"P\"\n    check A\n    |"),
            Context::Step
        );
    }

    #[test]
    fn control_verbs() {
        for (verb, keyword) in [
            (ControlVerb::Check, "check"),
            (ControlVerb::Set, "set"),
            (ControlVerb::Start, "start"),
            (ControlVerb::Stop, "stop"),
            (ControlVerb::Open, "open"),
            (ControlVerb::Close, "close"),
        ] {
            assert_eq!(
                at(&wrap(&format!("{keyword} |"))),
                Context::Control { verb },
                "after `{keyword}`"
            );
            assert_eq!(
                at(&wrap(&format!("{keyword} HYD_2|"))),
                Context::Control { verb },
                "part way through a name after `{keyword}`"
            );
        }
    }

    #[test]
    fn a_position_knows_which_control_it_belongs_to() {
        assert_eq!(
            at(&wrap("set FUEL_XFEED_SELECTOR = |")),
            Context::Position {
                control: "FUEL_XFEED_SELECTOR".to_string()
            }
        );
        assert_eq!(
            at(&wrap("set FUEL_XFEED_SELECTOR = TANK|")),
            Context::Position {
                control: "FUEL_XFEED_SELECTOR".to_string()
            }
        );
    }

    #[test]
    fn conditions() {
        assert_eq!(at(&wrap("wait |")), Context::Expression);
        assert_eq!(at(&wrap("wait hydraulic.2.|")), Context::Expression);
        assert_eq!(at(&wrap("if a && |")), Context::Expression);
        assert_eq!(at("procedure P {\n    trigger |"), Context::Expression);
        assert_eq!(at("procedure P {\n    require |"), Context::Expression);
        assert_eq!(at(&wrap("complete when |")), Context::Expression);
    }

    /// A condition split across lines is still a condition. The continuation
    /// announces itself: a line cannot start with `&&`.
    #[test]
    fn a_condition_may_span_lines() {
        assert_eq!(
            at("procedure P {\n    trigger hydraulic.2.pressure < 1800\n         && |"),
            Context::Expression
        );
        assert_eq!(
            at(&wrap("wait a > 1\n        timeout |")),
            Context::Duration
        );
        assert_eq!(
            at(&wrap("set FUEL_XFEED_SELECTOR\n        = |")),
            Context::Position {
                control: "FUEL_XFEED_SELECTOR".to_string()
            }
        );
    }

    /// …but a line that could start a statement does.
    #[test]
    fn a_new_line_starts_a_new_statement() {
        assert_eq!(at(&wrap("fail\n    |")), Context::Step);
        assert_eq!(at(&wrap("complete\n    |")), Context::Step);
        assert_eq!(at(&wrap("check A\n    |")), Context::Step);
        assert_eq!(
            at("procedure P {\n    name \"P\"\n    |"),
            Context::Metadata
        );
    }

    #[test]
    fn durations_after_timeout() {
        assert_eq!(at(&wrap("wait a > 1 timeout |")), Context::Duration);
        assert_eq!(at(&wrap("wait a > 1 timeout 30|")), Context::Duration);
    }

    #[test]
    fn call_targets() {
        assert_eq!(at(&wrap("call |")), Context::Procedure);
        assert_eq!(at(&wrap("call HYD_|")), Context::Procedure);
    }

    #[test]
    fn categories() {
        assert_eq!(at("procedure P {\n    category |"), Context::Category);
        assert_eq!(at("procedure P {\n    category abn|"), Context::Category);
    }

    #[test]
    fn inside_an_if_block_is_a_step() {
        assert_eq!(at(&wrap("if a {\n        |")), Context::Step);
        assert_eq!(
            at(&wrap("if a {\n        check X\n    } else {\n        |")),
            Context::Step
        );
    }

    /// The contextual-keyword cases. Both are legal source, and neither should
    /// be mistaken for the start of a statement.
    #[test]
    fn a_keyword_after_a_dot_is_a_path_segment() {
        assert_eq!(at(&wrap("wait hydraulic.name.check|")), Context::Expression);
        assert_eq!(
            at(&wrap("check hydraulic.name.|")),
            Context::Control {
                verb: ControlVerb::Check
            }
        );
    }

    #[test]
    fn a_keyword_after_set_is_the_control() {
        assert_eq!(
            at(&wrap("set complete.timeout = |")),
            Context::Position {
                control: "complete.timeout".to_string()
            }
        );
    }

    #[test]
    fn strings_and_comments_are_left_alone() {
        assert_eq!(at(&wrap("notify \"press the |")), Context::None);
        assert_eq!(at(&wrap("// remember to |")), Context::None);
        assert_eq!(at(&wrap("/* a note\n       about |")), Context::None);
        // …but a comment does not swallow the line after it.
        assert_eq!(at(&wrap("// a note\n    |")), Context::Metadata);
    }

    /// A `{` inside a message must not be counted as a block.
    #[test]
    fn a_brace_in_a_message_does_not_open_a_block() {
        assert_eq!(
            at("procedure P {\n    notify \"{{{\"\n    |"),
            Context::Step
        );
    }

    #[test]
    fn a_string_or_number_metadata_value_offers_nothing() {
        assert_eq!(at("procedure P {\n    name |"), Context::None);
        assert_eq!(at("procedure P {\n    priority |"), Context::None);
        assert_eq!(at(&wrap("notify |")), Context::None);
        assert_eq!(at(&wrap("fail |")), Context::None);
    }
}
