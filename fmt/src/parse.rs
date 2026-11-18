use std::cmp::Ordering;

use crate::Error;
use crate::doc::*;
use crate::lexer::{KEYWORDS, Kind, Lexed};

const MAX_DEPTH: usize = 400;

enum Entry {
    Tok(usize),
    Op(usize, usize),
    Key(usize, String),
}

pub struct Parsed {
    pub doc: Doc,
    pub norm: Vec<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Ov {
    Closure,
    Block,
    ParenMacro,
    BracketMacro,
    Control,
    Delimited,
    Call,
    Other,
}

struct E {
    doc: Doc,
    ov: Ov,
    bin: Option<Bin>,
    jump: bool,
    block_like: bool,
}

struct Bin {
    prec: u8,
    first: Doc,
    rest: Vec<(Doc, Doc)>,
    control: bool,
}

impl Bin {
    fn joinable(&self) -> bool {
        !self.control
            && !self.rest.iter().any(|(op, _)| op.brk())
            && (self.first.brk() || self.rest.iter().any(|(_, r)| r.brk()))
    }
}

impl E {
    fn new(doc: Doc, ov: Ov) -> E {
        E {
            doc,
            ov,
            bin: None,
            jump: false,
            block_like: false,
        }
    }

    fn doc(&self) -> Doc {
        let Some(b) = &self.bin else {
            return self.doc.clone();
        };
        let g = group(concat(vec![b.first.clone(), indent(bin_rest(b))]));
        if !b.joinable() {
            return g;
        }
        let mut joined = vec![b.first.clone()];
        for (op, rhs) in &b.rest {
            joined.extend([space(), op.clone(), space(), rhs.clone()]);
        }
        choice(vec![concat(joined), g])
    }

    fn hugs(&self) -> bool {
        self.ov != Ov::Other || self.bin.as_ref().is_some_and(Bin::joinable)
    }
}

fn bin_rest(b: &Bin) -> Doc {
    let mut v = Vec::new();
    for (op, rhs) in &b.rest {
        v.push(line());
        v.push(op.clone());
        v.push(space());
        v.push(rhs.clone());
    }
    concat(v)
}

#[derive(Clone, Copy, Default)]
struct R {
    no_struct: bool,
}

struct ListItem {
    doc: Doc,
    comma: Option<Doc>,
    has_comma: bool,
    blank: bool,
}

struct List {
    open: Doc,
    items: Vec<ListItem>,
    close_lead: Option<Doc>,
    close: Doc,
}

impl List {
    fn has_comments(&self) -> bool {
        self.close_lead.is_some()
            || self
                .items
                .iter()
                .any(|i| i.comma.is_some() || split_trailing(&i.doc).1.is_some())
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Tr {
    Vertical,
    Never,
    Always,
}

struct Block {
    open: Doc,
    stmts: Vec<(Doc, bool)>,
    tail: Option<Doc>,
    close_lead: Option<(Doc, bool)>,
    close: Doc,
}

struct Item {
    doc: Doc,
    blank: bool,
    sort: Option<(u8, UseTree, usize)>,
}

#[derive(Clone)]
struct UseTree {
    vis: String,
    segs: Vec<Seg>,
    doc: Doc,
}

#[derive(Clone)]
enum SegKind {
    Name(String),
    Glob,
    List(Vec<UseTree>),
}

#[derive(Clone)]
struct Seg {
    kind: SegKind,
    alias: Option<String>,
}

pub struct Parser<'s> {
    src: &'s str,
    lx: Lexed,
    pos: usize,
    norm: Vec<Entry>,
    consumed: Vec<usize>,
    run: usize,
    depth: usize,
    hoisted: Vec<bool>,
}

type Res<T> = Result<T, Error>;

fn is_keyword(s: &str) -> bool {
    KEYWORDS.contains(&s)
}

pub fn parse(src: &str, lx: Lexed) -> Res<Parsed> {
    let hoisted = vec![false; lx.tokens.len()];
    let mut p = Parser {
        src,
        lx,
        pos: 0,
        norm: Vec::new(),
        consumed: Vec::new(),
        run: 0,
        depth: 0,
        hoisted,
    };
    let doc = p.file()?;
    let mut seen = std::mem::take(&mut p.consumed);
    seen.sort_unstable();
    seen.dedup();
    if seen.len() != p.lx.tokens.len() - 1 {
        return Err(Error::internal(crate::Check::Tokens, 0));
    }
    let norm = p.norm_strings();
    Ok(Parsed { doc, norm })
}

impl<'s> Parser<'s> {
    fn norm_strings(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.norm.len() {
            match &self.norm[i] {
                Entry::Tok(t) => out.push(self.text(*t).to_string()),
                Entry::Op(a, b) => {
                    out.push(self.src[self.lx.tokens[*a].start..self.lx.tokens[*b].end].to_string())
                }
                Entry::Key(run, _) => {
                    let mut keys = Vec::new();
                    while let Some(Entry::Key(r, k)) = self.norm.get(i) {
                        if r != run {
                            break;
                        }
                        keys.push(k.clone());
                        i += 1;
                    }
                    keys.sort();
                    out.extend(keys);
                    continue;
                }
            }
            i += 1;
        }
        out
    }

    fn text(&self, i: usize) -> &'s str {
        let t = &self.lx.tokens[i];
        &self.src[t.start..t.end]
    }

    fn at(&self, n: usize) -> &'s str {
        let i = (self.pos + n).min(self.lx.tokens.len() - 1);
        self.text(i)
    }

