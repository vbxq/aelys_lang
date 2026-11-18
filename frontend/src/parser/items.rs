use super::Parser;
use super::types::substitute_closure_params;
use aelys_common::Result;
use aelys_common::error::CompileErrorKind;
use aelys_syntax::{
    AttrArg, Decorator, EnumVariantDecl, ForeignConv, ForeignDecl, Function, ImplDecl, ImportKind,
    Method, NeedsStmt, NeedsTarget, Receiver, Span, Stmt, StmtKind, StructFieldDecl, TokenKind,
    TraitDecl, TraitMethod, TraitRef, TypeAnnotation,
};

const KNOWN_ATTRIBUTES: &str = "`#[test]`, `#[inline]`, `#[inline(always)]`, `#[inline(never)]`, `#[cold]`, \
     `#[no_gc]`, `#[derive(..)]`, `#[repr(C)]`, `#[c_import(..)]`, `#[allow(..)]`";

impl Parser {
    pub(super) fn starts_item(&self) -> bool {
        match &self.peek().kind {
            TokenKind::Fn
            | TokenKind::Struct
            | TokenKind::Enum
            | TokenKind::Pub
            | TokenKind::Const
            | TokenKind::Static
            | TokenKind::Nogc
            | TokenKind::Extern
            | TokenKind::Impl
            | TokenKind::Trait
            | TokenKind::Type
            | TokenKind::Use
            | TokenKind::Mod
            | TokenKind::Pound => true,
            TokenKind::At => matches!(self.peek_at(1).kind, TokenKind::Identifier(_)),
            TokenKind::Unsafe => matches!(self.peek_at(1).kind, TokenKind::Extern | TokenKind::Fn),
            TokenKind::Let => self.block_depth == 0,
            TokenKind::Identifier(w) if w == "needs" => {
                matches!(self.peek_at(1).kind, TokenKind::Identifier(_) | TokenKind::String(_))
            }
            TokenKind::Identifier(w) if w == "union" => matches!(self.peek_at(1).kind, TokenKind::Identifier(_)),
            _ => false,
        }
    }

    pub(super) fn item(&mut self) -> Result<Vec<Stmt>> {
        let attrs = self.outer_attributes()?;
        let start = self.peek().span;
        let is_pub = self.visibility()?;
        let kind = self.peek().kind.clone();
        let next = self.peek_at(1).kind.clone();
        let mut stmts = match kind {
            TokenKind::Fn => vec![self.function_stmt(&attrs, is_pub, false)?],
            TokenKind::Nogc if next == TokenKind::Fn => {
                self.advance();
                vec![self.function_stmt(&attrs, is_pub, true)?]
            }
            TokenKind::Unsafe if next == TokenKind::Fn => {
                return self.fail_help(
                    137,
                    "`unsafe fn` is not supported",
                    "write a safe function and mark the dangerous calls `unsafe { }`",
                    start,
                );
            }
            TokenKind::Const if next == TokenKind::Fn => {
                return self.fail(137, "`const fn` is not supported", start);
            }
            TokenKind::Unsafe | TokenKind::Extern => self.extern_block(is_pub)?,
            TokenKind::Struct => vec![self.struct_declaration(is_pub)?],
            TokenKind::Enum => vec![self.enum_declaration(is_pub)?],
            TokenKind::Trait => vec![self.trait_declaration(is_pub)?],
            TokenKind::Impl => {
                if is_pub {
                    return Err(self.error(CompileErrorKind::MethodSyntax {
                        message: "an `impl` block takes no `pub`; mark each method `pub` instead".to_string(),
                    }));
                }
                vec![self.impl_declaration()?]
            }
            TokenKind::Const => vec![self.const_declaration(is_pub)?],
            TokenKind::Static => vec![self.static_declaration(is_pub)?],
            TokenKind::Type => vec![self.type_alias_declaration(is_pub)?],
            TokenKind::Use => self.use_declaration()?,
            TokenKind::Mod => vec![self.mod_declaration()?],
            TokenKind::Let if self.block_depth == 0 => {
                let mutable = matches!(next, TokenKind::Mut);
                let example = if mutable { "static mut g: i64 = 1;" } else { "static G: i64 = 1;" };
                return self.fail_help(
                    122,
                    &format!("a module-level variable is declared with `static`: `{example}`"),
                    "`let` declares a local inside a function body",
                    start,
                );
            }
            TokenKind::Identifier(w) if w == "needs" => return Err(self.old_needs()),
            TokenKind::Identifier(w) if w == "union" => {
                return self.fail(137, "`union` is not supported yet", start);
            }
            _ => {
                if is_pub || !attrs.is_empty() {
                    return self.unexpected("an item after `pub` or an attribute");
                }
                return self.unexpected("an item (`fn`, `struct`, `enum`, `trait`, `impl`, `const`, `static`, `use`, `mod`)");
            }
        };
        if !matches!(kind, TokenKind::Fn | TokenKind::Nogc) {
            for stmt in &mut stmts {
                stmt.attrs = attrs.clone();
            }
        }
        Ok(stmts)
    }

