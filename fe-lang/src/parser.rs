use crate::ast::*;
use crate::diagnostics::{Diagnostic, Diagnostics, Label, codes};
use crate::span::{Span, UnitId};
use crate::token::{Keyword, Token, TokenKind};

pub fn parse<'a>(unit: UnitId, tokens: &[Token<'a>], diagnostics: &mut Diagnostics) -> Ast {
    let _ = unit;
    Parser {
        tokens,
        pos: 0,
        diagnostics,
    }
    .file()
}

struct Parser<'a, 'b> {
    tokens: &'b [Token<'a>],
    pos: usize,
    diagnostics: &'b mut Diagnostics,
}

impl<'a, 'b> Parser<'a, 'b> {
    fn peek(&self) -> &Token<'a> {
        self.tokens
            .get(self.pos)
            .unwrap_or_else(|| self.tokens.last().expect("token stream is never empty"))
    }

    fn at_end(&self) -> bool {
        self.peek().kind == TokenKind::Eof
    }

    fn advance(&mut self) -> Token<'a> {
        let token = self.peek().clone();
        if !self.at_end() {
            self.pos += 1;
        }
        token
    }

    fn eat_keyword(&mut self, keyword: Keyword) -> Option<Token<'a>> {
        if self.peek().is_keyword(keyword) {
            Some(self.advance())
        } else {
            None
        }
    }

    fn error(&mut self, code: &'static str, message: impl Into<String>, label: Label) {
        self.diagnostics
            .push(Diagnostic::error(code, message, label));
    }

    fn expect(&mut self, kind: TokenKind, context: &str) -> Option<Token<'a>> {
        if self.peek().is(&kind) {
            return Some(self.advance());
        }
        let found = self.peek().kind.describe();
        let span = self.peek().span;
        self.error(
            codes::EXPECTED_TOKEN,
            format!("expected {} {context}", kind.describe()),
            Label::new(span, format!("found {found}")),
        );
        None
    }

    fn expect_ident(&mut self, context: &str) -> Option<Ident> {
        let token = self.peek().clone();
        if token.kind == TokenKind::Ident {
            self.advance();
            return Some(Ident {
                text: token.text.to_string(),
                span: token.span,
            });
        }
        let found = token.kind.describe();
        self.error(
            codes::EXPECTED_TOKEN,
            format!("expected an identifier {context}"),
            Label::new(token.span, format!("found {found}")),
        );
        None
    }

    fn expect_string(&mut self, context: &str) -> Option<Spanned<String>> {
        let token = self.peek().clone();
        if let TokenKind::Str(value) = token.kind {
            self.advance();
            return Some(Spanned::new(value, token.span));
        }
        let found = token.kind.describe();
        self.error(
            codes::EXPECTED_TOKEN,
            format!("expected a quoted string {context}"),
            Label::new(token.span, format!("found {found}")),
        );
        None
    }

    fn expect_number(&mut self, context: &str) -> Option<Spanned<f64>> {
        let token = self.peek().clone();
        if let TokenKind::Number(value) = token.kind {
            self.advance();
            return Some(Spanned::new(value, token.span));
        }
        let found = token.kind.describe();
        self.error(
            codes::EXPECTED_TOKEN,
            format!("expected a number {context}"),
            Label::new(token.span, format!("found {found}")),
        );
        None
    }

    fn file(mut self) -> Ast {
        let mut ast = Ast::default();
        while !self.at_end() {
            if self.peek().is_keyword(Keyword::Procedure) {
                if let Some(decl) = self.procedure() {
                    ast.procedures.push(decl);
                }
                continue;
            }
            let token = self.peek().clone();
            let found = token.kind.describe();
            self.error(
                codes::EXPECTED_DECLARATION,
                "expected a procedure declaration",
                Label::new(token.span, format!("found {found}")),
            );
            self.recover_to_declaration();
        }
        ast
    }

    fn recover_to_declaration(&mut self) {
        while !self.at_end() && !self.peek().is_keyword(Keyword::Procedure) {
            self.advance();
        }
    }

    fn procedure(&mut self) -> Option<ProcedureDecl> {
        let start = self.advance().span;
        let id = match self.expect_ident("for the procedure identifier") {
            Some(id) => id,
            None => {
                self.recover_to_declaration();
                return None;
            }
        };
        if self
            .expect(TokenKind::LBrace, "to open the procedure body")
            .is_none()
        {
            self.recover_to_declaration();
            return None;
        }

        let mut metadata = Metadata::default();
        let mut steps = Vec::new();
        let body_start = self.peek().span;

        while !self.at_end() && !self.peek().is(&TokenKind::RBrace) {
            let before = self.pos;
            self.item(&mut metadata, &mut steps);
            if self.pos == before {
                self.advance();
            }
        }
        let end = self
            .expect(TokenKind::RBrace, "to close the procedure body")
            .map(|t| t.span)
            .unwrap_or(self.peek().span);

        let body_span = if steps.is_empty() {
            body_start
        } else {
            steps[0].span().to(steps[steps.len() - 1].span())
        };

        Some(ProcedureDecl {
            id,
            metadata,
            body: Block {
                steps,
                span: body_span,
            },
            span: start.to(end),
        })
    }

    fn item(&mut self, metadata: &mut Metadata, steps: &mut Vec<Step>) {
        let token = self.peek().clone();
        let Some(keyword) = token.keyword() else {
            let found = token.kind.describe();
            self.error(
                codes::EXPECTED_STEP,
                "expected a step or metadata entry",
                Label::new(token.span, format!("found {found}")),
            );
            self.recover_in_body();
            return;
        };

        let is_metadata = matches!(
            keyword,
            Keyword::Name
                | Keyword::Description
                | Keyword::Category
                | Keyword::Priority
                | Keyword::Revision
                | Keyword::Trigger
                | Keyword::Require
        );

        if is_metadata {
            if !steps.is_empty() {
                self.error(
                    codes::METADATA_AFTER_STEPS,
                    format!("`{}` must appear before the first step", keyword.as_str()),
                    Label::new(token.span, "metadata belongs at the top of the procedure"),
                );
            }
            self.metadata_entry(keyword, metadata);
            return;
        }

        match self.step() {
            Some(step) => steps.push(step),
            None => self.recover_in_body(),
        }
    }

    fn duplicate(&mut self, keyword: Keyword, span: Span, previous: Span) {
        self.diagnostics.push(
            Diagnostic::error(
                codes::DUPLICATE_METADATA,
                format!("`{}` is specified more than once", keyword.as_str()),
                Label::new(span, "duplicate"),
            )
            .with_secondary(Label::new(previous, "first specified here")),
        );
    }

    fn metadata_entry(&mut self, keyword: Keyword, metadata: &mut Metadata) {
        let keyword_span = self.advance().span;
        match keyword {
            Keyword::Name => {
                if let Some(value) = self.expect_string("after `name`") {
                    match &metadata.name {
                        Some(previous) => self.duplicate(keyword, value.span, previous.span),
                        None => metadata.name = Some(value),
                    }
                }
            }
            Keyword::Description => {
                if let Some(value) = self.expect_string("after `description`") {
                    match &metadata.description {
                        Some(previous) => self.duplicate(keyword, value.span, previous.span),
                        None => metadata.description = Some(value),
                    }
                }
            }
            Keyword::Category => {
                if let Some(value) = self.expect_ident("after `category`") {
                    match &metadata.category {
                        Some(previous) => self.duplicate(keyword, value.span, previous.span),
                        None => metadata.category = Some(value),
                    }
                }
            }
            Keyword::Priority => {
                if let Some(value) = self.expect_number("after `priority`") {
                    match &metadata.priority {
                        Some(previous) => self.duplicate(keyword, value.span, previous.span),
                        None => metadata.priority = Some(value),
                    }
                }
            }
            Keyword::Revision => {
                if let Some(value) = self.expect_number("after `revision`") {
                    match &metadata.revision {
                        Some(previous) => self.duplicate(keyword, value.span, previous.span),
                        None => metadata.revision = Some(value),
                    }
                }
            }
            Keyword::Trigger => {
                let condition = self.expression();
                match &metadata.trigger {
                    Some(previous) => {
                        let span = condition.span();
                        let previous = previous.span();
                        self.duplicate(keyword, span, previous);
                    }
                    None => metadata.trigger = Some(condition),
                }
            }
            Keyword::Require => {
                let condition = self.expression();
                let message = match &self.peek().kind {
                    TokenKind::Str(_) => self.expect_string("after a `require` condition"),
                    _ => None,
                };
                let span = keyword_span.to(condition.span());
                metadata.requires.push(RequireClause {
                    condition,
                    message,
                    span,
                });
            }
            _ => unreachable!("not a metadata keyword"),
        }
    }

    fn recover_in_body(&mut self) {
        let mut depth = 0usize;
        while !self.at_end() {
            let token = self.peek();
            match token.kind {
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                _ => {}
            }
            if depth == 0 {
                if let Some(keyword) = token.keyword() {
                    if keyword.starts_step() && self.pos != 0 {
                        return;
                    }
                }
            }
            self.advance();
        }
    }

    fn step(&mut self) -> Option<Step> {
        let token = self.peek().clone();
        let keyword = token.keyword()?;
        let start = token.span;
        match keyword {
            Keyword::Check => {
                self.advance();
                let control = self.path("after `check`")?;
                let span = start.to(control.span);
                Some(Step::Check { control, span })
            }
            Keyword::Set => {
                self.advance();
                let control = self.path("after `set`")?;
                self.expect(TokenKind::Assign, "after the control name")?;
                let value = self.set_value()?;
                let value_span = match &value {
                    SetValue::Position(ident) => ident.span,
                    SetValue::Number(number) => number.span,
                };
                Some(Step::Set {
                    control,
                    value,
                    span: start.to(value_span),
                })
            }
            Keyword::Start | Keyword::Stop | Keyword::Open | Keyword::Close => {
                let verb_span = self.advance().span;
                let verb = match keyword {
                    Keyword::Start => Verb::Start,
                    Keyword::Stop => Verb::Stop,
                    Keyword::Open => Verb::Open,
                    _ => Verb::Close,
                };
                let control = self.path("after the verb")?;
                let span = start.to(control.span);
                Some(Step::Verb {
                    verb,
                    verb_span,
                    control,
                    span,
                })
            }
            Keyword::Notify => {
                self.advance();
                let message = self.expect_string("after `notify`")?;
                let span = start.to(message.span);
                Some(Step::Notify { message, span })
            }
            Keyword::Call => {
                self.advance();
                let target = self.expect_ident("after `call`")?;
                let span = start.to(target.span);
                Some(Step::Call { target, span })
            }
            Keyword::Wait => {
                self.advance();
                let condition = self.expression();
                let timeout = self.timeout_clause();
                let span = start.to(timeout
                    .as_ref()
                    .map(|t| t.span)
                    .unwrap_or_else(|| condition.span()));
                Some(Step::Wait {
                    condition,
                    timeout,
                    span,
                })
            }
            Keyword::If => Some(Step::If(self.if_step()?)),
            Keyword::Complete => {
                self.advance();
                let mut condition = None;
                let mut timeout = None;
                let mut end = start;
                if self.eat_keyword(Keyword::When).is_some() {
                    let expr = self.expression();
                    end = expr.span();
                    condition = Some(expr);
                    timeout = self.timeout_clause();
                    if let Some(t) = &timeout {
                        end = t.span;
                    }
                } else if self.peek().is_keyword(Keyword::Timeout) {
                    timeout = self.timeout_clause();
                    if let Some(t) = &timeout {
                        end = t.span;
                    }
                }
                Some(Step::Complete {
                    condition,
                    timeout,
                    span: start.to(end),
                })
            }
            Keyword::Fail => {
                self.advance();
                let message = match &self.peek().kind {
                    TokenKind::Str(_) => self.expect_string("after `fail`"),
                    _ => None,
                };
                let end = message.as_ref().map(|m| m.span).unwrap_or(start);
                Some(Step::Fail {
                    message,
                    span: start.to(end),
                })
            }
            other => {
                self.error(
                    codes::EXPECTED_STEP,
                    format!("`{}` cannot start a step", other.as_str()),
                    Label::bare(start),
                );
                None
            }
        }
    }

    fn if_step(&mut self) -> Option<IfStep> {
        let start = self.advance().span; // `if`
        let condition = self.expression();
        let then_block = self.block()?;
        let mut end = then_block.span;
        let mut else_branch = None;
        if self.eat_keyword(Keyword::Else).is_some() {
            if self.peek().is_keyword(Keyword::If) {
                let nested = self.if_step()?;
                end = nested.span;
                else_branch = Some(ElseBranch::If(Box::new(nested)));
            } else {
                let block = self.block()?;
                end = block.span;
                else_branch = Some(ElseBranch::Block(block));
            }
        }
        Some(IfStep {
            condition,
            then_block,
            else_branch,
            span: start.to(end),
        })
    }

    fn block(&mut self) -> Option<Block> {
        let open = self.expect(TokenKind::LBrace, "to open a block")?.span;
        let mut steps = Vec::new();
        while !self.at_end() && !self.peek().is(&TokenKind::RBrace) {
            let before = self.pos;
            match self.step() {
                Some(step) => steps.push(step),
                None => {
                    self.recover_in_body();
                    if self.pos == before {
                        self.advance();
                    }
                }
            }
        }
        let close = self
            .expect(TokenKind::RBrace, "to close a block")
            .map(|t| t.span)
            .unwrap_or(self.peek().span);
        Some(Block {
            steps,
            span: open.to(close),
        })
    }

    fn timeout_clause(&mut self) -> Option<Timeout> {
        let start = self.eat_keyword(Keyword::Timeout)?.span;
        let token = self.peek().clone();
        let millis = match token.kind {
            TokenKind::Duration(ms) => {
                self.advance();
                ms
            }
            _ => {
                let found = token.kind.describe();
                self.error(
                    codes::EXPECTED_TOKEN,
                    "expected a duration after `timeout`",
                    Label::new(token.span, format!("found {found}, try `10s`")),
                );
                return None;
            }
        };
        let mut end = token.span;
        let mut fail = false;
        if self.peek().is_keyword(Keyword::Else) {
            self.advance();
            if let Some(token) = self.eat_keyword(Keyword::Fail) {
                fail = true;
                end = token.span;
            } else {
                let token = self.peek().clone();
                let found = token.kind.describe();
                self.error(
                    codes::EXPECTED_TOKEN,
                    "expected `fail` after `timeout ... else`",
                    Label::new(token.span, format!("found {found}")),
                );
            }
        }
        Some(Timeout {
            millis,
            fail,
            span: start.to(end),
        })
    }

    fn set_value(&mut self) -> Option<SetValue> {
        let token = self.peek().clone();
        match token.kind {
            TokenKind::Ident => {
                self.advance();
                Some(SetValue::Position(Ident {
                    text: token.text.to_string(),
                    span: token.span,
                }))
            }
            TokenKind::Number(value) => {
                self.advance();
                Some(SetValue::Number(Spanned::new(value, token.span)))
            }
            TokenKind::Minus => {
                self.advance();
                let number = self.expect_number("after `-`")?;
                Some(SetValue::Number(Spanned::new(
                    -number.value,
                    token.span.to(number.span),
                )))
            }
            _ => {
                let found = token.kind.describe();
                self.error(
                    codes::EXPECTED_TOKEN,
                    "expected a control position or a number",
                    Label::new(token.span, format!("found {found}")),
                );
                None
            }
        }
    }

    fn path(&mut self, context: &str) -> Option<Path> {
        let first = self.expect_ident(context)?;
        let mut span = first.span;
        let mut text = first.text.clone();
        let mut segments = vec![first];
        while self.peek().is(&TokenKind::Dot) {
            self.advance();
            let token = self.peek().clone();
            let segment = match token.kind {
                TokenKind::Ident => {
                    self.advance();
                    Ident {
                        text: token.text.to_string(),
                        span: token.span,
                    }
                }
                TokenKind::Number(_) => {
                    self.advance();
                    Ident {
                        text: token.text.to_string(),
                        span: token.span,
                    }
                }
                _ => {
                    let found = token.kind.describe();
                    self.error(
                        codes::EXPECTED_TOKEN,
                        "expected a path segment after `.`",
                        Label::new(token.span, format!("found {found}")),
                    );
                    return None;
                }
            };
            text.push('.');
            text.push_str(&segment.text);
            span = span.to(segment.span);
            segments.push(segment);
        }
        Some(Path {
            text,
            segments,
            span,
        })
    }

    pub fn expression(&mut self) -> Expr {
        self.or_expression()
    }

    fn or_expression(&mut self) -> Expr {
        let mut lhs = self.and_expression();
        while self.peek().is(&TokenKind::OrOr) {
            let op_span = self.advance().span;
            let rhs = self.and_expression();
            let span = lhs.span().to(rhs.span());
            lhs = Expr::Binary {
                op: BinOp::Or,
                op_span,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        lhs
    }

    fn and_expression(&mut self) -> Expr {
        let mut lhs = self.comparison();
        while self.peek().is(&TokenKind::AndAnd) {
            let op_span = self.advance().span;
            let rhs = self.comparison();
            let span = lhs.span().to(rhs.span());
            lhs = Expr::Binary {
                op: BinOp::And,
                op_span,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        lhs
    }

    fn comparison(&mut self) -> Expr {
        let lhs = self.unary();
        let op = match self.peek().kind {
            TokenKind::Lt => BinOp::Lt,
            TokenKind::Le => BinOp::Le,
            TokenKind::Gt => BinOp::Gt,
            TokenKind::Ge => BinOp::Ge,
            TokenKind::EqEq => BinOp::Eq,
            TokenKind::BangEq => BinOp::Ne,
            _ => return lhs,
        };
        let op_span = self.advance().span;
        let rhs = self.unary();
        let span = lhs.span().to(rhs.span());
        let expr = Expr::Binary {
            op,
            op_span,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span,
        };
        if matches!(
            self.peek().kind,
            TokenKind::Lt
                | TokenKind::Le
                | TokenKind::Gt
                | TokenKind::Ge
                | TokenKind::EqEq
                | TokenKind::BangEq
        ) {
            let span = self.peek().span;
            self.error(
                codes::CHAINED_COMPARISON,
                "comparisons cannot be chained",
                Label::new(span, "unexpected second comparison"),
            );
            self.advance();
            let _ = self.unary();
        }
        expr
    }

    fn unary(&mut self) -> Expr {
        if self.peek().is(&TokenKind::Bang) {
            let start = self.advance().span;
            let operand = self.unary();
            let span = start.to(operand.span());
            return Expr::Not {
                operand: Box::new(operand),
                span,
            };
        }
        self.primary()
    }

    fn primary(&mut self) -> Expr {
        let token = self.peek().clone();
        match token.kind {
            TokenKind::LParen => {
                self.advance();
                let inner = self.expression();
                self.expect(TokenKind::RParen, "to close the group");
                inner
            }
            TokenKind::Number(value) => {
                self.advance();
                Expr::Number(value, token.span)
            }
            TokenKind::Minus => {
                self.advance();
                match self.expect_number("after `-`") {
                    Some(number) => Expr::Number(-number.value, token.span.to(number.span)),
                    None => Expr::Error(token.span),
                }
            }
            TokenKind::Duration(ms) => {
                self.advance();
                self.error(
                    codes::EXPECTED_EXPRESSION,
                    "a duration cannot be used in a condition",
                    Label::new(token.span, format!("`{ms}ms` is a duration")),
                );
                Expr::Error(token.span)
            }
            TokenKind::Ident => match Keyword::from_str(token.text) {
                Some(keyword) if keyword.starts_step() => {
                    self.error(
                        codes::EXPECTED_EXPRESSION,
                        "expected a condition",
                        Label::new(
                            token.span,
                            format!("found the keyword `{}`", keyword.as_str()),
                        ),
                    );
                    Expr::Error(token.span)
                }
                Some(Keyword::True) => {
                    self.advance();
                    Expr::Bool(true, token.span)
                }
                Some(Keyword::False) => {
                    self.advance();
                    Expr::Bool(false, token.span)
                }
                _ => match self.path("in the condition") {
                    Some(path) => Expr::Symbol(path),
                    None => Expr::Error(token.span),
                },
            },
            _ => {
                let found = token.kind.describe();
                self.error(
                    codes::EXPECTED_EXPRESSION,
                    "expected a condition",
                    Label::new(token.span, format!("found {found}")),
                );
                Expr::Error(token.span)
            }
        }
    }
}