    fn peek(&self) -> &'s str {
        self.at(0)
    }

    fn is(&self, s: &str) -> bool {
        self.peek() == s && self.kind(0) != Kind::Literal
    }

    fn kind(&self, n: usize) -> Kind {
        let i = (self.pos + n).min(self.lx.tokens.len() - 1);
        self.lx.tokens[i].kind
    }

    fn is_ident(&self, n: usize) -> bool {
        self.kind(n) == Kind::Ident && !is_keyword(self.at(n))
    }

    fn eof(&self) -> bool {
        self.kind(0) == Kind::Eof
    }

    fn err<T>(&self, msg: impl Into<String>) -> Res<T> {
        let off = self.lx.tokens[self.pos.min(self.lx.tokens.len() - 1)].start;
        Err(Error::syntax(off, msg))
    }

    fn expected<T>(&self, what: &str) -> Res<T> {
        if self.eof() {
            self.err(format!("expected {what}, found end of file"))
        } else {
            self.err(format!("expected {what}, found `{}`", self.peek()))
        }
    }

    fn bump(&mut self) -> usize {
        let i = self.pos;
        if self.kind(0) != Kind::Eof {
            self.consumed.push(i);
            self.pos += 1;
        }
        i
    }

    fn comment_text(&self, c: usize) -> Doc {
        let c = &self.lx.comments[c];
        text(&self.src[c.start..c.end])
    }

    fn lead(&self, i: usize) -> Option<(Doc, Doc)> {
        let ids = &self.lx.leading[i];
        if ids.is_empty() {
            return None;
        }
        let mut v = Vec::new();
        for (k, &c) in ids.iter().enumerate() {
            let cm = self.lx.comments[c];
            if k == 0 {
                v.push(freshline());
            } else {
                let prev = self.lx.comments[ids[k - 1]];
                if cm.newlines_before >= 2 {
                    v.push(blankline());
                } else if cm.newlines_before == 1 || prev.line {
                    v.push(hardline());
                } else {
                    v.push(space());
                }
            }
            v.push(self.comment_text(c));
        }
        let last = self.lx.comments[*ids.last().unwrap()];
        let nl = self.lx.tokens[i].newlines_before;
        let term = if nl >= 2 {
            blankline()
        } else if nl == 1 || last.line {
            hardline()
        } else {
            space()
        };
        Some((concat(v), term))
    }

    fn trail(&self, i: usize) -> Option<Doc> {
        let ids = &self.lx.trailing[i];
        if ids.is_empty() {
            return None;
        }
        let mut v = Vec::new();
        for &c in ids {
            let cm = self.lx.comments[c];
            v.push(space());
            v.push(self.comment_text(c));
            if cm.line || cm.newlines_after > 0 {
                v.push(break_after());
            }
        }
        Some(trail(concat(v)))
    }

    fn blank_before(&self, i: usize) -> bool {
        match self.lx.leading[i].first() {
            Some(&c) => self.lx.comments[c].newlines_before >= 2,
            None => self.lx.tokens[i].newlines_before >= 2,
        }
    }

    fn hoist(&mut self) -> Option<Doc> {
        let (l, t) = self.lead(self.pos)?;
        self.hoisted[self.pos] = true;
        Some(concat(vec![l, t]))
    }

    fn tok_doc(&self, i: usize, body: Doc) -> Doc {
        let lead = if self.hoisted[i] { None } else { self.lead(i) };
        let tr = self.trail(i);
        if lead.is_none() && tr.is_none() {
            return body;
        }
        let mut v = Vec::new();
        if let Some((l, t)) = lead {
            v.push(l);
            v.push(t);
        }
        v.push(body);
        if let Some(t) = tr {
            v.push(t);
        }
        concat(v)
    }

    fn tok(&mut self) -> Doc {
        let i = self.bump();
        self.norm.push(Entry::Tok(i));
        self.tok_doc(i, text(self.text(i)))
    }

    fn drop_tok(&mut self) -> Doc {
        let i = self.bump();
        self.tok_doc(i, nil())
    }

    fn take_comments(&mut self) -> Option<Doc> {
        let i = self.bump();
        if self.lx.leading[i].is_empty() && self.lx.trailing[i].is_empty() {
            None
        } else {
            Some(self.tok_doc(i, nil()))
        }
    }

    fn close_tok(&mut self) -> (Option<(Doc, bool)>, Doc) {
        let i = self.bump();
        self.norm.push(Entry::Tok(i));
        let lead = self.lead(i).map(|(l, _)| (l, self.blank_before(i)));
        let body = match self.trail(i) {
            Some(t) => concat(vec![text(self.text(i)), t]),
            None => text(self.text(i)),
        };
        (lead, body)
    }

    fn expect(&mut self, s: &str) -> Res<Doc> {
        if self.is(s) {
            Ok(self.tok())
        } else {
            self.expected(&format!("`{s}`"))
        }
    }

    fn ident(&mut self) -> Res<Doc> {
        if self.kind(0) == Kind::Ident {
            Ok(self.tok())
        } else {
            self.expected("an identifier")
        }
    }

    fn enter(&mut self) -> Res<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return self.err("nesting too deep to format");
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn file(&mut self) -> Res<Doc> {
        let items = self.items(true)?;
        let eof = self.pos;
        let mut v = vec![items_doc(items)];
        if let Some((l, _)) = self.lead(eof) {
            if self.blank_before(eof) && self.pos > 0 {
                v.push(blankline());
            }
            v.push(l);
        }
        Ok(concat(v))
    }

    fn items(&mut self, top: bool) -> Res<Vec<Item>> {
        let mut items: Vec<Item> = Vec::new();
        while !(self.eof() || (!top && self.is("}"))) {
            let blank = self.blank_before(self.pos);
            let lead = self.hoist();
            let mut item = self.item()?;
            item.blank = blank;
            if let Some(l) = lead {
                item.doc = concat(vec![l, item.doc]);
            }
            if let Some((kind, _, entry)) = &item.sort {
                let same = items
                    .last()
                    .is_some_and(|p| !blank && p.sort.as_ref().is_some_and(|(k, _, _)| k == kind));
                if !same {
                    self.run += 1;
                }
                if let Entry::Key(r, _) = &mut self.norm[*entry] {
                    *r = self.run;
                }
            }
            items.push(item);
        }
        Ok(sort_runs(items))
    }

    fn attrs(&mut self) -> Res<(Vec<Doc>, bool)> {
        let mut v = Vec::new();
        let mut skip = false;
        while self.is("#") || self.is("#!") {
            let start = self.pos;
            let mut d = vec![self.tok()];
            if self.is("!") {
                d.push(self.tok());
            }
            if !self.is("[") {
                return self.expected("`[`");
            }
            let tt = self.token_tree()?;
            let raw: String = (start..self.pos).map(|i| self.text(i)).collect();
            skip |= raw == "#[fmt::skip]";
            d.push(tt);
            v.push(concat(d));
        }
        Ok((v, skip))
    }

    fn with_attrs(attrs: Vec<Doc>, d: Doc) -> Doc {
        let mut v = Vec::new();
        for a in attrs {
            v.push(a);
            v.push(hardline());
        }
        v.push(d);
        concat(v)
    }

    fn verbatim(&self, start: usize) -> Doc {
        let first = start;
        let last = self.pos - 1;
        let body = text(&self.src[self.lx.tokens[first].start..self.lx.tokens[last].end]);
        let mut v = Vec::new();
        if let Some((l, t)) = self.lead(first).filter(|_| !self.hoisted[first]) {
            v.push(l);
            v.push(t);
        }
        v.push(body);
        if let Some(t) = self.trail(last) {
            v.push(t);
        }
        concat(v)
    }

    fn vis(&mut self) -> Res<Option<Doc>> {
        if !self.is("pub") {
            return Ok(None);
        }
        let mut v = vec![self.tok()];
        if self.is("(") && matches!(self.at(1), "crate" | "super" | "self" | "in") {
            v.push(self.token_tree()?);
        }
        v.push(space());
        Ok(Some(concat(v)))
    }

    fn starts_item(&self) -> bool {
        match self.peek() {
            "fn" | "pub" | "struct" | "enum" | "trait" | "impl" | "use" | "mod" | "static"
            | "type" => !(self.peek() == "type" && self.at(1) == "("),
            "const" => self.is_ident(1) || self.at(1) == "fn" || self.at(1) == "_",
            "extern" => {
                matches!(self.kind(1), Kind::Literal) || self.at(1) == "{" || self.at(1) == "fn"
            }
            "nogc" => self.at(1) == "fn",
            "unsafe" => matches!(self.at(1), "fn" | "extern" | "impl" | "trait"),
            "#" | "#!" => true,
            _ => false,
        }
    }

    fn item(&mut self) -> Res<Item> {
        let start = self.pos;
        let (attrs, skip) = self.attrs()?;
        let norm_mark = self.norm.len();
        let has_attrs = !attrs.is_empty();
        let vis = self.vis()?;
        let vis_text = if vis.is_some() {
            "pub ".to_string()
        } else {
            String::new()
        };
        let mut sort = None;
        let mut head = Vec::new();
        if let Some(v) = vis {
            head.push(v);
        }
        let body = match self.peek() {
            "use" => {
                let u = self.use_item(vis_text)?;
                if !has_attrs {
                    let key = format!("use {};", use_key(&u));
                    self.norm.truncate(norm_mark);
                    sort = Some((0, u.clone(), self.norm.len()));
                    self.norm.push(Entry::Key(0, key));
                }
                u.doc
            }
            "mod" => {
                let m = self.tok();
                let name_i = self.pos;
                let name = self.ident()?;
                if self.is(";") {
                    let semi = self.tok();
                    if !has_attrs {
                        let key = format!("{vis_text}mod {};", self.text(name_i));
                        self.norm.truncate(norm_mark);
                        let tree = UseTree {
                            vis: vis_text.clone(),
                            segs: vec![Seg {
                                kind: SegKind::Name(self.text(name_i).to_string()),
                                alias: None,
                            }],
                            doc: nil(),
                        };
                        sort = Some((1, tree, self.norm.len()));
                        self.norm.push(Entry::Key(0, key));
                    }
                    concat(vec![m, space(), name, semi])
                } else {
                    let b = self.item_block()?;
                    concat(vec![m, space(), name, space(), b])
                }
            }
            "struct" => self.struct_item()?,
            "enum" => self.enum_item()?,
            "trait" => self.trait_item()?,
            "impl" => self.impl_item()?,
            "const" | "static" if self.at(1) != "fn" => self.const_item()?,
            "type" => self.type_item()?,
            "extern" if self.at(2) == "{" || self.at(1) == "{" => self.extern_block()?,
            "unsafe" if self.at(1) == "extern" && self.at(3) == "{" => {
                let u = self.tok();
                let b = self.extern_block()?;
                concat(vec![u, space(), b])
            }
            "unsafe" if self.at(1) == "impl" => {
                let u = self.tok();
                let b = self.impl_item()?;
                concat(vec![u, space(), b])
            }
            _ => self.fn_item()?,
        };
        head.push(body);
        if skip {
            return Ok(Item {
                doc: self.verbatim(start),
                blank: false,
                sort: None,
            });
        }
        let doc = Self::with_attrs(attrs, concat(head));
        Ok(Item {
            doc,
            blank: false,
            sort,
        })
    }

    fn item_block(&mut self) -> Res<Doc> {
        let open = self.expect("{")?;
        let items = self.items(false)?;
        let (lead, close) = self.close_tok();
        Ok(seq_block(
            open,
            items.into_iter().map(|i| (i.doc, i.blank)).collect(),
            lead,
            close,
        ))
    }

    fn fn_item(&mut self) -> Res<Doc> {
        let mut v = Vec::new();
        loop {
            match self.peek() {
                "const" | "unsafe" | "nogc" | "async" => {
                    v.push(self.tok());
                    v.push(space());
                }
                "extern" => {
                    v.push(self.tok());
                    v.push(space());
                    if self.kind(0) == Kind::Literal {
                        v.push(self.tok());
                        v.push(space());
                    }
                }
                _ => break,
            }
        }
        v.push(self.expect("fn")?);
        v.push(space());
        v.push(self.ident()?);
        if self.is("<") {
            v.push(self.generics()?);
        }
        let l = self.list("(", ")", false, |p| p.param())?;
        v.push(layout(l, false, usize::MAX, Tr::Vertical, false));
        if self.is("->") {
            v.push(space());
            v.push(self.tok());
            v.push(space());
            v.push(self.ty()?);
        }
        let wh = self.where_clause()?;
        let has_where = wh.is_some();
        if let Some(w) = wh {
            v.push(w);
        }
        if self.is(";") {
            v.push(self.tok());
        } else {
            if !has_where {
                v.push(space());
            }
            v.push(self.block_doc()?);
        }
        Ok(concat(v))
    }

    fn param(&mut self) -> Res<Doc> {
        let (attrs, _) = self.attrs()?;
        let mut v = Vec::new();
        for a in attrs {
            v.push(a);
            v.push(space());
        }
        if self.is("...") {
            v.push(self.tok());
            return Ok(concat(v));
        }
        if self.is("&") || self.is("&&") {
            let save = self.pos;
            let mut r = vec![self.tok()];
            if self.kind(0) == Kind::Lifetime {
                r.push(self.tok());
                r.push(space());
            }
            if self.is("mut") {
                r.push(self.tok());
                r.push(space());
            }
            if self.is("self") {
                r.push(self.tok());
                v.extend(r);
                return Ok(concat(v));
            }
            self.restore(save);
        }
        if self.is("mut") && self.at(1) == "self" {
            v.push(self.tok());
            v.push(space());
        }
        if self.is("self") && self.at(1) != ":" {
            v.push(self.tok());
            return Ok(concat(v));
        }
        v.push(self.pat_no_top()?);
        if self.is(":") {
            v.push(self.tok());
            v.push(space());
            v.push(self.ty()?);
        }
        Ok(concat(v))
    }

    fn restore(&mut self, pos: usize) {
        while self.consumed.last().is_some_and(|&i| i >= pos) {
            self.consumed.pop();
        }
        while self.norm.last().is_some_and(|e| match e {
            Entry::Tok(i) | Entry::Op(i, _) => *i >= pos,
            Entry::Key(..) => false,
        }) {
            self.norm.pop();
        }
        self.pos = pos;
    }

    fn generics(&mut self) -> Res<Doc> {
        let mut v = vec![self.expect("<")?];
        let mut first = true;
        while !self.is(">") {
            if !first {
                v.push(self.expect(",")?);
                if self.is(">") {
                    break;
                }
                v.push(space());
            }
            first = false;
            let (attrs, _) = self.attrs()?;
            for a in attrs {
                v.push(a);
                v.push(space());
            }
            if self.is("const") {
                v.push(self.tok());
                v.push(space());
            }
            if self.kind(0) == Kind::Lifetime {
                v.push(self.tok());
            } else {
                v.push(self.ident()?);
            }
            if self.is(":") {
                v.push(self.tok());
                if !self.is(",") && !self.is(">") {
                    v.push(space());
                    v.push(self.bounds()?);
                }
            }
            if self.is("=") {
                v.push(space());
                v.push(self.tok());
                v.push(space());
                v.push(self.ty()?);
            }
        }
        v.push(self.tok());
        Ok(concat(v))
    }

    fn bounds(&mut self) -> Res<Doc> {
        let mut v = vec![self.bound()?];
        while self.is("+") {
            v.push(space());
            v.push(self.tok());
            v.push(space());
            v.push(self.bound()?);
        }
        Ok(concat(v))
    }

    fn bound(&mut self) -> Res<Doc> {
        if self.kind(0) == Kind::Lifetime {
            return Ok(self.tok());
        }
        if self.is("?") {
            let q = self.tok();
            return Ok(concat(vec![q, self.ty_path()?]));
        }
        if self.is("(") {
            let o = self.tok();
            let b = self.bounds()?;
            let c = self.expect(")")?;
            return Ok(concat(vec![o, b, c]));
        }
        if self.is("nogc") {
            return Ok(self.tok());
        }
        self.ty_path()
    }

    fn where_clause(&mut self) -> Res<Option<Doc>> {
        if !self.is("where") {
            return Ok(None);
        }
        let w = self.tok();
        let mut preds = Vec::new();
        while !self.is("{") && !self.is(";") && !self.eof() {
            let mut p = Vec::new();
            if self.kind(0) == Kind::Lifetime {
                p.push(self.tok());
            } else {
                p.push(self.ty()?);
            }
            p.push(self.expect(":")?);
            p.push(space());
            p.push(self.bounds()?);
            let comma = if self.is(",") { self.drop_tok() } else { nil() };
            preds.push(concat(vec![hardline(), concat(p), text(","), comma]));
        }
        Ok(Some(concat(vec![
            hardline(),
            w,
            indent(concat(preds)),
            hardline(),
        ])))
    }

    fn struct_item(&mut self) -> Res<Doc> {
        let mut v = vec![self.tok(), space(), self.ident()?];
        if self.is("<") {
            v.push(self.generics()?);
        }
        if self.is("(") {
            let l = self.list("(", ")", false, |p| p.tuple_field())?;
            v.push(layout(l, false, usize::MAX, Tr::Vertical, false));
            if let Some(w) = self.where_clause()? {
                v.push(w);
            }
            v.push(self.expect(";")?);
            return Ok(concat(v));
        }
        let wh = self.where_clause()?;
        let has_where = wh.is_some();
        if let Some(w) = wh {
            v.push(w);
        }
        if self.is(";") {
            v.push(self.tok());
            return Ok(concat(v));
        }
        if !has_where {
            v.push(space());
        }
        let l = self.list("{", "}", false, |p| p.field_def())?;
        v.push(layout(l, true, 0, Tr::Vertical, true));
        Ok(concat(v))
    }

    fn tuple_field(&mut self) -> Res<Doc> {
        let (attrs, _) = self.attrs()?;
        let mut v = Vec::new();
        for a in attrs {
            v.push(a);
            v.push(space());
        }
        if let Some(p) = self.vis()? {
            v.push(p);
        }
        v.push(self.ty()?);
        Ok(concat(v))
    }

    fn field_def(&mut self) -> Res<Doc> {
        let (attrs, _) = self.attrs()?;
        let mut v = Vec::new();
        if let Some(p) = self.vis()? {
            v.push(p);
        }
        v.push(self.ident()?);
        v.push(self.expect(":")?);
        v.push(space());
        v.push(self.ty()?);
        Ok(Self::with_attrs(attrs, concat(v)))
    }

    fn enum_item(&mut self) -> Res<Doc> {
        let mut v = vec![self.tok(), space(), self.ident()?];
        if self.is("<") {
            v.push(self.generics()?);
        }
        let wh = self.where_clause()?;
        if wh.is_none() {
            v.push(space());
        }
        if let Some(w) = wh {
            v.push(w);
        }
        let l = self.list("{", "}", false, |p| p.variant())?;
        v.push(layout(l, true, 0, Tr::Vertical, true));
        Ok(concat(v))
    }

    fn variant(&mut self) -> Res<Doc> {
        let (attrs, _) = self.attrs()?;
        let mut v = vec![self.ident()?];
        if self.is("(") {
            let l = self.list("(", ")", false, |p| p.tuple_field())?;
            v.push(layout(l, false, usize::MAX, Tr::Vertical, false));
        } else if self.is("{") {
            let l = self.list("{", "}", false, |p| p.field_def())?;
            v.push(space());
            v.push(layout(l, true, 35, Tr::Vertical, false));
        }
        if self.is("=") {
            v.push(space());
            v.push(self.tok());
            v.push(space());
            v.push(self.expr(R::default())?.doc());
        }
        Ok(Self::with_attrs(attrs, concat(v)))
    }

    fn trait_item(&mut self) -> Res<Doc> {
        let mut v = vec![self.tok(), space(), self.ident()?];
        if self.is("<") {
            v.push(self.generics()?);
        }
        if self.is(":") {
            v.push(self.tok());
            v.push(space());
            v.push(self.bounds()?);
        }
        let wh = self.where_clause()?;
        if wh.is_none() {
            v.push(space());
        }
        if let Some(w) = wh {
            v.push(w);
        }
        v.push(self.item_block()?);
        Ok(concat(v))
    }

    fn impl_item(&mut self) -> Res<Doc> {
        let mut v = vec![self.tok()];
        if self.is("<") {
            v.push(self.generics()?);
        }
        v.push(space());
        if self.is("!") {
            v.push(self.tok());
        }
        if self.is("nogc") {
            v.push(self.tok());
        } else {
            v.push(self.ty()?);
        }
        if self.is("for") {
            v.push(space());
            v.push(self.tok());
            v.push(space());
            v.push(self.ty()?);
        }
        let wh = self.where_clause()?;
        if wh.is_none() {
            v.push(space());
        }
        if let Some(w) = wh {
            v.push(w);
        }
        v.push(self.item_block()?);
        Ok(concat(v))
    }

    fn const_item(&mut self) -> Res<Doc> {
        let mut v = vec![self.tok(), space()];
        if self.is("mut") {
            v.push(self.tok());
            v.push(space());
        }
        v.push(self.ident()?);
        if self.is(":") {
            v.push(self.tok());
            v.push(space());
            v.push(self.ty()?);
        }
        if self.is("=") {
            v.push(space());
            v.push(self.tok());
            let e = self.expr(R::default())?;
            v.push(rhs(&e));
        }
        v.push(self.expect(";")?);
        Ok(concat(v))
    }

    fn type_item(&mut self) -> Res<Doc> {
        let mut v = vec![self.tok(), space(), self.ident()?];
        if self.is("<") {
            v.push(self.generics()?);
        }
        if self.is(":") {
            v.push(self.tok());
            v.push(space());
            v.push(self.bounds()?);
        }
        if self.is("=") {
            v.push(space());
            v.push(self.tok());
            v.push(space());
            v.push(self.ty()?);
        }
        v.push(self.expect(";")?);
        Ok(concat(v))
    }

    fn extern_block(&mut self) -> Res<Doc> {
        let mut v = vec![self.tok(), space()];
        if self.kind(0) == Kind::Literal {
            v.push(self.tok());
            v.push(space());
        }
        v.push(self.item_block()?);
        Ok(concat(v))
    }

    fn use_item(&mut self, vis: String) -> Res<UseTree> {
        let u = self.tok();
        let mut tree = self.use_tree()?;
        let semi = self.expect(";")?;
        tree.vis = vis;
        tree.doc = concat(vec![u, space(), tree.doc, semi]);
        Ok(tree)
    }

    fn use_tree(&mut self) -> Res<UseTree> {
        let mut segs = Vec::new();
        let mut v = Vec::new();
        if self.is("::") {
            v.push(self.tok());
        }
        loop {
            if self.is("*") {
                v.push(self.tok());
                segs.push(Seg {
                    kind: SegKind::Glob,
                    alias: None,
                });
                break;
            }
            if self.is("{") {
                let mut trees = Vec::new();
                let l = self.list("{", "}", false, |p| {
                    let t = p.use_tree()?;
                    let d = t.doc.clone();
                    trees.push(t);
                    Ok(d)
                })?;
                let mut order: Vec<usize> = (0..trees.len()).collect();
                order.sort_by(|&a, &b| tree_cmp(&trees[a], &trees[b]));
                let simple = trees.len() == 1
                    && !l.has_comments()
                    && matches!(trees[0].segs.first().map(|s| &s.kind), Some(SegKind::Name(n)) if n != "self");
                if simple {
                    let t = trees.pop().unwrap();
                    v.push(t.doc.clone());
                    segs.extend(t.segs);
                } else {
                    let mut items = l.items;
                    let mut sorted = Vec::new();
                    let mut slots: Vec<Option<ListItem>> = items.drain(..).map(Some).collect();
                    for &i in &order {
                        sorted.push(slots[i].take().unwrap());
                    }
                    let l = List {
                        open: l.open,
                        items: sorted,
                        close_lead: l.close_lead,
                        close: l.close,
                    };
                    v.push(layout(l, false, usize::MAX, Tr::Vertical, false));
                    let sorted_trees = order.iter().map(|&i| trees[i].clone()).collect();
                    segs.push(Seg {
                        kind: SegKind::List(sorted_trees),
                        alias: None,
                    });
                }
                break;
            }
            let i = self.pos;
            v.push(self.ident()?);
            segs.push(Seg {
                kind: SegKind::Name(self.text(i).to_string()),
                alias: None,
            });
            if self.is("::") {
                v.push(self.tok());
                continue;
            }
            if self.is("as") {
                v.push(space());
                v.push(self.tok());
                v.push(space());
                let a = self.pos;
                v.push(self.ident()?);
                segs.last_mut().unwrap().alias = Some(self.text(a).to_string());
            }
            break;
        }
        Ok(UseTree {
            vis: String::new(),
            segs,
            doc: concat(v),
        })
    }

    fn token_tree(&mut self) -> Res<Doc> {
        let open = self.peek();
        let close = match open {
            "(" => ")",
            "[" => "]",
            "{" => "}",
            _ => return self.expected("a delimiter"),
        };
        let mut v = vec![self.tok()];
        let mut prev = open.to_string();
        while !self.is(close) {
            if self.eof() {
                return self.expected(&format!("`{close}`"));
            }
            let cur = self.peek();
            if tt_space(&prev, cur) {
                v.push(space());
            }
            if matches!(cur, "(" | "[" | "{") && self.kind(0) == Kind::Punct {
                v.push(self.token_tree()?);
                prev = match cur {
                    "(" => ")",
                    "[" => "]",
                    _ => "}",
                }
                .to_string();
            } else {
                prev = cur.to_string();
                v.push(self.tok());
            }
        }
        v.push(self.tok());
        Ok(concat(v))
    }

    fn list(
        &mut self,
        open: &str,
        close: &str,
        tuple: bool,
        mut f: impl FnMut(&mut Self) -> Res<Doc>,
    ) -> Res<List> {
        let open = self.expect(open)?;
        let mut items: Vec<ListItem> = Vec::new();
        while !self.is(close) {
            if self.eof() {
                return self.expected(&format!("`{close}`"));
            }
            let blank = self.blank_before(self.pos);
            let lead = self.hoist();
            let doc = f(self)?;
            let doc = match lead {
                Some(l) => concat(vec![l, doc]),
                None => doc,
            };
            if self.is(",") {
                if self.at(1) != close || (tuple && items.is_empty()) {
                    self.norm.push(Entry::Tok(self.pos));
                }
                let comma = self.take_comments();
                items.push(ListItem {
                    doc,
                    comma,
                    has_comma: true,
                    blank,
                });
            } else {
                items.push(ListItem {
                    doc,
                    comma: None,
                    has_comma: false,
                    blank,
                });
                if !self.is(close) {
                    return self.expected(&format!("`,` or `{close}`"));
                }
            }
        }
        let (lead, close) = self.close_tok();
        Ok(List {
            open,
            items,
            close_lead: lead.map(|(d, _)| d),
            close,
        })
    }

    fn ty(&mut self) -> Res<Doc> {
        self.enter()?;
        let r = self.ty_inner();
        self.leave();
        r
    }

    fn ty_inner(&mut self) -> Res<Doc> {
        match self.peek() {
            "&" | "&&" => {
                let mut v = vec![self.tok()];
                if self.kind(0) == Kind::Lifetime {
                    v.push(self.tok());
                    v.push(space());
                }
                if self.is("mut") {
                    v.push(self.tok());
                    v.push(space());
                }
                v.push(self.ty()?);
                Ok(concat(v))
            }
            "*" => {
                let s = self.tok();
                let m = self.tok();
                Ok(concat(vec![s, m, space(), self.ty()?]))
            }
            "[" => {
                let mut v = vec![self.tok(), self.ty()?];
                if self.is(";") {
                    v.push(self.tok());
                    v.push(space());
                    v.push(self.expr(R::default())?.doc());
                }
                v.push(self.expect("]")?);
                Ok(concat(v))
            }
            "(" => {
                let l = self.list("(", ")", true, |p| p.ty())?;
                let tr = tuple_tr(&l);
                Ok(layout(l, false, usize::MAX, tr, false))
            }
            "!" | "_" => Ok(self.tok()),
            "fn" | "nogc" | "unsafe" | "extern" => {
                let mut v = Vec::new();
                while !self.is("fn") {
                    if self.eof() {
                        return self.expected("`fn`");
                    }
                    v.push(self.tok());
                    v.push(space());
                }
                v.push(self.tok());
                let l = self.list("(", ")", false, |p| p.fn_ty_param())?;
                v.push(layout(l, false, usize::MAX, Tr::Vertical, false));
                if self.is("->") {
                    v.push(space());
                    v.push(self.tok());
                    v.push(space());
                    v.push(self.ty()?);
                }
                Ok(concat(v))
            }
            "dyn" | "impl" => {
                let k = self.tok();
                Ok(concat(vec![k, space(), self.bounds()?]))
            }
            _ => self.ty_path(),
        }
    }

    fn fn_ty_param(&mut self) -> Res<Doc> {
        if self.is("...") {
            return Ok(self.tok());
        }
        if self.is_ident(0) && self.at(1) == ":" && self.at(2) != ":" {
            let n = self.tok();
            let c = self.tok();
            return Ok(concat(vec![n, c, space(), self.ty()?]));
        }
        self.ty()
    }

    fn ty_path(&mut self) -> Res<Doc> {
        let mut v = Vec::new();
        if self.is("::") {
            v.push(self.tok());
        }
        loop {
            let name = self.peek();
            v.push(self.ident()?);
            if self.is("<") && !is_primitive(name) {
                v.push(self.generic_args()?);
            } else if self.is("::") && self.at(1) == "<" {
                v.push(self.tok());
                v.push(self.generic_args()?);
            }
            if self.is("(") && matches!(name, "Fn" | "FnMut" | "FnOnce") {
                let l = self.list("(", ")", false, |p| p.ty())?;
                v.push(layout(l, false, usize::MAX, Tr::Vertical, false));
                if self.is("->") {
                    v.push(space());
                    v.push(self.tok());
                    v.push(space());
                    v.push(self.ty()?);
                }
            }
            if self.is("::") && self.kind(1) == Kind::Ident {
                v.push(self.tok());
                continue;
            }
            break;
        }
        Ok(concat(v))
    }

    fn generic_args(&mut self) -> Res<Doc> {
        let mut v = vec![self.expect("<")?];
        let mut first = true;
        while !self.is(">") {
            if !first {
                v.push(self.expect(",")?);
                if self.is(">") {
                    break;
                }
                v.push(space());
            }
            first = false;
            if self.kind(0) == Kind::Lifetime {
                v.push(self.tok());
            } else if self.is_ident(0) && self.at(1) == "=" {
                v.push(self.tok());
                v.push(space());
                v.push(self.tok());
                v.push(space());
                v.push(self.ty()?);
            } else if self.kind(0) == Kind::Literal || self.is("{") || self.is("-") {
                v.push(self.primary(R::default())?.doc());
            } else {
                v.push(self.ty()?);
            }
        }
        v.push(self.tok());
        Ok(concat(v))
    }

    fn pat(&mut self) -> Res<Doc> {
        let mut v = Vec::new();
        if self.is("|") {
            v.push(self.tok());
            v.push(space());
        }
        v.push(self.pat_no_top()?);
        if !self.is("|") {
            return Ok(concat(v));
        }
        while self.is("|") {
            v.push(line());
            v.push(self.tok());
            v.push(space());
            v.push(self.pat_no_top()?);
        }
        Ok(group(concat(v)))
    }

    fn pat_no_top(&mut self) -> Res<Doc> {
        self.enter()?;
        let r = self.pat_inner();
        self.leave();
        r
    }

    fn pat_inner(&mut self) -> Res<Doc> {
        let first = match self.peek() {
            "&" | "&&" => {
                let mut v = vec![self.tok()];
                if self.is("mut") {
                    v.push(self.tok());
                    v.push(space());
                }
                v.push(self.pat_no_top()?);
                return Ok(concat(v));
            }
            "(" if self.kind(0) == Kind::Punct => {
                let l = self.list("(", ")", true, |p| p.pat())?;
                let tr = tuple_tr(&l);
                return Ok(layout(l, false, usize::MAX, tr, false));
            }
            "[" if self.kind(0) == Kind::Punct => {
                let l = self.list("[", "]", false, |p| p.pat())?;
                return Ok(layout(l, false, usize::MAX, Tr::Vertical, false));
            }
            ".." | "..=" if self.kind(0) == Kind::Punct => {
                let d = self.tok();
                if self.kind(0) == Kind::Literal || self.is("-") {
                    return Ok(concat(vec![d, self.pat_atom()?]));
                }
                return Ok(d);
            }
            "ref" | "mut" | "box"
                if self.kind(0) == Kind::Ident && (self.is_ident(1) || self.at(1) == "mut") =>
            {
                let mut v = vec![self.tok(), space()];
                if self.is("mut") {
                    v.push(self.tok());
                    v.push(space());
                }
                v.push(self.pat_no_top()?);
                return Ok(concat(v));
            }
            _ => self.pat_atom()?,
        };
        if self.is("@") {
            let at = self.tok();
            return Ok(concat(vec![
                first,
                space(),
                at,
                space(),
                self.pat_no_top()?,
            ]));
        }
        if matches!(self.peek(), "..=" | "..") && self.kind(0) == Kind::Punct {
            let op = self.tok();
            if self.kind(0) == Kind::Literal || self.is("-") || self.is_ident(0) {
                return Ok(concat(vec![first, op, self.pat_atom()?]));
            }
            return Ok(concat(vec![first, op]));
        }
        Ok(first)
    }

    fn pat_atom(&mut self) -> Res<Doc> {
        if self.is("-") {
            let m = self.tok();
            if self.kind(0) != Kind::Literal {
                return self.expected("a literal");
            }
            return Ok(concat(vec![m, self.tok()]));
        }
        if self.kind(0) == Kind::Literal {
            return Ok(self.tok());
        }
        if self.kind(0) != Kind::Ident {
            return self.expected("a pattern");
        }
        let path = self.expr_path()?;
        if self.is("(") {
            let l = self.list("(", ")", false, |p| p.pat())?;
            return Ok(concat(vec![
                path,
                layout(l, false, usize::MAX, Tr::Vertical, false),
            ]));
        }
        if self.is("{") {
            let mut rest = false;
            let l = self.list("{", "}", false, |p| {
                rest = p.is("..");
                p.field_pat()
            })?;
            let tr = if rest { Tr::Never } else { Tr::Vertical };
            return Ok(concat(vec![path, space(), layout(l, true, 18, tr, false)]));
        }
        if self.is("!") {
            let bang = self.tok();
            return Ok(concat(vec![path, bang, self.token_tree()?]));
        }
        Ok(path)
    }

    fn field_pat(&mut self) -> Res<Doc> {
        if self.is("..") {
            return Ok(self.tok());
        }
        if (self.is_ident(0) || self.kind(0) == Kind::Literal) && self.at(1) == ":" {
            let n = self.tok();
            let c = self.tok();
            return Ok(concat(vec![n, c, space(), self.pat()?]));
        }
        self.pat_no_top()
    }

    fn expr_path(&mut self) -> Res<Doc> {
        let mut v = Vec::new();
        if self.is("::") {
            v.push(self.tok());
        }
        if self.is("<") {
            v.push(self.tok());
            v.push(self.ty()?);
            if self.is("as") {
                v.push(space());
                v.push(self.tok());
                v.push(space());
                v.push(self.ty()?);
            }
            v.push(self.expect(">")?);
            v.push(self.expect("::")?);
        }
        v.push(self.ident()?);
        while self.is("::") {
            v.push(self.tok());
            if self.is("<") {
                v.push(self.generic_args()?);
            } else {
                v.push(self.ident()?);
            }
        }
        Ok(concat(v))
    }

    fn block_doc(&mut self) -> Res<Doc> {
        let b = self.block()?;
        Ok(block_layout(b))
    }

    fn block(&mut self) -> Res<Block> {
        self.enter()?;
        let r = self.block_inner();
        self.leave();
        r
    }

    fn block_inner(&mut self) -> Res<Block> {
        let open = self.expect("{")?;
        let mut stmts = Vec::new();
        let mut tail = None;
        let mut only_tail = true;
        while !self.is("}") {
            if self.eof() {
                return self.expected("`}`");
            }
            let blank = self.blank_before(self.pos);
            let lead = self.hoist();
            let (doc, is_tail) = self.stmt()?;
            let doc = match lead {
                Some(l) => concat(vec![l, doc]),
                None => doc,
            };
            if is_tail && stmts.is_empty() {
                tail = Some(doc.clone());
            } else {
                only_tail = false;
            }
            stmts.push((doc, blank));
        }
        if !only_tail {
            tail = None;
        }
        let (close_lead, close) = self.close_tok();
        Ok(Block {
            open,
            stmts,
            tail,
            close_lead,
            close,
        })
    }

    fn stmt(&mut self) -> Res<(Doc, bool)> {
        let start = self.pos;
        if self.is(";") {
            return Ok((self.tok(), false));
        }
        if self.starts_item() {
            let attrs_only = self.is("#") && {
                let save = self.pos;
                let r = self.attrs();
                let item = r.is_ok() && self.starts_item();
                self.restore(save);
                !item
            };
            if !attrs_only {
                return Ok((self.item()?.doc, false));
            }
        }
        let (attrs, skip) = self.attrs()?;
        if self.is("let") {
            let d = self.let_stmt()?;
            if skip {
                return Ok((self.verbatim(start), false));
            }
            return Ok((Self::with_attrs(attrs, d), false));
        }
        let e = self.expr_stmt()?;
        let mut is_tail = false;
        let (d, t) = split_trailing(&e.doc());
        let t = t.unwrap_or_else(nil);
        let doc = if self.is(";") {
            if e.jump && self.at(1) == "}" {
                let c = self.drop_tok();
                concat(vec![d, text(";"), t, c])
            } else {
                concat(vec![d, self.tok(), t])
            }
        } else if self.is("}") {
            is_tail = !e.jump;
            let semi = if e.jump { text(";") } else { nil() };
            concat(vec![d, semi, t])
        } else if e.block_like {
            concat(vec![d, t])
        } else {
            return self.expected("`;`");
        };
        if skip {
            return Ok((self.verbatim(start), false));
        }
        Ok((Self::with_attrs(attrs, doc), is_tail && !skip))
    }

    fn let_stmt(&mut self) -> Res<Doc> {
        let mut v = vec![self.tok(), space(), self.pat()?];
        if self.is(":") {
            v.push(self.tok());
            v.push(space());
            v.push(self.ty()?);
        }
        if self.is("=") {
            v.push(space());
            v.push(self.tok());
            let e = self.expr(R::default())?;
            v.push(rhs(&e));
            if self.is("else") {
                v.push(space());
                v.push(self.tok());
                v.push(space());
                v.push(self.block_doc()?);
            }
        }
        v.push(self.expect(";")?);
        Ok(concat(v))
    }

    fn starts_block_like(&self) -> bool {
        match self.peek() {
            "{" | "if" | "match" | "loop" | "while" | "for" => self.kind(0) != Kind::Literal,
            "unsafe" => self.at(1) == "{",
            _ => self.kind(0) == Kind::Lifetime && self.at(1) == ":",
        }
    }

    fn expr_stmt(&mut self) -> Res<E> {
        if !self.starts_block_like() {
            return self.expr(R::default());
        }
        self.enter()?;
        let e = self.block_like(true);
        self.leave();
        let e = e?;
        if self.is(".") || self.is("?") {
            let e = self.postfix(e)?;
            return self.infix(e, 0, R::default());
        }
        Ok(e)
    }

    fn block_like(&mut self, stmt: bool) -> Res<E> {
        let mut e = match self.peek() {
            "{" => {
                let b = self.block()?;
                E::new(
                    if stmt {
                        block_layout(b)
                    } else {
                        closure_block(b)
                    },
                    Ov::Block,
                )
            }
            "unsafe" => {
                let u = self.tok();
                let b = self.block()?;
                let b = if stmt {
                    block_layout(b)
                } else {
                    closure_block(b)
                };
                E::new(concat(vec![u, space(), b]), Ov::Block)
            }
            "if" => self.if_expr(stmt)?,
            "match" => self.match_expr()?,
            "loop" => {
                let l = self.tok();
                E::new(concat(vec![l, space(), self.block_doc()?]), Ov::Control)
            }
            "while" => {
                let w = self.tok();
                let (cond, chain, _) = self.cond()?;
                let b = self.block()?;
                E::new(control_head(w, cond, chain, b), Ov::Control)
            }
            "for" => {
                let f = self.tok();
                let pat = self.pat()?;
                let i = self.expect("in")?;
                let e = self.expr(R { no_struct: true })?;
                let head = concat(vec![f, space(), pat, space(), i]);
                let (cd, chain) = cond_parts(&e);
                let b = self.block()?;
                E::new(control_head(head, cd, chain, b), Ov::Control)
            }
            _ => {
                let label = self.tok();
                let colon = self.expect(":")?;
                let inner = self.block_like(stmt)?;
                E::new(concat(vec![label, colon, space(), inner.doc]), Ov::Control)
            }
        };
        e.block_like = true;
        Ok(e)
    }

    fn cond(&mut self) -> Res<(Doc, bool, Doc)> {
        if self.is("let") {
            let l = self.tok();
            let p = self.pat()?;
            let eq = self.expect("=")?;
            let e = self.expr(R { no_struct: true })?;
            let d = concat(vec![l, space(), p, space(), eq, space(), e.doc()]);
            return Ok((d.clone(), false, d));
        }
        let e = self.expr(R { no_struct: true })?;
        let (d, chain) = cond_parts(&e);
        Ok((d, chain, e.doc()))
    }

    fn if_expr(&mut self, stmt: bool) -> Res<E> {
        let kw = self.tok();
        let (cond, chain, flat_cond) = self.cond()?;
        let then = self.block()?;
        if !self.is("else") {
            return Ok(E::new(control_head(kw, cond, chain, then), Ov::Control));
        }
        let els = self.tok();
        if self.is("if") {
            let rest = self.if_expr(true)?;
            let head = control_head(kw, cond, chain, then);
            return Ok(E::new(
                concat(vec![head, space(), els, space(), rest.doc]),
                Ov::Control,
            ));
        }
        let other = self.block()?;
        let simple = !stmt
            && then.tail.is_some()
            && other.tail.is_some()
            && then.close_lead.is_none()
            && other.close_lead.is_none();
        if simple {
            let a = then.tail.clone().unwrap();
            let b = other.tail.clone().unwrap();
            let d = concat(vec![
                kw,
                space(),
                flat_cond,
                space(),
                then.open,
                indent(concat(vec![line(), a])),
                line(),
                then.close,
                space(),
                els,
                space(),
                other.open,
                indent(concat(vec![line(), b])),
                line(),
                other.close,
            ]);
            return Ok(E::new(group_limit(d, 50), Ov::Control));
        }
        let head = control_head(kw, cond, chain, then);
        Ok(E::new(
            concat(vec![head, space(), els, space(), block_layout(other)]),
            Ov::Control,
        ))
    }

    fn match_expr(&mut self) -> Res<E> {
        let kw = self.tok();
        let e = self.expr(R { no_struct: true })?;
        let (scrut, chain) = cond_parts(&e);
        let (open, open_trail) = split_trailing(&self.expect("{")?);
        let mut arms = Vec::new();
        while !self.is("}") {
            if self.eof() {
                return self.expected("`}`");
            }
            let blank = self.blank_before(self.pos);
            let lead = self.hoist();
            let arm = self.arm()?;
            arms.push((concat(vec![lead.unwrap_or_else(nil), arm]), blank));
        }
        let (lead, close) = self.close_tok();
        let head = if chain {
            concat(vec![kw, group(concat(vec![space(), scrut, line(), open]))])
        } else {
            concat(vec![kw, space(), scrut, space(), open])
        };
        let body = seq_rest(arms, lead, close);
        Ok(E::new(
            concat(vec![head, open_trail.unwrap_or_else(nil), body]),
            Ov::Control,
        ))
    }

    fn arm(&mut self) -> Res<Doc> {
        let (attrs, _) = self.attrs()?;
        let mut v = vec![self.pat()?];
        if self.is("if") {
            v.push(space());
            v.push(self.tok());
            v.push(space());
            v.push(self.expr(R::default())?.doc());
        }
        v.push(space());
        v.push(self.expect("=>")?);
        let plain_block = self.is("{");
        let body = if plain_block {
            let mut e = E::new(self.block_doc()?, Ov::Block);
            e.block_like = true;
            if self.is(".") || self.is("?") {
                let e = self.postfix(e)?;
                self.infix(e, 0, R::default())?
            } else {
                e
            }
        } else if self.starts_block_like() {
            let e = self.block_like(false)?;
            if self.is(".") || self.is("?") {
                let e = self.postfix(e)?;
                self.infix(e, 0, R::default())?
            } else {
                e
            }
        } else {
            self.expr(R::default())?
        };
        let is_block = plain_block && body.block_like;
        let comma = if self.is(",") {
            self.drop_tok()
        } else if self.is("}") || is_block || body.block_like {
            nil()
        } else {
            return self.expected("`,` or `}`");
        };
        let bd = body.doc();
        if is_block {
            v.push(space());
            v.push(bd);
            v.push(comma);
        } else {
            let (bd, t) = split_trailing(&bd);
            if body.hugs() || body.jump {
                v.push(space());
                v.push(bd);
            } else {
                v.push(group(indent(concat(vec![line(), bd]))));
            }
            v.push(text(","));
            if let Some(t) = t {
                v.push(t);
            }
            v.push(comma);
        }
        Ok(Self::with_attrs(attrs, concat(v)))
    }

    fn expr(&mut self, r: R) -> Res<E> {
        self.enter()?;
        let e = self.expr_bp(0, r);
        self.leave();
        e
    }

    fn can_start_expr(&self) -> bool {
        if self.eof() {
            return false;
        }
        match self.kind(0) {
            Kind::Literal | Kind::Lifetime => true,
            Kind::Ident => !matches!(self.peek(), "as" | "else" | "in"),
            _ => matches!(
                self.peek(),
                "(" | "["
                    | "{"
                    | "|"
                    | "||"
                    | "-"
                    | "!"
                    | "*"
                    | "&"
                    | "&&"
                    | ".."
                    | "..="
                    | "<"
                    | "::"
            ),
        }
    }

    fn expr_bp(&mut self, min: u8, r: R) -> Res<E> {
        let lhs = self.prefix(r)?;
        self.infix(lhs, min, r)
    }

    fn prefix(&mut self, r: R) -> Res<E> {
        match self.peek() {
            "return" | "break" | "continue" | "yield" if self.kind(0) == Kind::Ident => {
                let mut v = vec![self.tok()];
                if self.kind(0) == Kind::Lifetime {
                    v.push(space());
                    v.push(self.tok());
                }
                if self.can_start_expr() && !(self.is("{") && r.no_struct) {
                    v.push(space());
                    v.push(self.expr(r)?.doc());
                }
                let mut e = E::new(concat(v), Ov::Other);
                e.jump = true;
                Ok(e)
            }
            "move" | "|" | "||" if self.kind(0) != Kind::Literal => self.closure(r),
            "-" | "!" | "*" | "&" | "&&" if self.kind(0) == Kind::Punct => {
                let amp = matches!(self.peek(), "&" | "&&");
                let mut v = vec![self.tok()];
                if amp && self.is("mut") {
                    v.push(self.tok());
                    v.push(space());
                }
                self.enter()?;
                let inner = self.expr_bp(14, r);
                self.leave();
                let inner = inner?;
                v.push(inner.doc());
                let ov = inner.ov;
                Ok(E::new(concat(v), ov))
            }
            ".." | "..=" if self.kind(0) == Kind::Punct => {
                let mut v = vec![self.tok()];
                if self.can_start_expr() && !(self.is("{") && r.no_struct) {
                    v.push(self.expr_bp(3, r)?.doc());
                }
                Ok(E::new(concat(v), Ov::Other))
            }
            _ => {
                let p = self.primary(r)?;
                self.postfix(p)
            }
        }
    }

    fn closure(&mut self, r: R) -> Res<E> {
        let mut v = Vec::new();
        if self.is("move") {
            v.push(self.tok());
            v.push(space());
        }
        if self.is("||") {
            v.push(self.tok());
        } else {
            v.push(self.expect("|")?);
            let mut first = true;
            while !self.is("|") {
                if !first {
                    v.push(self.expect(",")?);
                    if self.is("|") {
                        break;
                    }
                    v.push(space());
                }
                first = false;
                v.push(self.pat_no_top()?);
                if self.is(":") {
                    v.push(self.tok());
                    v.push(space());
                    v.push(self.ty()?);
                }
            }
            v.push(self.tok());
        }
        v.push(space());
        if self.is("->") {
            v.push(self.tok());
            v.push(space());
            v.push(self.ty()?);
            v.push(space());
        }
        if self.is("{") {
            let b = self.block()?;
            v.push(closure_block(b));
            return Ok(E::new(concat(v), Ov::Closure));
        }
        let body = self.expr(r)?;
        v.push(body.doc());
        Ok(E::new(concat(v), Ov::Closure))
    }

    fn infix_op(&self) -> Option<(u8, u8, usize)> {
        if self.kind(0) != Kind::Punct && self.peek() != "as" {
            return None;
        }
        let t = self.peek();
        if t == ">" {
            let tok = self.lx.tokens[self.pos];
            if tok.joint && self.at(1) == ">" {
                let t2 = self.lx.tokens[self.pos + 1];
                if t2.joint && self.at(2) == "=" {
                    return Some((1, 1, 3));
                }
                return Some((9, 10, 2));
            }
            if tok.joint && self.at(1) == "=" {
                return Some((5, 6, 2));
            }
            return Some((5, 6, 1));
        }
        let (l, r) = match t {
            "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "^=" | "&=" | "|=" | "<<=" => (1, 1),
            ".." | "..=" => (2, 3),
            "||" => (3, 4),
            "&&" => (4, 5),
            "==" | "!=" | "<" | "<=" => (5, 6),
            "|" => (6, 7),
            "^" => (7, 8),
            "&" => (8, 9),
            "<<" => (9, 10),
            "+" | "-" => (10, 11),
            "*" | "/" | "%" => (11, 12),
            "as" => (12, 13),
            _ => return None,
        };
        Some((l, r, 1))
    }

    fn op_doc(&mut self, n: usize) -> Doc {
        if n == 1 {
            return self.tok();
        }
        let a = self.pos;
        let mut v = Vec::new();
        for _ in 0..n {
            let i = self.bump();
            v.push(self.tok_doc(i, text(self.text(i))));
        }
        self.norm.push(Entry::Op(a, a + n - 1));
        concat(v)
    }

    fn infix(&mut self, mut lhs: E, min: u8, r: R) -> Res<E> {
        while let Some((l, rb, n)) = self.infix_op() {
            if l < min {
                break;
            }
            let is_as = self.peek() == "as";
            let is_range = matches!(self.peek(), ".." | "..=");
            let op = self.op_doc(n);
            if is_as {
                let t = self.ty()?;
                let ov = lhs.ov;
                lhs = E::new(concat(vec![lhs.doc(), space(), op, space(), t]), ov);
                continue;
            }
            if is_range {
                let mut v = vec![lhs.doc(), op];
                if self.can_start_expr() && !(self.is("{") && r.no_struct) {
                    v.push(self.expr_bp(rb, r)?.doc());
                }
                lhs = E::new(concat(v), Ov::Other);
                continue;
            }
            self.enter()?;
            let rhs_e = self.expr_bp(rb, r);
            self.leave();
            let rhs_e = rhs_e?;
            if l == 1 {
                lhs = E::new(concat(vec![lhs.doc(), space(), op, rhs(&rhs_e)]), Ov::Other);
                continue;
            }
            let control = rhs_e.ov == Ov::Control || lhs.ov == Ov::Control;
            match lhs.bin {
                Some(mut b) if b.prec == l => {
                    b.control |= control;
                    b.rest.push((op, rhs_e.doc()));
                    lhs = E {
                        doc: nil(),
                        ov: Ov::Other,
                        bin: Some(b),
                        jump: false,
                        block_like: false,
                    };
                }
                _ => {
                    let b = Bin {
                        prec: l,
                        first: lhs.doc(),
                        rest: vec![(op, rhs_e.doc())],
                        control,
                    };
                    lhs = E {
                        doc: nil(),
                        ov: Ov::Other,
                        bin: Some(b),
                        jump: false,
                        block_like: false,
                    };
                }
            }
        }
        Ok(lhs)
    }

    fn primary(&mut self, r: R) -> Res<E> {
        if self.starts_block_like() {
            return self.block_like(false);
        }
        match self.kind(0) {
            Kind::Literal => return Ok(E::new(self.tok(), Ov::Other)),
            Kind::Eof => return self.expected("an expression"),
            _ => {}
        }
        match self.peek() {
            "(" if self.kind(0) == Kind::Punct => {
                let mut kinds = Vec::new();
                let l = self.list("(", ")", true, |p| {
                    let e = p.expr(R::default())?;
                    kinds.push(e.ov);
                    Ok(e.doc())
                })?;
                if l.items.len() == 1 && !l.items[0].has_comma {
                    let it = &l.items[0];
                    let d = concat(vec![
                        l.open.clone(),
                        it.doc.clone(),
                        l.close_lead.clone().unwrap_or_else(nil),
                        l.close.clone(),
                    ]);
                    return Ok(E::new(d, Ov::Other));
                }
                let tr = tuple_tr(&l);
                Ok(E::new(args_layout(l, &kinds, tr), Ov::Call))
            }
            "[" if self.kind(0) == Kind::Punct => self.array(),
            "true" | "false" => Ok(E::new(self.tok(), Ov::Other)),
            _ => {
                if self.kind(0) != Kind::Ident && !self.is("<") && !self.is("::") {
                    return self.expected("an expression");
                }
                if self.is_keyword_start() {
                    return self.expected("an expression");
                }
                let path = self.expr_path()?;
                if self.is("!") && matches!(self.at(1), "(" | "[" | "{") {
                    let bang = self.tok();
                    return self.macro_call(path, bang);
                }
                if self.is("{") && !r.no_struct && self.looks_like_struct() {
                    return self.struct_lit(path);
                }
                Ok(E::new(path, Ov::Other))
            }
        }
    }

    fn is_keyword_start(&self) -> bool {
        let t = self.peek();
        is_keyword(t) && !matches!(t, "self" | "Self" | "crate" | "super")
    }

    fn looks_like_struct(&self) -> bool {
        let a = self.at(1);
        let k = self.kind(1);
        a == "}" && k == Kind::Punct
            || a == ".." && k == Kind::Punct
            || (k == Kind::Ident || k == Kind::Literal)
                && (matches!(self.at(2), ":" | "," | "}") && self.kind(2) == Kind::Punct)
            || a == "#"
    }

    fn struct_lit(&mut self, path: Doc) -> Res<E> {
        let mut rest = false;
        let l = self.list("{", "}", false, |p| {
            if p.is("..") {
                rest = true;
                let d = p.tok();
                if p.is("}") {
                    return Ok(d);
                }
                return Ok(concat(vec![d, p.expr(R::default())?.doc()]));
            }
            let (attrs, _) = p.attrs()?;
            let mut v = Vec::new();
            for a in attrs {
                v.push(a);
                v.push(space());
            }
            v.push(p.tok());
            if p.is(":") {
                v.push(p.tok());
                v.push(space());
                v.push(p.expr(R::default())?.doc());
            }
            Ok(concat(v))
        })?;
        let tr = if rest { Tr::Never } else { Tr::Vertical };
        Ok(E::new(
            concat(vec![path, space(), layout(l, true, 18, tr, false)]),
            Ov::Delimited,
        ))
    }

    fn array(&mut self) -> Res<E> {
        if self.repeat_array() {
            let o = self.tok();
            let x = self.expr(R::default())?;
            let semi = self.expect(";")?;
            let n = self.expr(R::default())?;
            let c = self.expect("]")?;
            return Ok(E::new(
                concat(vec![o, x.doc(), semi, space(), n.doc(), c]),
                Ov::Delimited,
            ));
        }
        let mut short = true;
        let l = self.list("[", "]", false, |p| {
            let s = p.pos;
            let e = p.expr(R::default())?;
            let lit = (s..p.pos).all(|i| p.lx.tokens[i].kind == Kind::Literal || p.text(i) == "-")
                && p.pos - s <= 2;
            let w = p.src[p.lx.tokens[s].start..p.lx.tokens[p.pos - 1].end]
                .chars()
                .count();
            short &= lit && w <= 10;
            Ok(e.doc())
        })?;
        Ok(E::new(array_layout(l, short), Ov::Delimited))
    }

    fn repeat_array(&self) -> bool {
        let mut depth = 0usize;
        let mut i = self.pos;
        while i < self.lx.tokens.len() - 1 {
            let t = self.text(i);
            if self.lx.tokens[i].kind == Kind::Punct {
                match t {
                    "(" | "[" | "{" => depth += 1,
                    ")" | "]" | "}" => {
                        depth -= 1;
                        if depth == 0 {
                            return false;
                        }
                    }
                    ";" if depth == 1 => return true,
                    "," if depth == 1 => return false,
                    _ => {}
                }
            }
            i += 1;
        }
        false
    }

    fn macro_call(&mut self, path: Doc, bang: Doc) -> Res<E> {
        let head = concat(vec![path, bang]);
        let offset = format_offset(self.text(self.pos - 2));
        let save = self.pos;
        let delim = self.peek();
        let attempt = match delim {
            "(" => {
                let mut kinds = Vec::new();
                let mut simple = Vec::new();
                self.list("(", ")", false, |p| {
                    let start = p.pos;
                    let e = p.expr(R::default())?;
                    simple.push((start..p.pos).all(|i| {
                        p.lx.tokens[i].kind != Kind::Punct
                            || matches!(p.text(i), "." | "::" | "&" | "-" | "!" | "*" | "[" | "]")
                    }));
                    kinds.push(e.ov);
                    Ok(e.doc())
                })
                .map(|l| {
                    let d = match offset {
                        Some(k) if l.items.len() > k + 1 && simple[k + 1..].iter().all(|s| *s) => {
                            format_args_layout(l, k)
                        }
                        _ => args_layout(l, &kinds, Tr::Never),
                    };
                    E::new(d, Ov::ParenMacro)
                })
            }
            "[" => self.array().map(|e| E::new(e.doc, Ov::BracketMacro)),
            _ => Err(Error::syntax(0, "")),
        };
        let e = match attempt {
            Ok(e) => e,
            Err(_) => {
                self.restore(save);
                let ov = if delim == "(" {
                    Ov::ParenMacro
                } else {
                    Ov::BracketMacro
                };
                E::new(self.token_tree()?, ov)
            }
        };
        Ok(E::new(concat(vec![head, e.doc]), e.ov))
    }

    fn postfix(&mut self, root: E) -> Res<E> {
        let mut root = root;
        let mut children: Vec<(Doc, bool)> = Vec::new();
        loop {
            match self.peek() {
                "." if self.kind(0) == Kind::Punct => {
                    let dot = self.tok();
                    if self.kind(0) != Kind::Ident && self.kind(0) != Kind::Literal {
                        return self.expected("a field or method name");
                    }
                    let mut v = vec![dot, self.tok()];
                    if self.is("::") && self.at(1) == "<" {
                        v.push(self.tok());
                        v.push(self.generic_args()?);
                    }
                    let call = self.is("(");
                    if call {
                        v.push(self.call_args()?);
                    }
                    children.push((concat(v), call));
                }
                "(" if self.kind(0) == Kind::Punct => {
                    let args = self.call_args()?;
                    match children.last_mut() {
                        Some((d, c)) => {
                            *d = concat(vec![d.clone(), args]);
                            *c = true;
                        }
                        None => root = E::new(concat(vec![root.doc(), args]), Ov::Call),
                    }
                }
                "[" if self.kind(0) == Kind::Punct => {
                    let o = self.tok();
                    let i = self.expr(R::default())?;
                    let c = self.expect("]")?;
                    let idx = concat(vec![o, i.doc(), c]);
                    match children.last_mut() {
                        Some((d, _)) => *d = concat(vec![d.clone(), idx]),
                        None => root = E::new(concat(vec![root.doc(), idx]), Ov::Other),
                    }
                }
                "?" if self.kind(0) == Kind::Punct => {
                    let q = self.tok();
                    match children.last_mut() {
                        Some((d, _)) => *d = concat(vec![d.clone(), q]),
                        None => {
                            let ov = root.ov;
                            root = E::new(concat(vec![root.doc(), q]), ov)
                        }
                    }
                }
                _ => break,
            }
        }
        if children.is_empty() {
            return Ok(root);
        }
        let calls = children.iter().filter(|c| c.1).count();
        let last_call = children.last().unwrap().1;
        let mut rd = root.doc();
        if calls < 2 {
            let mut v = vec![rd];
            v.extend(children.into_iter().map(|c| c.0));
            let ov = if last_call { Ov::Call } else { Ov::Other };
            return Ok(E::new(concat(v), ov));
        }
        while rd.flat().is_some_and(|w| w <= 4) && !children.is_empty() {
            let c = children.remove(0);
            rd = concat(vec![rd, c.0]);
        }
        if children.is_empty() {
            return Ok(E::new(rd, Ov::Call));
        }
        let mut alts = Vec::new();
        let mut flat = vec![rd.clone()];
        flat.extend(children.iter().map(|c| c.0.clone()));
        let flat = concat(flat);
        if flat.flat().is_some_and(|w| w <= 60) {
            alts.push(flat.clone());
        }
        let n = children.len();
        let prefix_flat = rd.flat().unwrap_or(usize::MAX).saturating_add(
            children[..n - 1]
                .iter()
                .map(|c| c.0.flat().unwrap_or(usize::MAX))
                .fold(0usize, |a, b| a.saturating_add(b)),
        );
        if children[n - 1].0.brk() && children[n - 1].1 && prefix_flat <= 60 {
            alts.push(flat);
        }
        let mut v = Vec::new();
        for c in &children {
            v.push(softline());
            v.push(c.0.clone());
        }
        alts.push(force(group(concat(vec![rd, indent(concat(v))]))));
        Ok(E::new(choice(alts), Ov::Call))
    }

    fn call_args(&mut self) -> Res<Doc> {
        let mut kinds = Vec::new();
        let l = self.list("(", ")", false, |p| {
            let e = p.expr(R::default())?;
            kinds.push(e.ov);
            Ok(e.doc())
        })?;
        Ok(args_layout(l, &kinds, Tr::Vertical))
    }
}