    fn old_needs(&mut self) -> aelys_common::error::AelysError {
        let start = self.peek().span;
        self.advance();
        let mut words = Vec::new();
        let mut from = None;
        loop {
            match self.peek().kind.clone() {
                TokenKind::Identifier(w) if w == "from" => from = Some(words.len()),
                TokenKind::Identifier(w) => words.push(w),
                TokenKind::Dot | TokenKind::Comma => {}
                _ => break,
            }
            self.advance();
        }
        let symbols: Vec<String> = match from {
            Some(at) => words.drain(..at).collect(),
            None => Vec::new(),
        };
        let path = words.join("::");
        let suggestion = match symbols.len() {
            0 => format!("use {path};"),
            1 => format!("use {path}::{};", symbols[0]),
            _ => format!("use {path}::{{{}}};", symbols.join(", ")),
        };
        self.rule(
            115,
            &format!("`needs` is not a declaration; write `{suggestion}`"),
            start,
        )
        .into()
    }

    pub(super) fn outer_attributes(&mut self) -> Result<Vec<Decorator>> {
        let mut attrs = Vec::new();
        loop {
            if self.check(&TokenKind::At) {
                let span = self.peek().span;
                let name = match &self.peek_at(1).kind {
                    TokenKind::Identifier(n) => n.clone(),
                    _ => return self.unexpected("an item"),
                };
                let spelled = match name.as_str() {
                    "inline_always" => "#[inline(always)]".to_string(),
                    "inline_never" => "#[inline(never)]".to_string(),
                    other => format!("#[{other}]"),
                };
                return self.fail(116, &format!("`@{name}` is not an attribute; write `{spelled}`"), span);
            }
            if !self.check(&TokenKind::Pound) {
                return Ok(attrs);
            }
            let start = self.peek().span;
            self.advance();
            if self.check(&TokenKind::Bang) {
                return self.fail(130, "inner attributes `#![..]` are not supported", start.merge(self.peek().span));
            }
            self.consume(&TokenKind::LBracket, "`[` after `#`")?;
            let meta = self.attr_meta()?;
            self.consume(&TokenKind::RBracket, "`]`")?;
            let span = start.merge(self.previous().span);
            let (name, args) = match meta {
                AttrArg::Word(n) => (n, Vec::new()),
                AttrArg::Call(n, args) => (n, args),
                AttrArg::Assign(n, v) => (n, vec![*v]),
                _ => return self.fail(130, "an attribute starts with a name", span),
            };
            let known = matches!(
                name.as_str(),
                "test" | "inline" | "cold" | "no_gc" | "derive" | "repr" | "c_import" | "allow" | "must_use" | "fmt::skip"
            );
            if !known {
                let written = &self.source.content[span.start..span.end];
                return self.fail_help(
                    130,
                    &format!("unknown attribute `{written}`"),
                    &format!("the known attributes are {KNOWN_ATTRIBUTES}"),
                    span,
                );
            }
            attrs.push(Decorator { name, args, span });
        }
    }

    fn attr_meta(&mut self) -> Result<AttrArg> {
        match self.peek().kind.clone() {
            TokenKind::String(s) => {
                self.advance();
                Ok(AttrArg::Str(s))
            }
            TokenKind::Int(n) => {
                self.advance();
                Ok(AttrArg::Int(n))
            }
            TokenKind::LBracket => {
                self.advance();
                let mut items = Vec::new();
                while !self.check(&TokenKind::RBracket) {
                    items.push(self.attr_meta()?);
                    if !self.match_token(&TokenKind::Comma) {
                        break;
                    }
                }
                self.consume(&TokenKind::RBracket, "`]`")?;
                Ok(AttrArg::List(items))
            }
            _ => {
                let mut name = self.consume_member_name("an attribute name")?;
                while self.match_token(&TokenKind::ColonColon) {
                    name.push_str("::");
                    name.push_str(&self.consume_member_name("an attribute name")?);
                }
                if self.match_token(&TokenKind::Eq) {
                    return Ok(AttrArg::Assign(name, Box::new(self.attr_meta()?)));
                }
                if self.match_token(&TokenKind::LParen) {
                    let mut args = Vec::new();
                    while !self.check(&TokenKind::RParen) {
                        args.push(self.attr_meta()?);
                        if !self.match_token(&TokenKind::Comma) {
                            break;
                        }
                    }
                    self.consume(&TokenKind::RParen, "`)`")?;
                    return Ok(AttrArg::Call(name, args));
                }
                Ok(AttrArg::Word(name))
            }
        }
    }

