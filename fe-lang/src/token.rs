use crate::span::Span;

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    Ident,
    Number(f64),
    Str(String),
    Duration(u32),
    LBrace,
    RBrace,
    LParen,
    RParen,
    Dot,
    Minus,
    Assign,
    EqEq,
    BangEq,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    Bang,
    Eof,
    Error,
}

impl TokenKind {
    pub fn describe(&self) -> String {
        match self {
            TokenKind::Ident => "identifier".to_string(),
            TokenKind::Number(_) => "number".to_string(),
            TokenKind::Str(_) => "string".to_string(),
            TokenKind::Duration(_) => "duration".to_string(),
            TokenKind::Eof => "end of file".to_string(),
            TokenKind::Error => "invalid token".to_string(),
            other => format!("`{}`", other.punctuation().unwrap_or("?")),
        }
    }

    pub fn punctuation(&self) -> Option<&'static str> {
        Some(match self {
            TokenKind::LBrace => "{",
            TokenKind::RBrace => "}",
            TokenKind::LParen => "(",
            TokenKind::RParen => ")",
            TokenKind::Dot => ".",
            TokenKind::Minus => "-",
            TokenKind::Assign => "=",
            TokenKind::EqEq => "==",
            TokenKind::BangEq => "!=",
            TokenKind::Lt => "<",
            TokenKind::Le => "<=",
            TokenKind::Gt => ">",
            TokenKind::Ge => ">=",
            TokenKind::AndAnd => "&&",
            TokenKind::OrOr => "||",
            TokenKind::Bang => "!",
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Token<'a> {
    pub kind: TokenKind,
    pub span: Span,
    pub text: &'a str,
}

impl<'a> Token<'a> {
    pub fn is(&self, kind: &TokenKind) -> bool {
        core::mem::discriminant(&self.kind) == core::mem::discriminant(kind)
    }

    pub fn keyword(&self) -> Option<Keyword> {
        if self.kind != TokenKind::Ident {
            return None;
        }
        Keyword::from_str(self.text)
    }

    pub fn is_keyword(&self, keyword: Keyword) -> bool {
        self.keyword() == Some(keyword)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Keyword {
    Procedure,
    Name,
    Description,
    Category,
    Priority,
    Revision,
    Trigger,
    Require,
    If,
    Else,
    Set,
    Check,
    Start,
    Stop,
    Open,
    Close,
    Notify,
    Call,
    Wait,
    Timeout,
    Complete,
    When,
    Fail,
    True,
    False,
}

impl Keyword {
    pub fn from_str(text: &str) -> Option<Keyword> {
        Some(match text {
            "procedure" => Keyword::Procedure,
            "name" => Keyword::Name,
            "description" => Keyword::Description,
            "category" => Keyword::Category,
            "priority" => Keyword::Priority,
            "revision" => Keyword::Revision,
            "trigger" => Keyword::Trigger,
            "require" => Keyword::Require,
            "if" => Keyword::If,
            "else" => Keyword::Else,
            "set" => Keyword::Set,
            "check" => Keyword::Check,
            "start" => Keyword::Start,
            "stop" => Keyword::Stop,
            "open" => Keyword::Open,
            "close" => Keyword::Close,
            "notify" => Keyword::Notify,
            "call" => Keyword::Call,
            "wait" => Keyword::Wait,
            "timeout" => Keyword::Timeout,
            "complete" => Keyword::Complete,
            "when" => Keyword::When,
            "fail" => Keyword::Fail,
            "true" => Keyword::True,
            "false" => Keyword::False,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Keyword::Procedure => "procedure",
            Keyword::Name => "name",
            Keyword::Description => "description",
            Keyword::Category => "category",
            Keyword::Priority => "priority",
            Keyword::Revision => "revision",
            Keyword::Trigger => "trigger",
            Keyword::Require => "require",
            Keyword::If => "if",
            Keyword::Else => "else",
            Keyword::Set => "set",
            Keyword::Check => "check",
            Keyword::Start => "start",
            Keyword::Stop => "stop",
            Keyword::Open => "open",
            Keyword::Close => "close",
            Keyword::Notify => "notify",
            Keyword::Call => "call",
            Keyword::Wait => "wait",
            Keyword::Timeout => "timeout",
            Keyword::Complete => "complete",
            Keyword::When => "when",
            Keyword::Fail => "fail",
            Keyword::True => "true",
            Keyword::False => "false",
        }
    }

    pub fn starts_step(self) -> bool {
        matches!(
            self,
            Keyword::If
                | Keyword::Set
                | Keyword::Check
                | Keyword::Start
                | Keyword::Stop
                | Keyword::Open
                | Keyword::Close
                | Keyword::Notify
                | Keyword::Call
                | Keyword::Wait
                | Keyword::Complete
                | Keyword::Fail
                | Keyword::Require
                | Keyword::Name
                | Keyword::Description
                | Keyword::Category
                | Keyword::Priority
                | Keyword::Revision
                | Keyword::Trigger
        )
    }
}
