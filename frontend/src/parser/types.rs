use super::Parser;
use aelys_common::Result;
use aelys_common::error::CompileErrorKind;
use aelys_syntax::{Parameter, RefKind, Span, TokenKind, TraitRef, TypeAnnotation, TypeBounds};

const PRIMITIVES: [&str; 15] = [
    "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "bool", "char", "string",
    "usize", "isize",
];

pub(super) fn is_primitive(name: &str) -> bool {
    PRIMITIVES.contains(&name) || name == "String"
}

pub(super) fn canonical_type_name(name: &str) -> &str {
    match name {
        "String" | "str" => "string",
        "usize" => "u64",
        "isize" => "i64",
        "ExitCode" => "i64",
        other => other,
    }
}

pub(super) struct Generics {
    pub params: Vec<String>,
    pub bounds: Vec<TypeBounds>,
    pub closures: Vec<(String, TypeAnnotation)>,
}

impl Parser {
    pub(super) fn parse_type(&mut self) -> Result<TypeAnnotation> {
        self.enter_recursion()?;
        let result = self.parse_type_inner();
        self.exit_recursion();
        result
    }

    fn parse_type_inner(&mut self) -> Result<TypeAnnotation> {
        let start = self.peek().span;
        match self.peek().kind.clone() {
            TokenKind::AndAnd => self.fail(137, "a reference to a reference is not supported", start),
            TokenKind::Ampersand => {
                self.advance();
                if let TokenKind::Lifetime(name) = self.peek().kind.clone() {
                    return self.fail_help(
                        123,
                        &format!("Aelys has no lifetime annotations; remove `'{name}`"),
                        "values are managed, so a reference needs no lifetime",
                        self.peek().span,
                    );
                }
                let mutable = self.match_token(&TokenKind::Mut);
                self.reference_type(start, mutable)
            }
            TokenKind::LBracket => {
                self.advance();
                let inner = self.parse_type()?;
                self.consume(&TokenKind::Semicolon, "`;` in an array type `[T; N]`")?;
                let size = self.peek().kind.clone();
                match size {
                    TokenKind::Int(n) if n >= 0 => {
                        self.advance();
                        self.consume(&TokenKind::RBracket, "`]`")?;
                        Ok(TypeAnnotation::array_sized(inner, n as u64, start.merge(self.previous().span)))
                    }
                    TokenKind::Identifier(_) => {
                        let name = self.type_path_name()?;
                        self.consume(&TokenKind::RBracket, "`]`")?;
                        let mut ann = TypeAnnotation::array_sized(inner, 0, start.merge(self.previous().span));
                        ann.array_size = None;
                        ann.array_size_name = Some(name);
                        Ok(ann)
                    }
                    _ => self.unexpected("an array size, an integer literal or a const name"),
                }
            }
            TokenKind::LParen => {
                self.advance();
                if self.match_token(&TokenKind::RParen) {
                    return Err(self.unit_type_refusal(start));
                }
                let first = self.parse_type()?;
                if !self.match_token(&TokenKind::Comma) {
                    self.consume(&TokenKind::RParen, "`)`")?;
                    return Ok(first);
                }
                if self.check(&TokenKind::RParen) {
                    return self.fail(
                        902,
                        "a one-element tuple is not supported; write the type itself",
                        start.merge(self.peek().span),
                    );
                }
                let mut elems = vec![first];
                loop {
                    elems.push(self.parse_type()?);
                    if !self.match_token(&TokenKind::Comma) || self.check(&TokenKind::RParen) {
                        break;
                    }
                }
                self.consume(&TokenKind::RParen, "`)`")?;
                Ok(TypeAnnotation::tuple(elems, start.merge(self.previous().span)))
            }
            TokenKind::Nogc => {
                self.advance();
                if !self.match_token(&TokenKind::Fn) {
                    return self.unexpected("`fn` after `nogc` in a type");
                }
                self.function_type(start, true)
            }
            TokenKind::Fn => {
                self.advance();
                self.function_type(start, false)
            }
            TokenKind::Unsafe | TokenKind::Extern => self.fail(
                137,
                "a C function pointer type is not supported yet",
                start,
            ),
            TokenKind::Star => self.fail(137, "raw pointer types `*const T` and `*mut T` are not supported yet", start),
            TokenKind::Bang => self.fail(137, "the never type `!` is not written in Aelys", start),
            TokenKind::Impl | TokenKind::Dyn => {
                let is_dyn = self.peek().kind == TokenKind::Dyn;
                self.advance();
                if self.closure_trait_ahead() {
                    return self.closure_trait_type(start);
                }
                if !is_dyn {
                    return self.fail_help(
                        137,
                        "`impl Trait` is not supported; take a type parameter `<T: Trait>`",
                        "`impl Fn(A) -> R` is accepted and means the closure type `fn(A) -> R`",
                        start.merge(self.peek().span),
                    );
                }
                let mut ann = self.path_type()?;
                ann.dyn_trait = true;
                ann.span = start.merge(ann.span);
                Ok(ann)
            }
            TokenKind::Identifier(w) if w == "_" => self.fail(137, "`_` is not a type here; write the type", start),
            TokenKind::Identifier(_) | TokenKind::Crate | TokenKind::Super => {
                if self.closure_trait_ahead() {
                    return self.closure_trait_type(start);
                }
                self.path_type()
            }
            _ => self.unexpected("a type"),
        }
    }