    pub(super) fn visibility(&mut self) -> Result<bool> {
        if !self.match_token(&TokenKind::Pub) {
            return Ok(false);
        }
        if self.check(&TokenKind::LParen) {
            match self.peek_at(1).kind {
                TokenKind::Crate | TokenKind::Super => {}
                TokenKind::Identifier(ref w) if w == "self" => {}
                _ => return self.fail(137, "`pub(in path)` is not supported", self.peek().span),
            }
            self.advance();
            self.advance();
            self.consume(&TokenKind::RParen, "`)`")?;
        }
        Ok(true)
    }

    fn function_decorators(&self, attrs: &[Decorator]) -> Vec<Decorator> {
        attrs
            .iter()
            .map(|a| {
                let name = match (a.name.as_str(), a.args.first()) {
                    ("inline", Some(AttrArg::Word(w))) if w == "always" => "inline_always".to_string(),
                    ("inline", Some(AttrArg::Word(w))) if w == "never" => "inline_never".to_string(),
                    (n, _) => n.to_string(),
                };
                Decorator {
                    name,
                    args: a.args.clone(),
                    span: a.span,
                }
            })
            .collect()
    }

    fn function_stmt(&mut self, attrs: &[Decorator], is_pub: bool, is_nogc: bool) -> Result<Stmt> {
        let decorators = self.function_decorators(attrs);
        let (function, _, _) = self.function_item(decorators, is_pub, is_nogc, false, false)?;
        let span = function.span;
        Ok(Stmt::new(StmtKind::Function(function), span))
    }

    pub(super) fn self_param_message() -> String {
        "a `self` parameter is only valid in a method of an `impl` block, as its first parameter".to_string()
    }

    pub(super) fn receiver_ahead(&self) -> Option<(Receiver, usize)> {
        let at = |i: usize| &self.peek_at(i).kind;
        let is_self = |k: &TokenKind| matches!(k, TokenKind::Identifier(n) if n == "self");
        if is_self(at(0)) {
            return Some((Receiver::Value, 1));
        }
        if *at(0) == TokenKind::Ampersand {
            if is_self(at(1)) {
                return Some((Receiver::Ref, 2));
            }
            if *at(1) == TokenKind::Mut && is_self(at(2)) {
                return Some((Receiver::RefMut, 3));
            }
        }
        if *at(0) == TokenKind::Mut && is_self(at(1)) {
            return Some((Receiver::Value, 2));
        }
        None
    }

    pub(super) fn function_item(
        &mut self,
        decorators: Vec<Decorator>,
        is_pub: bool,
        is_nogc: bool,
        allow_receiver: bool,
        body_optional: bool,
    ) -> Result<(Function, Receiver, bool)> {
        let start = self.peek().span;
        self.advance();
        let name = self.consume_value_name("a function name")?;
        let mut generics = self.generic_params()?;
        self.consume(&TokenKind::LParen, "`(`")?;

        let mut receiver = Receiver::None;
        self.mut_self = false;
        if let Some((kind, len)) = self.receiver_ahead() {
            if !allow_receiver {
                return Err(self.error(CompileErrorKind::MethodSyntax {
                    message: Self::self_param_message(),
                }));
            }
            self.mut_self = self.peek().kind == TokenKind::Mut;
            for _ in 0..len {
                self.advance();
            }
            if self.check(&TokenKind::Colon) {
                return Err(self.error(CompileErrorKind::MethodSyntax {
                    message: "a `self` receiver takes no type annotation; write `self`, `&self` or `&mut self`"
                        .to_string(),
                }));
            }
            receiver = kind;
            if !self.check(&TokenKind::RParen) {
                self.consume(&TokenKind::Comma, "`,` or `)`")?;
            }
        }
        let mut params = Vec::new();
        while !self.check(&TokenKind::RParen) {
            params.push(self.parse_parameter()?);
            if !self.match_token(&TokenKind::Comma) {
                break;
            }
        }
        self.consume(&TokenKind::RParen, "`)`")?;
        let mut return_type = if self.match_token(&TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };
        self.parse_where_clause(&mut generics)?;
        let mut type_params = generics.params;
        let mut bounds = generics.bounds;
        if !generics.closures.is_empty() {
            for p in &mut params {
                if let Some(ann) = &mut p.type_annotation {
                    substitute_closure_params(ann, &generics.closures);
                }
            }
            if let Some(ann) = &mut return_type {
                substitute_closure_params(ann, &generics.closures);
            }
            let keep: Vec<bool> = type_params
                .iter()
                .map(|p| !generics.closures.iter().any(|(n, _)| n == p))
                .collect();
            let mut i = 0;
            type_params.retain(|_| {
                i += 1;
                keep[i - 1]
            });
            let mut i = 0;
            bounds.retain(|_| {
                i += 1;
                keep.get(i - 1).copied().unwrap_or(true)
            });
        }

        let mut bodiless = false;
        let body = if body_optional && !self.check(&TokenKind::LBrace) {
            bodiless = true;
            self.expect_semi("this method declaration")?;
            Vec::new()
        } else {
            if !self.check(&TokenKind::LBrace) {
                return self.unexpected("`{` to open the function body");
            }
            self.function_body(return_type.is_some())?
        };
        let function = Function {
            name,
            type_params,
            bounds,
            params,
            return_type,
            body,
            decorators,
            is_pub,
            is_nogc,
            foreign: None,
            span: start.merge(self.previous().span),
        };
        Ok((function, receiver, bodiless))
    }

