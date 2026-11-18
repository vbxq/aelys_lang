use super::graph::ModuleUnit;
use aelys_syntax::{ImportKind, NeedsTarget, Span, StmtKind};
use std::collections::HashSet;
use std::ops::Range;

struct Candidate {
    unit: usize,
    stmt: usize,
    method: Option<usize>,
    name: String,
    span: Span,
}

// every reference to a function spells its name, so a std function no kept text names is never reached
pub(crate) fn strip_unnamed_std_functions(units: &mut [ModuleUnit]) {
    let mut words: HashSet<String> = HashSet::new();
    let mut candidates: Vec<Candidate> = Vec::new();
    for (u, unit) in units.iter().enumerate() {
        let text = &unit.source.content;
        let is_std = unit.dotted.starts_with("std.");
        let mut imports: Vec<Range<usize>> = Vec::new();
        for (s, stmt) in unit.stmts.iter().enumerate() {
            match &stmt.kind {
                StmtKind::Needs(needs) => {
                    imports.push(stmt.span.start..stmt.span.end);
                    if let NeedsTarget::Module {
                        kind: ImportKind::Symbols(names),
                        ..
                    } = &needs.target
                    {
                        for (item, _) in names {
                            add_words(&mut words, item);
                        }
                    }
                }
                _ if !is_std => {}
                StmtKind::Function(f) if f.foreign.is_none() => candidates.push(Candidate {
                    unit: u,
                    stmt: s,
                    method: None,
                    name: f.name.clone(),
                    span: f.span,
                }),
                StmtKind::Impl(decl) if decl.trait_ref.is_none() => {
                    for (m, method) in decl.methods.iter().enumerate() {
                        candidates.push(Candidate {
                            unit: u,
                            stmt: s,
                            method: Some(m),
                            name: method.func.name.clone(),
                            span: method.func.span,
                        });
                    }
                }
                _ => add_words(&mut words, &text[stmt.span.start..stmt.span.end]),
            }
        }
        if !is_std {
            // one `use` with a brace list yields several imports sharing its span
            imports.sort_by_key(|span| span.start);
            let mut from = 0;
            for span in imports {
                if span.start > from {
                    add_words(&mut words, &text[from..span.start]);
                }
                from = from.max(span.end);
            }
            add_words(&mut words, &text[from..]);
        }
    }
    let mut kept = vec![false; candidates.len()];
    loop {
        let before = words.len();
        for (candidate, kept) in candidates.iter().zip(kept.iter_mut()) {
            if !*kept && words.contains(&candidate.name) {
                *kept = true;
                let text = &units[candidate.unit].source.content;
                add_words(&mut words, &text[candidate.span.start..candidate.span.end]);
            }
        }
        if words.len() == before {
            break;
        }
    }
    for (candidate, _) in candidates.iter().zip(&kept).rev().filter(|(_, k)| !**k) {
        let stmts = &mut units[candidate.unit].stmts;
        match candidate.method {
            None => {
                stmts.remove(candidate.stmt);
            }
            Some(m) => {
                if let StmtKind::Impl(decl) = &mut stmts[candidate.stmt].kind {
                    decl.methods.remove(m);
                }
            }
        }
    }
}

fn add_words(words: &mut HashSet<String>, text: &str) {
    for word in text.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
        if !word.is_empty() && !words.contains(word) {
            words.insert(word.to_string());
        }
    }
}