    fn reference_type(&mut self, start: Span, mutable: bool) -> Result<TypeAnnotation> {
        let kind = if mutable { RefKind::Mut } else { RefKind::Shared };
        if self.match_token(&TokenKind::LBracket) {
            let element = self.parse_type()?;
            self.consume(&TokenKind::RBracket, "`]`")?;
            let mut ann = TypeAnnotation::slice_referent(element, start.merge(self.previous().span));
            ann.reference = Some(kind);
            return Ok(ann);
        }
        if self.check_ident("str") {
            self.advance();
            return Ok(TypeAnnotation::new("string".to_string(), start.merge(self.previous().span)));
        }
        let mut referent = self.parse_type()?;
        referent.reference = Some(kind);
        referent.span = start.merge(referent.span);
        Ok(referent)
    }

    pub(super) fn unit_type_refusal(&self, open: Span) -> aelys_common::error::AelysError {
        let kind = CompileErrorKind::UnexpectedToken {
            expected: "a type, the unit type is written by leaving out the return type: \
                       `fn f()` rather than `fn f() -> ()`"
                .to_string(),
            found: "()".to_string(),
        };
        aelys_common::error::CompileError::new(kind, open.merge(self.previous().span), self.source.clone())
            .into()
    }

    fn closure_trait_ahead(&self) -> bool {
        matches!(&self.peek().kind, TokenKind::Identifier(w) if matches!(w.as_str(), "Fn" | "FnMut" | "FnOnce"))
            && self.peek_at(1).kind == TokenKind::LParen
    }

    fn closure_trait_type(&mut self, start: Span) -> Result<TypeAnnotation> {
        self.advance();
        self.function_type(start, false)
    }

    fn function_type(&mut self, start: Span, nogc: bool) -> Result<TypeAnnotation> {
        self.consume(&TokenKind::LParen, "`(`")?;
        let mut params = Vec::new();
        while !self.check(&TokenKind::RParen) {
            params.push(self.parse_type()?);
            if !self.match_token(&TokenKind::Comma) {
                break;
            }
        }
        self.consume(&TokenKind::RParen, "`)`")?;
        let ret = if self.match_token(&TokenKind::Arrow) {
            self.parse_type()?
        } else {
            TypeAnnotation::new("null".to_string(), self.previous().span)
        };
        Ok(TypeAnnotation::function_type(params, ret, nogc, start.merge(self.previous().span)))
    }