    fn extern_block(&mut self, is_pub: bool) -> Result<Vec<Stmt>> {
        let start = self.peek().span;
        let had_unsafe = self.match_token(&TokenKind::Unsafe);
        self.consume(&TokenKind::Extern, "`extern`")?;
        if matches!(self.peek().kind, TokenKind::Fn | TokenKind::Nogc) {
            let end = self.peek().span;
            if had_unsafe {
                return self.fail_help(
                    139,
                    "a C function is declared in an `extern \"C\"` block: `extern \"C\" { fn labs(x: i64) -> i64; }`",
                    "`unsafe extern fn` is the old form",
                    start.merge(end),
                );
            }
            return self.fail(139, "a C function is declared in an `extern \"C\"` block: `extern \"C\" { fn labs(x: i64) -> i64; }`", start.merge(end));
        }
        match self.peek().kind.clone() {
            TokenKind::String(abi) if abi == "C" => {
                self.advance();
            }
            TokenKind::String(abi) => {
                return self.fail(139, &format!("the only foreign ABI is \"C\", not \"{abi}\""), self.peek().span);
            }
            _ => return self.unexpected("`\"C\"` after `extern`"),
        }
        if self.check(&TokenKind::Fn) {
            return self.fail(137, "an `extern \"C\" fn` with a body is not supported yet", start);
        }
        if is_pub {
            return self.fail(614, "an `extern` block cannot be `pub`; mark each declaration", start);
        }
        if self.block_depth > 0 {
            return Err(self.malformed_foreign("an external declaration belongs at the top level"));
        }
        self.consume(&TokenKind::LBrace, "`{`")?;
        let mut stmts = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            let decl_attrs = self.outer_attributes()?;
            if !decl_attrs.is_empty() {
                return Err(self.malformed_foreign("an external declaration carries no attribute"));
            }
            let decl_start = self.peek().span;
            if self.check(&TokenKind::Pub) {
                return Err(self.malformed_foreign("an external declaration cannot be `pub`"));
            }
            if self.check_ident("safe") {
                self.advance();
            }
            let is_nogc = self.match_token(&TokenKind::Nogc);
            if self.check(&TokenKind::Static) {
                return self.fail(137, "an `extern` static is not supported yet", self.peek().span);
            }
            self.consume(&TokenKind::Fn, "`fn` in an `extern` block")?;
            let name = self.consume_value_name("a function name")?;
            if self.check(&TokenKind::Lt) {
                return Err(self.malformed_foreign("an external declaration takes no type parameter"));
            }
            self.consume(&TokenKind::LParen, "`(`")?;
            let mut params = Vec::new();
            while !self.check(&TokenKind::RParen) {
                if self.check(&TokenKind::DotDot) || self.check(&TokenKind::Dot) {
                    return self.fail(137, "variadic C functions are not supported yet", self.peek().span);
                }
                params.push(self.parse_parameter()?);
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
            self.consume(&TokenKind::RParen, "`)`")?;
            let return_type = if self.match_token(&TokenKind::Arrow) {
                Some(self.parse_type()?)
            } else {
                None
            };
            if self.check(&TokenKind::LBrace) {
                return Err(self.malformed_foreign("an external declaration has no body"));
            }
            self.expect_semi("this external declaration")?;
            let span = decl_start.merge(self.previous().span);
            let function = Function {
                name: name.clone(),
                type_params: Vec::new(),
                bounds: Vec::new(),
                params,
                return_type,
                body: Vec::new(),
                decorators: Vec::new(),
                is_pub: false,
                is_nogc,
                foreign: Some(ForeignDecl {
                    symbol: name,
                    calling_conv: ForeignConv::C,
                    is_unsafe: true,
                    span,
                }),
                span,
            };
            stmts.push(Stmt::new(StmtKind::Function(function), span));
        }
        self.consume(&TokenKind::RBrace, "`}`")?;
        Ok(stmts)
    }

