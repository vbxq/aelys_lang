use std::rc::Rc;

const WIDTH: usize = 100;
const INDENT: usize = 4;

enum Node {
    Text(Rc<str>),
    Space,
    Concat(Vec<Doc>),
    Group(Doc, usize),
    Indent(Doc),
    Line,
    SoftLine,
    HardLine,
    FreshLine,
    BlankLine,
    BreakAfter,
    IfBreak(Doc, Doc),
    Fill(Vec<Doc>),
    Choice(Vec<Doc>),
    Trail(Doc),
    NextLine,
}

struct Inner {
    node: Node,
    flat: Option<usize>,
    brk: bool,
    multiline: bool,
    open_flat: Option<usize>,
    open_brk: bool,
}

#[derive(Clone)]
pub struct Doc(Rc<Inner>);

fn width(s: &str) -> usize {
    s.chars().count()
}

impl Doc {
    fn new(node: Node) -> Doc {
        let (flat, brk, multiline) = match &node {
            Node::Text(s) => match s.find('\n') {
                Some(i) => (Some(width(&s[..i])), false, true),
                None => (Some(width(s)), false, false),
            },
            Node::Space | Node::Line => (Some(1), false, false),
            Node::SoftLine => (Some(0), false, false),
            Node::HardLine
            | Node::FreshLine
            | Node::BlankLine
            | Node::BreakAfter
            | Node::NextLine => (None, true, false),
            Node::Concat(v) | Node::Fill(v) => {
                let flat = v.iter().try_fold(0, |a, d| Some(a + d.0.flat?));
                (
                    flat,
                    v.iter().any(|d| d.0.brk),
                    v.iter().any(|d| d.0.multiline),
                )
            }
            Node::Group(d, _) => (d.0.flat, d.0.brk, d.0.multiline),
            Node::Indent(d) | Node::Trail(d) => (d.0.flat, d.0.brk, d.0.multiline),
            Node::IfBreak(_, f) => (f.0.flat, f.0.brk, f.0.multiline),
            Node::Choice(v) => (v[0].0.flat, v.iter().all(|d| d.0.brk), v[0].0.multiline),
        };
        let (open_flat, open_brk) = match &node {
            Node::Trail(_) => (Some(0), false),
            Node::Concat(v) => match v.split_last() {
                Some((last, init)) => (
                    last.0
                        .open_flat
                        .and_then(|l| init.iter().try_fold(l, |a, d| Some(a + d.0.flat?))),
                    init.iter().any(|d| d.0.brk) || last.0.open_brk,
                ),
                None => (Some(0), false),
            },
            Node::Indent(d) => (d.0.open_flat, d.0.open_brk),
            Node::Group(d, limit) => {
                let breaks = d.0.open_brk || d.0.open_flat.is_none_or(|w| w > *limit);
                (d.0.open_flat, breaks)
            }
            Node::Choice(v) => (v[0].0.open_flat, v.iter().all(|d| d.0.open_brk)),
            _ => (flat, brk),
        };
        let brk = match &node {
            Node::Group(..) => brk || open_brk,
            _ => brk,
        };
        Doc(Rc::new(Inner {
            node,
            flat,
            brk,
            multiline,
            open_flat,
            open_brk,
        }))
    }

    fn breaks(&self) -> bool {
        match &self.0.node {
            Node::Group(..) => self.0.open_brk,
            _ => self.0.brk,
        }
    }

    pub fn open_flat(&self) -> Option<usize> {
        self.0.open_flat
    }

    pub fn open_brk(&self) -> bool {
        self.0.open_brk
    }
}

pub fn text(s: impl Into<Rc<str>>) -> Doc {
    Doc::new(Node::Text(s.into()))
}

pub fn space() -> Doc {
    Doc::new(Node::Space)
}

pub fn line() -> Doc {
    Doc::new(Node::Line)
}

pub fn softline() -> Doc {
    Doc::new(Node::SoftLine)
}

pub fn hardline() -> Doc {
    Doc::new(Node::HardLine)
}

pub fn next_line() -> Doc {
    Doc::new(Node::NextLine)
}

pub fn freshline() -> Doc {
    Doc::new(Node::FreshLine)
}

pub fn blankline() -> Doc {
    Doc::new(Node::BlankLine)
}

pub fn break_after() -> Doc {
    Doc::new(Node::BreakAfter)
}

pub fn concat(v: Vec<Doc>) -> Doc {
    Doc::new(Node::Concat(v))
}

pub fn nil() -> Doc {
    concat(Vec::new())
}

pub fn indent(d: Doc) -> Doc {
    Doc::new(Node::Indent(d))
}

pub fn group(d: Doc) -> Doc {
    Doc::new(Node::Group(d, usize::MAX))
}

pub fn group_limit(d: Doc, limit: usize) -> Doc {
    Doc::new(Node::Group(d, limit))
}

