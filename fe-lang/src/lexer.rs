use crate::diagnostics::{Diagnostic, Diagnostics, Label, codes};
use crate::span::{Span, UnitId};
use crate::token::{Token, TokenKind};

pub struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    unit: UnitId,
}

pub fn tokenize<'a>(unit: UnitId, src: &'a str, diagnostics: &mut Diagnostics) -> Vec<Token<'a>> {
    Lexer::new(unit, src).run(diagnostics)
}

impl<'a> Lexer<'a> {
    pub fn new(unit: UnitId, src: &'a str) -> Lexer<'a> {
        Lexer {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            unit,
        }
    }

    fn span(&self, start: usize) -> Span {
        Span::new(self.unit, start, self.pos)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.bytes.get(self.pos + ahead).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn token(&self, kind: TokenKind, start: usize) -> Token<'a> {
        Token {
            kind,
            span: Span::new(self.unit, start, self.pos),
            text: self.src.get(start..self.pos).unwrap_or(""),
        }
    }

    pub fn run(mut self, diagnostics: &mut Diagnostics) -> Vec<Token<'a>> {
        let mut tokens = Vec::new();
        loop {
            self.skip_trivia(diagnostics);
            let start = self.pos;
            let Some(b) = self.peek() else { break };

            let kind = match b {
                b'{' => self.single(TokenKind::LBrace),
                b'}' => self.single(TokenKind::RBrace),
                b'(' => self.single(TokenKind::LParen),
                b')' => self.single(TokenKind::RParen),
                b'.' => self.single(TokenKind::Dot),
                b'-' => self.single(TokenKind::Minus),
                b'=' => {
                    self.pos += 1;
                    if self.peek() == Some(b'=') {
                        self.pos += 1;
                        TokenKind::EqEq
                    } else {
                        TokenKind::Assign
                    }
                }
                b'!' => {
                    self.pos += 1;
                    if self.peek() == Some(b'=') {
                        self.pos += 1;
                        TokenKind::BangEq
                    } else {
                        TokenKind::Bang
                    }
                }
                b'<' => {
                    self.pos += 1;
                    if self.peek() == Some(b'=') {
                        self.pos += 1;
                        TokenKind::Le
                    } else {
                        TokenKind::Lt
                    }
                }
                b'>' => {
                    self.pos += 1;
                    if self.peek() == Some(b'=') {
                        self.pos += 1;
                        TokenKind::Ge
                    } else {
                        TokenKind::Gt
                    }
                }
                b'&' => {
                    self.pos += 1;
                    if self.peek() == Some(b'&') {
                        self.pos += 1;
                        TokenKind::AndAnd
                    } else {
                        diagnostics.push(
                            Diagnostic::error(
                                codes::UNEXPECTED_CHARACTER,
                                "unexpected character `&`",
                                Label::new(self.span(start), "did you mean `&&`?"),
                            )
                            .with_note("the language has no bitwise operators"),
                        );
                        TokenKind::Error
                    }
                }
                b'|' => {
                    self.pos += 1;
                    if self.peek() == Some(b'|') {
                        self.pos += 1;
                        TokenKind::OrOr
                    } else {
                        diagnostics.push(Diagnostic::error(
                            codes::UNEXPECTED_CHARACTER,
                            "unexpected character `|`",
                            Label::new(self.span(start), "did you mean `||`?"),
                        ));
                        TokenKind::Error
                    }
                }
                b'"' => self.string(diagnostics),
                b'0'..=b'9' => self.number(diagnostics),
                b if b.is_ascii_alphabetic() || b == b'_' => {
                    while self
                        .peek()
                        .map(|b| b.is_ascii_alphanumeric() || b == b'_')
                        .unwrap_or(false)
                    {
                        self.pos += 1;
                    }
                    TokenKind::Ident
                }
                other => {
                    self.pos += 1;
                    while self.pos < self.bytes.len() && (self.bytes[self.pos] & 0xC0) == 0x80 {
                        self.pos += 1;
                    }
                    let text = self.src.get(start..self.pos).unwrap_or("?");
                    diagnostics.push(Diagnostic::error(
                        codes::UNEXPECTED_CHARACTER,
                        format!("unexpected character `{text}`"),
                        Label::bare(self.span(start)),
                    ));
                    let _ = other;
                    TokenKind::Error
                }
            };
            tokens.push(self.token(kind, start));
        }
        let start = self.pos;
        tokens.push(self.token(TokenKind::Eof, start));
        tokens
    }

    fn single(&mut self, kind: TokenKind) -> TokenKind {
        self.pos += 1;
        kind
    }