    fn malformed_foreign(&self, reason: &str) -> aelys_common::error::AelysError {
        self.error(CompileErrorKind::MalformedForeignDecl {
            reason: reason.to_string(),
        })
    }

    fn capitalized_name(&mut self, what: &str) -> Result<String> {
        let name = self.consume_identifier(&format!("a {what} name"))?;
        if name.chars().next().is_none_or(|c| !c.is_uppercase()) {
            return Err(self.error(CompileErrorKind::UnexpectedToken {
                expected: format!("capitalized {what} name"),
                found: name,
            }));
        }
        Ok(name)
    }

    fn plain_type_params(&mut self) -> Result<Vec<String>> {
        let generics = self.generic_params()?;
        Ok(generics.params)
    }

    fn struct_declaration(&mut self, is_pub: bool) -> Result<Stmt> {
        let start = self.peek().span;
        self.advance();
        let name = self.capitalized_name("struct")?;
        let type_params = self.plain_type_params()?;
        let mut fields = Vec::new();
        if self.match_token(&TokenKind::LParen) {
            let mut index = 0;
            while !self.check(&TokenKind::RParen) {
                let field_span = self.peek().span;
                let field_pub = self.visibility()?;
                let type_annotation = self.parse_type()?;
                fields.push(StructFieldDecl {
                    name: index.to_string(),
                    type_annotation,
                    is_pub: field_pub,
                    span: field_span.merge(self.previous().span),
                });
                index += 1;
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
            self.consume(&TokenKind::RParen, "`)`")?;
            self.expect_semi("a tuple struct declaration")?;
        } else if !self.match_token(&TokenKind::Semicolon) {
            self.consume(&TokenKind::LBrace, "`{`, `(` or `;` after the struct name")?;
            while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
                self.outer_attributes()?;
                let field_span = self.peek().span;
                let field_pub = self.visibility()?;
                let field_name = self.consume_member_name("a field name")?;
                self.consume(&TokenKind::Colon, "`:`")?;
                let type_annotation = self.parse_type()?;
                fields.push(StructFieldDecl {
                    name: field_name,
                    type_annotation,
                    is_pub: field_pub,
                    span: field_span.merge(self.previous().span),
                });
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
            self.consume(&TokenKind::RBrace, "`}`")?;
        }
        Ok(Stmt::new(
            StmtKind::StructDecl {
                name,
                type_params,
                fields,
                is_pub,
            },
            start.merge(self.previous().span),
        ))
    }

    fn enum_declaration(&mut self, is_pub: bool) -> Result<Stmt> {
        let start = self.peek().span;
        self.advance();
        let name = self.capitalized_name("enum")?;
        let type_params = self.plain_type_params()?;
        self.consume(&TokenKind::LBrace, "`{`")?;
        let mut variants = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            self.outer_attributes()?;
            let variant_span = self.peek().span;
            let variant_name = self.capitalized_name("variant")?;
            let mut fields = Vec::new();
            if self.match_token(&TokenKind::LParen) {
                while !self.check(&TokenKind::RParen) {
                    fields.push(self.parse_type()?);
                    if !self.match_token(&TokenKind::Comma) {
                        break;
                    }
                }
                self.consume(&TokenKind::RParen, "`)`")?;
            } else if self.check(&TokenKind::LBrace) {
                return self.fail(
                    127,
                    "a variant with named fields is not supported; use a tuple variant",
                    variant_span.merge(self.peek().span),
                );
            } else if self.check(&TokenKind::Eq) {
                return self.fail(
                    127,
                    "an explicit discriminant is not supported",
                    variant_span.merge(self.peek().span),
                );
            }
            variants.push(EnumVariantDecl {
                name: variant_name,
                fields,
                span: variant_span.merge(self.previous().span),
            });
            if !self.match_token(&TokenKind::Comma) {
                break;
            }
        }
        self.consume(&TokenKind::RBrace, "`}`")?;
        Ok(Stmt::new(
            StmtKind::EnumDecl {
                name,
                type_params,
                variants,
                is_pub,
            },
            start.merge(self.previous().span),
        ))
    }