pub fn force(d: Doc) -> Doc {
    if d.breaks() {
        return d;
    }
    match &d.0.node {
        Node::Group(inner, _) => group_limit(inner.clone(), 0),
        Node::Choice(v) => choice(v[1..].to_vec()),
        Node::Concat(v) => {
            let Some(i) = v.iter().rposition(|x| {
                matches!(
                    x.0.node,
                    Node::Group(..) | Node::Concat(_) | Node::Choice(_)
                )
            }) else {
                return d;
            };
            let mut v = v.clone();
            v[i] = force(v[i].clone());
            concat(v)
        }
        _ => d,
    }
}

pub fn trail(d: Doc) -> Doc {
    Doc::new(Node::Trail(d))
}

pub fn split_trailing(d: &Doc) -> (Doc, Option<Doc>) {
    match &d.0.node {
        Node::Trail(_) => (nil(), Some(d.clone())),
        Node::Concat(v) if !v.is_empty() => {
            let (last, t) = split_trailing(v.last().unwrap());
            if t.is_none() {
                return (d.clone(), None);
            }
            let mut v = v.clone();
            *v.last_mut().unwrap() = last;
            (concat(v), t)
        }
        Node::Indent(inner) => {
            let (d, t) = split_trailing(inner);
            (indent(d), t)
        }
        Node::Group(inner, limit) => {
            let (g, t) = split_trailing(inner);
            if t.is_none() {
                return (d.clone(), None);
            }
            (group_limit(g, *limit), t)
        }
        Node::Choice(v) => {
            let parts: Vec<_> = v.iter().map(split_trailing).collect();
            if parts.iter().any(|(_, t)| t.is_none()) {
                return (d.clone(), None);
            }
            let t = parts[0].1.clone();
            (choice(parts.into_iter().map(|(d, _)| d).collect()), t)
        }
        _ => (d.clone(), None),
    }
}

pub fn if_break(broken: Doc, flat: Doc) -> Doc {
    Doc::new(Node::IfBreak(broken, flat))
}

pub fn fill(v: Vec<Doc>) -> Doc {
    Doc::new(Node::Fill(v))
}

pub fn choice(v: Vec<Doc>) -> Doc {
    if v.len() == 1 {
        return v.into_iter().next().unwrap();
    }
    Doc::new(Node::Choice(v))
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Flat,
    Break,
}

#[derive(Clone, Copy)]
struct Cmd<'d> {
    ind: usize,
    mode: Mode,
    doc: &'d Doc,
    from: usize,
}

impl<'d> Cmd<'d> {
    fn new(ind: usize, mode: Mode, doc: &'d Doc) -> Cmd<'d> {
        Cmd {
            ind,
            mode,
            doc,
            from: 0,
        }
    }
}

struct Printer {
    out: String,
    col: usize,
    protected: usize,
    at_line_start: bool,
    line_indent: usize,
    blank: bool,
    need_newline: bool,
}

impl Printer {
    fn newline(&mut self, ind: usize) {
        while self.out.len() > self.protected && self.out.ends_with(' ') {
            self.out.pop();
        }
        if !self.at_line_start {
            self.out.push('\n');
        }
        self.at_line_start = true;
        self.line_indent = ind;
        self.col = ind;
        self.need_newline = false;
    }

    fn text(&mut self, s: &str, ind: usize) {
        if self.need_newline {
            self.newline(ind);
        }
        if self.at_line_start {
            if self.blank && !self.out.is_empty() {
                self.out.push('\n');
            }
            self.blank = false;
            for _ in 0..self.line_indent {
                self.out.push(' ');
            }
            self.at_line_start = false;
        }
        self.out.push_str(s);
        self.protected = self.out.len();
        match s.rfind('\n') {
            Some(i) => self.col = width(&s[i + 1..]),
            None => self.col += width(s),
        }
    }