    fn skip_trivia(&mut self, diagnostics: &mut Diagnostics) {
        loop {
            match self.peek() {
                Some(b) if b.is_ascii_whitespace() => {
                    self.pos += 1;
                }
                Some(b'/') if self.peek_at(1) == Some(b'/') => {
                    while let Some(b) = self.peek() {
                        if b == b'\n' {
                            break;
                        }
                        self.pos += 1;
                    }
                }
                Some(b'/') if self.peek_at(1) == Some(b'*') => {
                    let start = self.pos;
                    self.pos += 2;
                    loop {
                        match self.peek() {
                            None => {
                                diagnostics.push(Diagnostic::error(
                                    codes::UNTERMINATED_COMMENT,
                                    "unterminated block comment",
                                    Label::new(self.span(start), "started here"),
                                ));
                                return;
                            }
                            Some(b'*') if self.peek_at(1) == Some(b'/') => {
                                self.pos += 2;
                                break;
                            }
                            _ => {
                                self.pos += 1;
                            }
                        }
                    }
                }
                _ => return,
            }
        }
    }

    fn string(&mut self, diagnostics: &mut Diagnostics) -> TokenKind {
        let start = self.pos;
        self.pos += 1; // opening quote
        let mut value = String::new();
        loop {
            let Some(b) = self.bump() else {
                diagnostics.push(Diagnostic::error(
                    codes::UNTERMINATED_STRING,
                    "unterminated string literal",
                    Label::new(self.span(start), "opened here"),
                ));
                return TokenKind::Error;
            };
            match b {
                b'"' => break,
                b'\n' => {
                    diagnostics.push(Diagnostic::error(
                        codes::UNTERMINATED_STRING,
                        "unterminated string literal",
                        Label::new(self.span(start), "string literals may not span lines"),
                    ));
                    return TokenKind::Error;
                }
                b'\\' => {
                    let escape_start = self.pos - 1;
                    match self.bump() {
                        Some(b'"') => value.push('"'),
                        Some(b'\\') => value.push('\\'),
                        Some(b'n') => value.push('\n'),
                        Some(b't') => value.push('\t'),
                        _ => {
                            diagnostics.push(
                                Diagnostic::error(
                                    codes::INVALID_ESCAPE,
                                    "unknown escape sequence",
                                    Label::bare(Span::new(self.unit, escape_start, self.pos)),
                                )
                                .with_note("valid escapes are \\\" \\\\ \\n \\t"),
                            );
                        }
                    }
                }
                _ => {
                    let char_start = self.pos - 1;
                    while self.pos < self.bytes.len() && (self.bytes[self.pos] & 0xC0) == 0x80 {
                        self.pos += 1;
                    }
                    value.push_str(self.src.get(char_start..self.pos).unwrap_or(""));
                }
            }
        }
        TokenKind::Str(value)
    }

    fn number(&mut self, diagnostics: &mut Diagnostics) -> TokenKind {
        let start = self.pos;
        while self.peek().map(|b| b.is_ascii_digit()).unwrap_or(false) {
            self.pos += 1;
        }
        if self.peek() == Some(b'.') && self.peek_at(1).map(|b| b.is_ascii_digit()).unwrap_or(false)
        {
            self.pos += 1;
            while self.peek().map(|b| b.is_ascii_digit()).unwrap_or(false) {
                self.pos += 1;
            }
        }
        let digits = self.src.get(start..self.pos).unwrap_or("0");
        let value: f64 = match digits.parse() {
            Ok(v) => v,
            Err(_) => {
                diagnostics.push(Diagnostic::error(
                    codes::MALFORMED_NUMBER,
                    format!("`{digits}` is not a valid number"),
                    Label::bare(self.span(start)),
                ));
                return TokenKind::Error;
            }
        };

        let unit_start = self.pos;
        while self
            .peek()
            .map(|b| b.is_ascii_alphabetic())
            .unwrap_or(false)
        {
            self.pos += 1;
        }
        if unit_start == self.pos {
            return TokenKind::Number(value);
        }
        let suffix = self.src.get(unit_start..self.pos).unwrap_or("");
        let millis = match suffix {
            "ms" => value,
            "s" => value * 1000.0,
            "m" => value * 60_000.0,
            other => {
                diagnostics.push(
                    Diagnostic::error(
                        codes::INVALID_DURATION,
                        format!("unknown duration unit `{other}`"),
                        Label::bare(Span::new(self.unit, unit_start, self.pos)),
                    )
                    .with_help("use `ms`, `s` or `m`"),
                );
                return TokenKind::Error;
            }
        };
        if !(0.0..=u32::MAX as f64).contains(&millis) {
            diagnostics.push(Diagnostic::error(
                codes::INVALID_DURATION,
                "duration is out of range",
                Label::bare(self.span(start)),
            ));
            return TokenKind::Error;
        }
        TokenKind::Duration(millis as u32)
    }
}