fn tuple_tr(l: &List) -> Tr {
    if l.items.len() == 1 && l.items[0].has_comma {
        Tr::Always
    } else {
        Tr::Vertical
    }
}

fn is_primitive(s: &str) -> bool {
    matches!(
        s,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
            | "bool"
            | "char"
            | "str"
    )
}

fn tt_space(prev: &str, cur: &str) -> bool {
    if matches!(prev, "(" | "[" | "{" | "#" | "#!" | "::" | "." | "&" | "$") {
        return false;
    }
    if matches!(cur, ")" | "]" | "}" | "," | ";" | "::" | "." | "?") {
        return false;
    }
    if matches!(cur, "(" | "[") {
        return !(prev
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
            || matches!(prev, ")" | "]" | ">" | "!"));
    }
    if cur == "!"
        && prev
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
    {
        return false;
    }
    true
}

fn rhs(e: &E) -> Doc {
    let d = e.doc();
    if e.hugs() || e.jump {
        return concat(vec![space(), d]);
    }
    let same_line = concat(vec![space(), d.clone()]);
    if d.brk() {
        return same_line;
    }
    let next_line = indent(concat(vec![next_line(), d]));
    choice(vec![same_line.clone(), next_line, same_line])
}

fn cond_parts(e: &E) -> (Doc, bool) {
    match &e.bin {
        Some(b) => (concat(vec![b.first.clone(), indent(bin_rest(b))]), true),
        None => (e.doc(), false),
    }
}

