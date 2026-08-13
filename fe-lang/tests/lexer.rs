use fe_lang::diagnostics::Diagnostics;
use fe_lang::lexer::tokenize;
use fe_lang::span::UnitId;
use fe_lang::token::{Keyword, TokenKind};

fn lex(source: &str) -> (Vec<TokenKind>, Diagnostics) {
    let mut diagnostics = Diagnostics::new();
    let tokens = tokenize(UnitId(0), source, &mut diagnostics);
    (tokens.into_iter().map(|t| t.kind).collect(), diagnostics)
}

fn lex_ok(source: &str) -> Vec<TokenKind> {
    let (kinds, diagnostics) = lex(source);
    assert!(
        !diagnostics.has_errors(),
        "unexpected lexical errors in {source:?}"
    );
    kinds
}

#[test]
fn identifiers_and_keywords() {
    let mut diagnostics = Diagnostics::new();
    let tokens = tokenize(UnitId(0), "procedure HYD_2 wait _x9", &mut diagnostics);
    assert!(!diagnostics.has_errors());
    assert_eq!(tokens[0].keyword(), Some(Keyword::Procedure));
    assert_eq!(tokens[1].keyword(), None);
    assert_eq!(tokens[1].text, "HYD_2");
    assert_eq!(tokens[2].keyword(), Some(Keyword::Wait));
    assert_eq!(tokens[3].text, "_x9");
    assert_eq!(tokens[4].kind, TokenKind::Eof);
}

#[test]
fn numbers() {
    assert_eq!(lex_ok("1800")[0], TokenKind::Number(1800.0));
    assert_eq!(lex_ok("22.5")[0], TokenKind::Number(22.5));
    assert_eq!(lex_ok("0")[0], TokenKind::Number(0.0));
}

#[test]
fn a_dot_after_digits_is_a_path_separator_not_a_decimal_point() {
    let kinds = lex_ok("hydraulic.2.pressure");
    assert_eq!(
        kinds,
        vec![
            TokenKind::Ident,
            TokenKind::Dot,
            TokenKind::Number(2.0),
            TokenKind::Dot,
            TokenKind::Ident,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn durations() {
    assert_eq!(lex_ok("10s")[0], TokenKind::Duration(10_000));
    assert_eq!(lex_ok("250ms")[0], TokenKind::Duration(250));
    assert_eq!(lex_ok("2m")[0], TokenKind::Duration(120_000));
    assert_eq!(lex_ok("1.5s")[0], TokenKind::Duration(1500));
}

#[test]
fn unknown_duration_unit_is_reported() {
    let (_, diagnostics) = lex("10h");
    assert_eq!(diagnostics.errors().next().unwrap().code, "E0006");
}

#[test]
fn strings_with_escapes() {
    let kinds = lex_ok(r#" "low \"pressure\"\nnow" "#);
    assert_eq!(
        kinds[0],
        TokenKind::Str("low \"pressure\"\nnow".to_string())
    );
}

#[test]
fn strings_keep_non_ascii() {
    let kinds = lex_ok("\"pression hydraulique — basse\"");
    assert_eq!(
        kinds[0],
        TokenKind::Str("pression hydraulique — basse".to_string())
    );
}

#[test]
fn unterminated_string_is_reported_once() {
    let (_, diagnostics) = lex("\"never closed");
    assert_eq!(diagnostics.errors().count(), 1);
    assert_eq!(diagnostics.errors().next().unwrap().code, "E0002");
}

#[test]
fn string_may_not_span_lines() {
    let (_, diagnostics) = lex("\"open\nclosed\"");
    assert_eq!(diagnostics.errors().next().unwrap().code, "E0002");
}

#[test]
fn unknown_escape_is_reported() {
    let (_, diagnostics) = lex(r#""bad \q escape""#);
    assert_eq!(diagnostics.errors().next().unwrap().code, "E0003");
}

#[test]
fn operators() {
    assert_eq!(
        lex_ok("< <= > >= == != && || ! = ( ) { } . -"),
        vec![
            TokenKind::Lt,
            TokenKind::Le,
            TokenKind::Gt,
            TokenKind::Ge,
            TokenKind::EqEq,
            TokenKind::BangEq,
            TokenKind::AndAnd,
            TokenKind::OrOr,
            TokenKind::Bang,
            TokenKind::Assign,
            TokenKind::LParen,
            TokenKind::RParen,
            TokenKind::LBrace,
            TokenKind::RBrace,
            TokenKind::Dot,
            TokenKind::Minus,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn single_ampersand_suggests_the_double() {
    let (_, diagnostics) = lex("a & b");
    let diagnostic = diagnostics.errors().next().unwrap();
    assert_eq!(diagnostic.code, "E0001");
    assert!(diagnostic.primary.message.as_ref().unwrap().contains("&&"));
}

#[test]
fn comments_are_trivia() {
    let kinds = lex_ok(
        r#"
        // a line comment
        set /* inline */ X
        /* multi
           line */
        "#,
    );
    assert_eq!(
        kinds,
        vec![TokenKind::Ident, TokenKind::Ident, TokenKind::Eof]
    );
}

#[test]
fn unterminated_block_comment_is_reported() {
    let (_, diagnostics) = lex("/* forever");
    assert_eq!(diagnostics.errors().next().unwrap().code, "E0005");
}

#[test]
fn unexpected_character_is_reported_and_recovered() {
    let (kinds, diagnostics) = lex("a $ b");
    assert_eq!(diagnostics.errors().count(), 1);
    assert_eq!(kinds.len(), 4);
}

#[test]
fn spans_point_at_the_token() {
    let mut diagnostics = Diagnostics::new();
    let source = "procedure FOO";
    let tokens = tokenize(UnitId(0), source, &mut diagnostics);
    let span = tokens[1].span;
    assert_eq!(&source[span.start as usize..span.end as usize], "FOO");
}

#[test]
fn non_ascii_outside_a_string_does_not_split_a_character() {
    let (_, diagnostics) = lex("é");
    assert_eq!(diagnostics.errors().count(), 1);
}
