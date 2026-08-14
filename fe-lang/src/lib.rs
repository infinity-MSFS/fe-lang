pub mod ast;
pub mod diagnostics;
pub mod lexer;
pub mod parser;
pub mod span;
pub mod token;

pub use ast::Ast;
pub use diagnostics::{codes, Diagnostic, Diagnostics, Label, Severity};
pub use lexer::{Trivia, TriviaKind};
pub use span::{Location, SourceMap, SourceUnit, Span, UnitId};
pub use token::{Keyword, Token, TokenKind};

pub fn parse_unit(unit: UnitId, source: &str, diagnostics: &mut Diagnostics) -> Ast {
    let tokens = lexer::tokenize(unit, source, diagnostics);
    parser::parse(unit, &tokens, diagnostics)
}