fn control_head(kw: Doc, cond: Doc, chain: bool, b: Block) -> Doc {
    let (open, t) = split_trailing(&b.open);
    let t = t.unwrap_or_else(nil);
    let rest = block_rest(b);
    if chain {
        let head = group(concat(vec![space(), cond, line(), open]));
        concat(vec![kw, head, t, rest])
    } else {
        concat(vec![kw, space(), cond, space(), open, t, rest])
    }
}

fn closure_block(b: Block) -> Doc {
    match b.tail.clone() {
        Some(tail) if b.close_lead.is_none() => {
            let (open, t) = split_trailing(&b.open);
            let inner = concat(vec![t.unwrap_or_else(nil), line(), tail]);
            group(concat(vec![open, indent(inner), line(), b.close]))
        }
        _ => block_layout(b),
    }
}

fn block_layout(b: Block) -> Doc {
    let open = b.open.clone();
    concat(vec![open, block_rest(b)])
}

fn block_rest(b: Block) -> Doc {
    seq_rest(b.stmts, b.close_lead, b.close)
}

fn seq_block(open: Doc, items: Vec<(Doc, bool)>, lead: Option<(Doc, bool)>, close: Doc) -> Doc {
    concat(vec![open, seq_rest(items, lead, close)])
}

fn seq_rest(items: Vec<(Doc, bool)>, lead: Option<(Doc, bool)>, close: Doc) -> Doc {
    if items.is_empty() && lead.is_none() {
        return close;
    }
    let had_items = !items.is_empty();
    let mut v = Vec::new();
    for (k, (d, blank)) in items.into_iter().enumerate() {
        v.push(if k > 0 && blank {
            blankline()
        } else {
            hardline()
        });
        v.push(d);
    }
    if let Some((l, blank)) = lead {
        if blank && had_items {
            v.push(blankline());
        }
        v.push(l);
    }
    concat(vec![indent(concat(v)), hardline(), close])
}