    pub(super) fn type_path_name(&mut self) -> Result<String> {
        let mut segments = Vec::new();
        loop {
            match self.peek().kind.clone() {
                TokenKind::Crate if segments.is_empty() => {
                    self.advance();
                    self.consume(&TokenKind::ColonColon, "`::` after `crate`")?;
                    continue;
                }
                TokenKind::Super => {
                    return self.fail(137, "`super::` paths are not supported yet", self.peek().span);
                }
                _ => segments.push(self.consume_identifier("a type name")?),
            }
            if self.check(&TokenKind::Dot) && matches!(self.peek_at(1).kind, TokenKind::Identifier(_)) {
                let mut path = segments.join("::");
                let mut i = 1;
                while let TokenKind::Identifier(s) = &self.peek_at(i).kind {
                    path.push_str("::");
                    path.push_str(s);
                    if self.peek_at(i + 1).kind != TokenKind::Dot {
                        break;
                    }
                    i += 2;
                }
                return self.fail(
                    115,
                    &format!("a module path is written with `::`: write `{path}`"),
                    self.peek().span,
                );
            }
            if !(self.check(&TokenKind::ColonColon) && matches!(self.peek_at(1).kind, TokenKind::Identifier(_))) {
                break;
            }
            self.advance();
        }
        if segments.len() > 1 && segments[0] == "std" && segments.last().is_some_and(|s| s == "ExitCode") {
            return Ok("i64".to_string());
        }
        if segments.len() == 1 {
            return Ok(canonical_type_name(&segments[0]).to_string());
        }
        let fmt_item = match segments.as_slice() {
            [m, item] | [_, m, item] if m == "fmt" && (segments.len() == 2 || segments[0] == "std") => Some(item.as_str()),
            _ => None,
        };
        match fmt_item {
            Some("Display") => Ok("Display".to_string()),
            Some("Result") => Ok("string".to_string()),
            Some("Formatter") => Ok("fmt.Formatter".to_string()),
            _ => Ok(segments.join(".")),
        }
    }

    fn path_type(&mut self) -> Result<TypeAnnotation> {
        let start = self.peek().span;
        let name = self.type_path_name()?;
        if !self.check(&TokenKind::Lt) {
            return Ok(TypeAnnotation::new(name, start.merge(self.previous().span)));
        }
        self.advance();
        let mut args = Vec::new();
        loop {
            if let TokenKind::Lifetime(l) = self.peek().kind.clone() {
                return self.fail(123, &format!("Aelys has no lifetime annotations; remove `'{l}`"), self.peek().span);
            }
            args.push(self.parse_type()?);
            if !self.match_token(&TokenKind::Comma) || matches!(self.peek().kind, TokenKind::Gt | TokenKind::Shr) {
                break;
            }
        }
        self.consume_gt()?;
        let span = start.merge(self.previous().span);
        Ok(if args.len() == 1 {
            TypeAnnotation::with_param(name, args.pop().unwrap(), span)
        } else {
            TypeAnnotation::with_params(name, args, span)
        })
    }

    pub(super) fn parse_cast_target(&mut self) -> Result<TypeAnnotation> {
        if let TokenKind::Identifier(name) = self.peek().kind.clone()
            && is_primitive(&name)
            && self.peek_at(1).kind != TokenKind::ColonColon
        {
            let span = self.peek().span;
            self.advance();
            return Ok(TypeAnnotation::new(canonical_type_name(&name).to_string(), span));
        }
        self.parse_type()
    }

    pub(super) fn generic_params(&mut self) -> Result<Generics> {
        let mut generics = Generics {
            params: Vec::new(),
            bounds: Vec::new(),
            closures: Vec::new(),
        };
        if !self.match_token(&TokenKind::Lt) {
            return Ok(generics);
        }
        loop {
            if let TokenKind::Lifetime(l) = self.peek().kind.clone() {
                return self.fail(123, &format!("Aelys has no lifetime annotations; remove `'{l}`"), self.peek().span);
            }
            if self.check(&TokenKind::Const) {
                return self.fail(137, "const generic parameters are not supported", self.peek().span);
            }
            let name = self.consume_identifier("a type parameter")?;
            let mut set = TypeBounds::default();
            if self.match_token(&TokenKind::Colon) {
                if let Some(closure) = self.parse_bound_list(&mut set)? {
                    generics.closures.push((name.clone(), closure));
                }
            }
            generics.params.push(name);
            generics.bounds.push(set);
            if !self.match_token(&TokenKind::Comma) || matches!(self.peek().kind, TokenKind::Gt | TokenKind::Shr) {
                break;
            }
        }
        self.consume_gt()?;
        Ok(generics)
    }