    fn trait_declaration(&mut self, is_pub: bool) -> Result<Stmt> {
        let start = self.peek().span;
        self.advance();
        if self.block_depth > 0 {
            return self.fail(757, "a `trait` belongs at the top level of a module", start);
        }
        let name = self.consume_identifier("a trait name")?;
        let type_params = self.plain_type_params()?;
        if self.check(&TokenKind::Colon) {
            return self.fail(137, "supertraits are not supported yet", self.peek().span);
        }
        self.consume(&TokenKind::LBrace, "`{`")?;
        let mut methods = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            let attrs = self.outer_attributes()?;
            let decorators = self.function_decorators(&attrs);
            let method_pub = self.visibility()?;
            let is_nogc = self.match_token(&TokenKind::Nogc);
            if !self.check(&TokenKind::Fn) {
                return self.fail(
                    757,
                    &format!("only `fn` items are allowed in a `trait`, found {}", self.peek().kind),
                    self.peek().span,
                );
            }
            let (func, receiver, bodiless) = self.function_item(decorators, method_pub, is_nogc, true, true)?;
            methods.push(TraitMethod {
                func,
                receiver,
                has_default: !bodiless,
                mut_self: self.mut_self,
            });
        }
        self.consume(&TokenKind::RBrace, "`}`")?;
        let span = start.merge(self.previous().span);
        Ok(Stmt::new(
            StmtKind::Trait(TraitDecl {
                name,
                type_params,
                is_pub,
                methods,
                span,
            }),
            span,
        ))
    }

    fn impl_declaration(&mut self) -> Result<Stmt> {
        let start = self.peek().span;
        self.advance();
        if self.block_depth > 0 {
            return Err(self.error(CompileErrorKind::MethodSyntax {
                message: "an `impl` block belongs at the top level of a module".to_string(),
            }));
        }
        let mut generics = self.generic_params()?;
        let nogc_ref = if self.check(&TokenKind::Nogc) && self.peek_at(1).kind == TokenKind::For {
            let span = self.peek().span;
            self.advance();
            self.advance();
            Some(TraitRef::new("nogc".to_string(), span))
        } else {
            None
        };
        let first = self.impl_head()?;
        let (trait_ref, self_ty) = if nogc_ref.is_some() {
            (nogc_ref, first)
        } else if self.match_token(&TokenKind::For) {
            if first.reference.is_some() || first.is_slice || first.is_function_type() {
                return self.fail(757, "the name after `impl` and before `for` is a trait name", first.span);
            }
            let mut args = first.type_params.clone();
            if args.is_empty()
                && let Some(p) = &first.type_param
            {
                args.push((**p).clone());
            }
            let name = match first.name.as_str() {
                "PartialEq" | "Eq" => "eq".to_string(),
                "Ord" | "PartialOrd" => "ord".to_string(),
                other => other.to_string(),
            };
            (
                Some(TraitRef {
                    name,
                    span: first.span,
                    args,
                }),
                self.parse_type()?,
            )
        } else {
            (None, first)
        };
        self.parse_where_clause(&mut generics)?;
        self.consume(&TokenKind::LBrace, "`{`")?;
        let mut methods = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            let attrs = self.outer_attributes()?;
            let decorators = self.function_decorators(&attrs);
            let method_pub = self.visibility()?;
            let is_nogc = self.match_token(&TokenKind::Nogc);
            if !self.check(&TokenKind::Fn) {
                return Err(self.error(CompileErrorKind::MethodSyntax {
                    message: format!("only `fn` items are allowed in an `impl` block, found {}", self.peek().kind),
                }));
            }
            let display_fmt = trait_ref.as_ref().is_some_and(|t| t.name == "Display")
                && matches!(&self.peek_at(1).kind, TokenKind::Identifier(n) if n == "fmt");
            if display_fmt {
                self.fmt_writes = Some(Vec::new());
            }
            let parsed = self.function_item(decorators, method_pub, is_nogc, true, false);
            let writes = self.fmt_writes.take();
            let (mut func, receiver, _) = parsed?;
            if let Some(writes) = writes {
                self.display_fmt_to_string(&mut func, writes)?;
            }
            methods.push(Method {
                func,
                receiver,
                mut_self: self.mut_self,
            });
        }
        self.consume(&TokenKind::RBrace, "`}`")?;
        let span = start.merge(self.previous().span);
        Ok(Stmt::new(
            StmtKind::Impl(ImplDecl {
                type_params: generics.params,
                bounds: generics.bounds,
                trait_ref,
                self_ty,
                methods,
                span,
            }),
            span,
        ))
    }

    fn display_fmt_to_string(&self, func: &mut Function, writes: Vec<(String, Span)>) -> Result<()> {
        let formatter = match func.params.as_slice() {
            [p] if p.type_annotation.as_ref().is_some_and(|t| t.name == "fmt.Formatter") => p.name.clone(),
            _ => return self.fail(396, "`Display::fmt` takes `&self` and `f: &mut fmt::Formatter`", func.span),
        };
        if func.return_type.as_ref().is_none_or(|t| t.name != "string") {
            return self.fail(396, "`Display::fmt` returns `fmt::Result`", func.span);
        }
        if let Some((_, span)) = writes.iter().find(|(n, _)| *n != formatter) {
            return self.fail(396, &format!("`write!` writes only to `{formatter}`, the formatter of `Display::fmt`"), *span);
        }
        func.name = "to_string".to_string();
        func.params.clear();
        Ok(())
    }

    fn impl_head(&mut self) -> Result<TypeAnnotation> {
        if self.check(&TokenKind::LBracket) {
            let save = self.current;
            let start = self.peek().span;
            self.advance();
            let element = self.parse_type()?;
            if self.match_token(&TokenKind::RBracket) {
                let span = start.merge(self.previous().span);
                return Ok(TypeAnnotation::slice_referent(element, span));
            }
            self.current = save;
        }
        self.parse_type()
    }

    fn const_declaration(&mut self, is_pub: bool) -> Result<Stmt> {
        let start = self.peek().span;
        self.advance();
        if self.block_depth > 0 {
            return self.fail(101, "a const is declared at module level", start);
        }
        let name_span = self.peek().span;
        let name = self.consume_identifier("a const name")?;
        if !name.starts_with(char::is_uppercase) {
            return self.fail(
                101,
                &format!("a const name starts with an uppercase letter; write `{}`", name.to_uppercase()),
                name_span,
            );
        }
        if !self.match_token(&TokenKind::Colon) {
            return self.fail(101, &format!("a const needs a type: `const {name}: i64 = ...;`"), self.peek().span);
        }
        let type_annotation = self.parse_type()?;
        self.consume(&TokenKind::Eq, "`=`")?;
        let initializer = self.expression()?;
        self.expect_semi("this const declaration")?;
        Ok(Stmt::new(
            StmtKind::Const {
                name,
                type_annotation,
                initializer,
                is_pub,
            },
            start.merge(self.previous().span),
        ))
    }

    fn static_declaration(&mut self, is_pub: bool) -> Result<Stmt> {
        let start = self.peek().span;
        self.advance();
        if self.block_depth > 0 {
            return self.fail(122, "a `static` is declared at module level; use `let` inside a function", start);
        }
        let mutable = self.match_token(&TokenKind::Mut);
        let name = self.consume_value_name("a static name")?;
        if !self.match_token(&TokenKind::Colon) {
            return self.fail(101, &format!("a static needs a type: `static {name}: i64 = ...;`"), self.peek().span);
        }
        let type_annotation = Some(self.parse_type()?);
        self.consume(&TokenKind::Eq, "`=`")?;
        let initializer = self.expression()?;
        self.expect_semi("this static declaration")?;
        Ok(Stmt::new(
            StmtKind::Let {
                name,
                mutable,
                type_annotation,
                initializer,
                is_pub,
            },
            start.merge(self.previous().span),
        ))
    }

    fn type_alias_declaration(&mut self, is_pub: bool) -> Result<Stmt> {
        let start = self.peek().span;
        self.advance();
        if self.block_depth > 0 {
            return self.fail(101, "a type alias is declared at module level", start);
        }
        let name_span = self.peek().span;
        let name = self.consume_identifier("a type alias name")?;
        if !name.starts_with(char::is_uppercase) {
            return self.fail(
                101,
                &format!("a type alias name is capitalized; `{name}` starts with a lowercase letter"),
                name_span,
            );
        }
        if matches!(name.as_str(), "Option" | "Result" | "Vec" | "Rc") {
            return self.fail(
                204,
                &format!("the name `{name}` is defined twice, once by the prelude and once as a type alias"),
                name_span,
            );
        }
        let type_params = self.plain_type_params()?;
        self.consume(&TokenKind::Eq, "`=`")?;
        let target = self.parse_type()?;
        self.expect_semi("this type alias")?;
        Ok(Stmt::new(
            StmtKind::TypeAlias {
                name,
                type_params,
                target,
                is_pub,
            },
            start.merge(self.previous().span),
        ))
    }

    fn mod_declaration(&mut self) -> Result<Stmt> {
        let start = self.peek().span;
        self.advance();
        if self.block_depth > 0 {
            return self.fail(137, "a `mod` declaration belongs at module level", start);
        }
        let name = self.consume_identifier("a module name")?;
        if self.check(&TokenKind::LBrace) {
            return self.fail(137, "an inline module `mod name { .. }` is not supported yet; put it in its own file", start);
        }
        self.expect_semi("this `mod` declaration")?;
        let span = start.merge(self.previous().span);
        Ok(needs_stmt(vec![name], ImportKind::Module { alias: None }, span))
    }

    fn use_declaration(&mut self) -> Result<Vec<Stmt>> {
        let start = self.peek().span;
        self.advance();
        if self.block_depth > 0 {
            return self.fail(137, "a `use` declaration belongs at module level", start);
        }
        let mut out = Vec::new();
        self.use_tree(Vec::new(), &mut out)?;
        self.expect_semi("this `use` declaration")?;
        let span = start.merge(self.previous().span);
        Ok(out
            .into_iter()
            .filter_map(|(path, kind)| intrinsic_import(path, kind))
            .map(|(path, kind)| needs_stmt(path, kind, span))
            .collect())
    }

    fn use_tree(&mut self, mut prefix: Vec<String>, out: &mut Vec<(Vec<String>, ImportKind)>) -> Result<()> {
        loop {
            match self.peek().kind.clone() {
                TokenKind::Crate if prefix.is_empty() => {
                    self.advance();
                }
                TokenKind::Identifier(w) if w == "self" && prefix.is_empty() => {
                    self.advance();
                }
                TokenKind::Super => {
                    return self.fail(137, "`super::` paths are not supported yet", self.peek().span);
                }
                TokenKind::Star => {
                    self.advance();
                    out.push((prefix, ImportKind::Wildcard));
                    return Ok(());
                }
                TokenKind::LBrace => {
                    self.advance();
                    let mut symbols = Vec::new();
                    while !self.check(&TokenKind::RBrace) {
                        let simple = matches!(self.peek().kind, TokenKind::Identifier(_))
                            && matches!(self.peek_at(1).kind, TokenKind::Comma | TokenKind::RBrace);
                        if simple {
                            let name = self.consume_identifier("a name")?;
                            if name == "self" {
                                out.push((prefix.clone(), ImportKind::Module { alias: None }));
                            } else {
                                symbols.push(name);
                            }
                        } else {
                            self.use_tree(prefix.clone(), out)?;
                        }
                        if !self.match_token(&TokenKind::Comma) {
                            break;
                        }
                    }
                    self.consume(&TokenKind::RBrace, "`}`")?;
                    if !symbols.is_empty() {
                        out.push((prefix, ImportKind::Symbols(symbols)));
                    }
                    return Ok(());
                }
                _ => {
                    let segment = self.consume_identifier("a module path segment")?;
                    if self.match_token(&TokenKind::ColonColon) {
                        prefix.push(segment);
                        continue;
                    }
                    if self.match_token(&TokenKind::As) {
                        let alias = self.consume_value_name("an alias")?;
                        prefix.push(segment);
                        out.push((prefix, ImportKind::Module { alias: Some(alias) }));
                        return Ok(());
                    }
                    if prefix.is_empty() || segment.starts_with(char::is_lowercase) {
                        prefix.push(segment);
                        out.push((prefix, ImportKind::Module { alias: None }));
                    } else {
                        out.push((prefix, ImportKind::Symbols(vec![segment])));
                    }
                    return Ok(());
                }
            }
            if !self.match_token(&TokenKind::ColonColon) && !prefix.is_empty() {
                return self.unexpected("`::`");
            }
        }
    }
}

fn intrinsic_import(path: Vec<String>, kind: ImportKind) -> Option<(Vec<String>, ImportKind)> {
    let intrinsic: &[&str] = match path.as_slice() {
        [a, b] if a == "std" && b == "fmt" => &["Display", "Formatter", "Result"],
        [a, b] if a == "std" && b == "process" => &["ExitCode"],
        _ => return Some((path, kind)),
    };
    match kind {
        ImportKind::Module { alias: None } if path[1] == "fmt" => None,
        ImportKind::Symbols(names) => {
            let rest: Vec<String> = names.into_iter().filter(|n| !intrinsic.contains(&n.as_str())).collect();
            (!rest.is_empty()).then(|| (path, ImportKind::Symbols(rest)))
        }
        kind => Some((path, kind)),
    }
}

fn needs_stmt(path: Vec<String>, kind: ImportKind, span: Span) -> Stmt {
    Stmt::new(
        StmtKind::Needs(NeedsStmt {
            target: NeedsTarget::Module { path, kind },
            span,
        }),
        span,
    )
}