fn items_doc(items: Vec<Item>) -> Doc {
    let mut v = Vec::new();
    for (k, it) in items.into_iter().enumerate() {
        if k > 0 {
            v.push(if it.blank { blankline() } else { hardline() });
        }
        v.push(it.doc);
    }
    concat(v)
}

fn sort_runs(items: Vec<Item>) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    let mut run: Vec<Item> = Vec::new();
    let flush = |run: &mut Vec<Item>, out: &mut Vec<Item>| {
        if run.is_empty() {
            return;
        }
        let blank = run[0].blank;
        run.sort_by(|a, b| {
            let (_, ta, _) = a.sort.as_ref().unwrap();
            let (_, tb, _) = b.sort.as_ref().unwrap();
            tree_cmp(ta, tb)
        });
        for (k, it) in run.iter_mut().enumerate() {
            it.blank = k == 0 && blank;
        }
        out.append(run);
    };
    for it in items {
        let kind = it.sort.as_ref().map(|s| s.0);
        let continues = kind.is_some()
            && !it.blank
            && run
                .last()
                .is_some_and(|p: &Item| p.sort.as_ref().map(|s| s.0) == kind);
        if !continues {
            flush(&mut run, &mut out);
        }
        if kind.is_some() {
            run.push(it);
        } else {
            out.push(it);
        }
    }
    flush(&mut run, &mut out);
    out
}