    fn space(&mut self) {
        if !self.at_line_start && !self.need_newline {
            self.out.push(' ');
            self.col += 1;
        }
    }
}

fn fits_all<'d>(mut cmds: Vec<Cmd<'d>>, rest: &[Cmd<'d>], mut room: isize) -> bool {
    let mut rest_idx = rest.len();
    while room >= 0 {
        let Some(Cmd {
            ind,
            mode,
            doc,
            from,
        }) = cmds.pop()
        else {
            if rest_idx == 0 {
                return true;
            }
            rest_idx -= 1;
            cmds.push(rest[rest_idx]);
            continue;
        };
        match &doc.0.node {
            Node::Text(_) => {
                if doc.0.multiline {
                    return room >= doc.0.flat.unwrap_or(0) as isize;
                }
                room -= doc.0.flat.unwrap_or(0) as isize;
            }
            Node::Space => room -= 1,
            Node::Concat(v) | Node::Fill(v) => {
                for d in v[from..].iter().rev() {
                    cmds.push(Cmd::new(ind, mode, d));
                }
            }
            Node::Trail(d) if d.open_brk() => return true,
            Node::Trail(_) => {}
            Node::Indent(d) => cmds.push(Cmd::new(ind + INDENT, mode, d)),
            Node::Group(d, _) => {
                let mode = if doc.breaks() { Mode::Break } else { mode };
                cmds.push(Cmd::new(ind, mode, d))
            }
            Node::IfBreak(b, f) => {
                cmds.push(Cmd::new(ind, mode, if mode == Mode::Break { b } else { f }))
            }
            Node::Choice(v) => {
                let pick = if mode == Mode::Break {
                    v.last().unwrap()
                } else {
                    &v[0]
                };
                cmds.push(Cmd::new(ind, mode, pick))
            }
            Node::NextLine => room = WIDTH as isize - ind as isize,
            Node::Line => {
                if mode == Mode::Break {
                    return true;
                }
                room -= 1;
            }
            Node::SoftLine => {
                if mode == Mode::Break {
                    return true;
                }
            }
            Node::HardLine | Node::FreshLine | Node::BlankLine | Node::BreakAfter => return true,
        }
    }
    false
}

pub fn print(doc: &Doc) -> String {
    let mut p = Printer {
        out: String::new(),
        col: 0,
        protected: 0,
        at_line_start: true,
        line_indent: 0,
        blank: false,
        need_newline: false,
    };
    let mut stack = vec![Cmd::new(0, Mode::Break, doc)];
    while let Some(cmd) = stack.pop() {
        let Cmd {
            ind,
            mode,
            doc,
            from,
        } = cmd;
        match &doc.0.node {
            Node::Text(s) => p.text(s, ind),
            Node::Space => p.space(),
            Node::Concat(v) => {
                for d in v.iter().rev() {
                    stack.push(Cmd::new(ind, mode, d));
                }
            }
            Node::Indent(d) => stack.push(Cmd::new(ind + INDENT, mode, d)),
            Node::Trail(d) => stack.push(Cmd::new(ind, mode, d)),
            Node::Group(d, _) => {
                let flat = Cmd::new(ind, Mode::Flat, d);
                let m = if doc.breaks() {
                    Mode::Break
                } else if mode == Mode::Flat || fits_all(vec![flat], &stack, room(&p)) {
                    Mode::Flat
                } else {
                    Mode::Break
                };
                stack.push(Cmd::new(ind, m, d));
            }
            Node::Choice(v) => {
                if mode == Mode::Flat {
                    stack.push(Cmd::new(ind, mode, &v[0]));
                } else {
                    let last = v.len() - 1;
                    let pick = v[..last]
                        .iter()
                        .find(|d| fits_all(vec![Cmd::new(ind, Mode::Flat, d)], &stack, room(&p)));
                    match pick {
                        Some(d) => stack.push(Cmd::new(ind, Mode::Flat, d)),
                        None => stack.push(Cmd::new(ind, Mode::Break, &v[last])),
                    }
                }
            }
            Node::IfBreak(b, f) => {
                stack.push(Cmd::new(ind, mode, if mode == Mode::Break { b } else { f }))
            }
            Node::Line => {
                if mode == Mode::Break || p.need_newline {
                    p.newline(ind);
                } else {
                    p.space();
                }
            }
            Node::SoftLine => {
                if mode == Mode::Break || p.need_newline {
                    p.newline(ind);
                }
            }
            Node::HardLine | Node::NextLine => p.newline(ind),
            Node::FreshLine => {
                if !p.at_line_start {
                    p.newline(ind);
                }
            }
            Node::BlankLine => {
                if !p.at_line_start {
                    p.newline(ind);
                }
                p.blank = true;
            }
            Node::BreakAfter => p.need_newline = true,
            Node::Fill(v) => print_fill(&mut p, &mut stack, ind, mode, doc, v, from),
        }
    }
    let mut out = p.out;
    while out.ends_with(' ') && out.len() > p.protected {
        out.pop();
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn room(p: &Printer) -> isize {
    WIDTH as isize - p.col as isize
}

fn print_fill<'d>(
    p: &mut Printer,
    stack: &mut Vec<Cmd<'d>>,
    ind: usize,
    mode: Mode,
    doc: &'d Doc,
    v: &'d [Doc],
    from: usize,
) {
    let parts = &v[from..];
    if parts.is_empty() {
        return;
    }
    let r = room(p);
    let flat = |d| Cmd::new(ind, Mode::Flat, d);
    let broken = |d| Cmd::new(ind, Mode::Break, d);
    let content_fits = fits_all(vec![flat(&parts[0])], &[], r);
    if parts.len() == 1 {
        stack.push(if content_fits {
            flat(&parts[0])
        } else {
            broken(&parts[0])
        });
        return;
    }
    if parts.len() == 2 {
        if content_fits {
            stack.push(flat(&parts[1]));
            stack.push(flat(&parts[0]));
        } else {
            stack.push(broken(&parts[1]));
            stack.push(broken(&parts[0]));
        }
        return;
    }
    stack.push(Cmd {
        ind,
        mode,
        doc,
        from: from + 2,
    });
    let pair_fits = fits_all(
        vec![flat(&parts[2]), flat(&parts[1]), flat(&parts[0])],
        &[],
        r,
    );
    if pair_fits {
        stack.push(flat(&parts[1]));
        stack.push(flat(&parts[0]));
    } else if content_fits {
        stack.push(broken(&parts[1]));
        stack.push(flat(&parts[0]));
    } else {
        stack.push(broken(&parts[1]));
        stack.push(broken(&parts[0]));
    }
}