    // a closure bound `F: Fn(A) -> R` names the managed closure type itself, so the parameter is substituted away
    pub(super) fn parse_bound_list(&mut self, set: &mut TypeBounds) -> Result<Option<TypeAnnotation>> {
        let mut closure = None;
        loop {
            if self.closure_trait_ahead() {
                let start = self.peek().span;
                closure = Some(self.closure_trait_type(start)?);
            } else {
                self.parse_type_bound(set)?;
            }
            if !self.match_token(&TokenKind::Plus) {
                break;
            }
        }
        Ok(closure)
    }

    fn parse_type_bound(&mut self, set: &mut TypeBounds) -> Result<()> {
        if self.match_token(&TokenKind::Nogc) {
            set.nogc = true;
            return Ok(());
        }
        let span = self.peek().span;
        if let TokenKind::Lifetime(l) = self.peek().kind.clone() {
            return self.fail(123, &format!("Aelys has no lifetime annotations; remove `'{l}`"), span);
        }
        let name = self.type_path_name()?;
        match name.as_str() {
            "PartialEq" | "Eq" => {
                set.eq = true;
                return Ok(());
            }
            "Ord" | "PartialOrd" => {
                set.ord = true;
                return Ok(());
            }
            "eq" | "ord" => {
                let new = if name == "eq" { "PartialEq" } else { "Ord" };
                return self.fail(301, &format!("the bound `{name}` is written `{new}`"), span);
            }
            _ => {}
        }
        let mut args = Vec::new();
        if self.match_token(&TokenKind::Lt) {
            loop {
                args.push(self.parse_type()?);
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
            self.consume_gt()?;
        }
        let r = TraitRef {
            name,
            span: span.merge(self.previous().span),
            args,
        };
        if !set.traits.contains(&r) {
            set.traits.push(r);
        }
        Ok(())
    }

    pub(super) fn parse_where_clause(&mut self, generics: &mut Generics) -> Result<()> {
        if !self.match_token(&TokenKind::Where) {
            return Ok(());
        }
        loop {
            let span = self.peek().span;
            let name = self.consume_identifier("a type parameter")?;
            self.consume(&TokenKind::Colon, "`:`")?;
            let mut set = TypeBounds::default();
            let closure = self.parse_bound_list(&mut set)?;
            let Some(i) = generics.params.iter().position(|p| *p == name) else {
                return self.fail(
                    106,
                    &format!("`where` names `{name}`, which is not a type parameter of this item"),
                    span,
                );
            };
            if let Some(closure) = closure {
                generics.closures.push((name, closure));
            }
            generics.bounds[i].merge(&set);
            if !self.match_token(&TokenKind::Comma) || self.check(&TokenKind::LBrace) {
                break;
            }
        }
        Ok(())
    }

    pub(super) fn parse_parameter(&mut self) -> Result<Parameter> {
        let span = self.peek().span;
        if self.receiver_ahead().is_some() {
            return Err(self.error(CompileErrorKind::MethodSyntax {
                message: Self::self_param_message(),
            }));
        }
        let mutable = self.match_token(&TokenKind::Mut);
        let name = self.consume_value_name("a parameter name")?;
        let type_annotation = if self.match_token(&TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        Ok(Parameter::new(name, mutable, type_annotation, span.merge(self.previous().span)))
    }
}

pub(super) fn substitute_closure_params(ann: &mut TypeAnnotation, closures: &[(String, TypeAnnotation)]) {
    if ann.reference.is_none()
        && ann.type_param.is_none()
        && ann.type_params.is_empty()
        && let Some((_, closure)) = closures.iter().find(|(n, _)| *n == ann.name)
    {
        let span = ann.span;
        *ann = closure.clone();
        ann.span = span;
        return;
    }
    if let Some(p) = ann.type_param.as_deref_mut() {
        substitute_closure_params(p, closures);
    }
    for p in &mut ann.type_params {
        substitute_closure_params(p, closures);
    }
    for p in ann.fn_params.iter_mut().flatten() {
        substitute_closure_params(p, closures);
    }
    if let Some(r) = ann.fn_ret.as_deref_mut() {
        substitute_closure_params(r, closures);
    }
    for p in ann.tuple_elems.iter_mut().flatten() {
        substitute_closure_params(p, closures);
    }
}