fn layout(l: List, pad: bool, limit: usize, tr: Tr, vertical: bool) -> Doc {
    let n = l.items.len();
    if n == 0 {
        return match l.close_lead {
            Some(cl) => concat(vec![l.open, indent(cl), hardline(), l.close]),
            None => concat(vec![l.open, l.close]),
        };
    }
    let mut inner = Vec::new();
    for k in 0..n {
        let it = &l.items[k];
        let (d, t) = split_trailing(&it.doc);
        inner.push(d);
        if k + 1 < n {
            inner.push(text(","));
        } else {
            match tr {
                Tr::Vertical => inner.push(if_break(text(","), nil())),
                Tr::Never => {}
                Tr::Always => inner.push(text(",")),
            }
        }
        if let Some(t) = t {
            inner.push(t);
        }
        if let Some(c) = &it.comma {
            inner.push(c.clone());
        }
        if k + 1 < n {
            if vertical && l.items[k + 1].blank {
                inner.push(blankline());
            } else {
                inner.push(line());
            }
        }
    }
    if let Some(cl) = l.close_lead {
        inner.push(cl);
    }
    let sep = if pad { line() } else { softline() };
    let ow = l.open.flat().unwrap_or(0) + l.close.flat().unwrap_or(0) + if pad { 2 } else { 0 };
    let d = concat(vec![
        l.open,
        indent(concat(vec![sep.clone(), concat(inner)])),
        sep,
        l.close,
    ]);
    if vertical {
        group_limit(d, 0)
    } else {
        group_limit(d, limit.saturating_add(ow))
    }
}

fn overflowable(ov: Ov, n: usize) -> bool {
    match ov {
        Ov::Closure | Ov::Block | Ov::ParenMacro => true,
        Ov::Control | Ov::Delimited | Ov::BracketMacro | Ov::Call => n == 1,
        Ov::Other => false,
    }
}

fn args_layout(l: List, kinds: &[Ov], tr: Tr) -> Doc {
    let n = l.items.len();
    let hug_ok = n > 0
        && overflowable(kinds[n - 1], n)
        && !l.has_comments()
        && l.items[..n - 1].iter().all(|i| !i.doc.brk());
    if !hug_ok {
        let limit = if n == 1 { usize::MAX } else { 60 };
        return layout(l, false, limit, tr, false);
    }
    let last = l.items[n - 1].doc.clone();
    let mut heads = Vec::new();
    for it in &l.items[..n - 1] {
        heads.push(it.doc.clone());
        heads.push(text(","));
        heads.push(space());
    }
    let hug = |last: Doc| {
        let mut v = vec![l.open.clone()];
        v.extend(heads.iter().cloned());
        v.push(last);
        if tr == Tr::Always {
            v.push(text(","));
        }
        v.push(l.close.clone());
        concat(v)
    };
    let mut alts = Vec::new();
    let flat = hug(last.clone());
    if last.brk() {
        alts.push(flat);
    } else {
        if flat.flat().is_some_and(|w| w <= 62) {
            alts.push(flat);
        }
        alts.push(hug(force(last.clone())));
    }
    alts.push(force(layout(l, false, 60, tr, false)));
    choice(alts)
}

fn format_offset(name: &str) -> Option<usize> {
    match name {
        "format" | "print" | "println" | "eprint" | "eprintln" | "panic" | "unreachable"
        | "todo" => Some(0),
        "assert" | "debug_assert" | "write" | "writeln" => Some(1),
        "assert_eq" | "assert_ne" | "debug_assert_eq" | "debug_assert_ne" => Some(2),
        _ => None,
    }
}

fn format_args_layout(l: List, k: usize) -> Doc {
    if l.has_comments() {
        return layout(l, false, 60, Tr::Never, false);
    }
    let n = l.items.len();
    let mut lead = Vec::new();
    for it in &l.items[..=k] {
        lead.push(it.doc.clone());
        lead.push(text(","));
        lead.push(line());
    }
    let mut parts = Vec::new();
    for (j, it) in l.items[k + 1..].iter().enumerate() {
        if k + 1 + j + 1 < n {
            parts.push(concat(vec![it.doc.clone(), text(",")]));
            parts.push(line());
        } else {
            parts.push(it.doc.clone());
        }
    }
    lead.push(fill(parts));
    let ow = l.open.flat().unwrap_or(0) + l.close.flat().unwrap_or(0);
    let d = concat(vec![
        l.open,
        indent(concat(vec![softline(), concat(lead)])),
        softline(),
        l.close,
    ]);
    group_limit(d, 60 + ow)
}

fn array_layout(l: List, short: bool) -> Doc {
    let n = l.items.len();
    if !short || n == 0 || l.has_comments() {
        return layout(l, false, 60, Tr::Vertical, false);
    }
    let mut parts = Vec::new();
    for (k, it) in l.items.iter().enumerate() {
        if k + 1 < n {
            parts.push(concat(vec![it.doc.clone(), text(",")]));
            parts.push(line());
        } else {
            parts.push(it.doc.clone());
        }
    }
    let ow = l.open.flat().unwrap_or(0) + l.close.flat().unwrap_or(0);
    let d = concat(vec![
        l.open,
        indent(concat(vec![
            softline(),
            fill(parts),
            if_break(text(","), nil()),
        ])),
        softline(),
        l.close,
    ]);
    group_limit(d, 60 + ow)
}

fn use_key(t: &UseTree) -> String {
    format!("{}{}", t.vis, segs_key(&t.segs))
}

fn segs_key(segs: &[Seg]) -> String {
    let parts: Vec<String> = segs
        .iter()
        .map(|s| {
            let base = match &s.kind {
                SegKind::Name(n) => n.clone(),
                SegKind::Glob => "*".to_string(),
                SegKind::List(l) => {
                    format!(
                        "{{{}}}",
                        l.iter()
                            .map(|t| segs_key(&t.segs))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            };
            match &s.alias {
                Some(a) => format!("{base} as {a}"),
                None => base,
            }
        })
        .collect();
    parts.join("::")
}

fn seg_rank(s: &Seg) -> u8 {
    match &s.kind {
        SegKind::Name(n) if n == "self" => 0,
        SegKind::Name(n) if n == "super" => 1,
        SegKind::Name(n) if n == "crate" => 2,
        SegKind::Name(_) => 3,
        SegKind::Glob => 4,
        SegKind::List(_) => 5,
    }
}

fn is_upper_snake(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_uppercase() || c == '_' || c.is_numeric())
}

fn seg_cmp(a: &Seg, b: &Seg) -> Ordering {
    let (ra, rb) = (seg_rank(a), seg_rank(b));
    if ra != rb {
        return ra.cmp(&rb);
    }
    match (&a.kind, &b.kind) {
        (SegKind::Name(x), SegKind::Name(y)) => {
            let x = x.trim_start_matches("r#");
            let y = y.trim_start_matches("r#");
            let xu = x.starts_with(char::is_uppercase);
            let yu = y.starts_with(char::is_uppercase);
            if xu && y.starts_with(char::is_lowercase) {
                return Ordering::Greater;
            }
            if x.starts_with(char::is_lowercase) && yu {
                return Ordering::Less;
            }
            match (is_upper_snake(x), is_upper_snake(y)) {
                (true, false) => return Ordering::Greater,
                (false, true) => return Ordering::Less,
                _ => {}
            }
            x.cmp(y).then_with(|| a.alias.cmp(&b.alias))
        }
        (SegKind::List(x), SegKind::List(y)) => {
            for (p, q) in x.iter().zip(y.iter()) {
                let o = tree_cmp(p, q);
                if o != Ordering::Equal {
                    return o;
                }
            }
            x.len().cmp(&y.len())
        }
        _ => Ordering::Equal,
    }
}

fn tree_cmp(a: &UseTree, b: &UseTree) -> Ordering {
    for (x, y) in a.segs.iter().zip(b.segs.iter()) {
        let o = seg_cmp(x, y);
        if o != Ordering::Equal {
            return o;
        }
    }
    a.segs.len().cmp(&b.segs.len())
}
