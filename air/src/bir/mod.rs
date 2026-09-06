// bir, the borrow/ownership ir, built from the typed ast and checked before air lowering.

pub mod build;
pub mod category;
pub mod effects;
pub mod loans;
pub mod moves;
pub mod origins;

use aelys_sema::{InferType, TypedProgram};
use aelys_syntax::ForeignConv;

pub use category::{Category, category};
pub use effects::{Effect, EffectSet, Step, StepKind, effect_summaries, managed_chain};
pub use moves::{BirCheck, check_program, check_program_with_imports};

// the private field is the seal: check is the only constructor, so optimize cannot run unchecked
pub struct Checked(TypedProgram);

impl Checked {
    pub fn program(&self) -> &TypedProgram {
        &self.0
    }
    pub fn into_inner(self) -> TypedProgram {
        self.0
    }
}

pub fn check(program: TypedProgram) -> Result<Checked, Vec<BirDiagnostic>> {
    check_with_imports(program, &Imports::default()).map(|(checked, _)| checked)
}

#[derive(Clone)]
pub struct ForeignSig {
    pub symbol: String,
    pub calling_conv: ForeignConv,
    pub params: Vec<InferType>,
    pub ret: InferType,
    pub declared_nogc: bool,
    pub span: aelys_syntax::Span,
}

pub struct ForeignClash {
    pub name: String,
    pub reason: String,
    pub span: aelys_syntax::Span,
    pub same_unit: bool,
}

pub fn foreign_conflict(a: &ForeignSig, b: &ForeignSig) -> Option<String> {
    if a.symbol != b.symbol {
        return Some(format!(
            "one names the symbol `{}` and the other names `{}`",
            a.symbol, b.symbol
        ));
    }
    if a.calling_conv != b.calling_conv {
        return Some("the two declarations claim different calling conventions".to_string());
    }
    if a.declared_nogc != b.declared_nogc {
        return Some(
            "one declaration claims `nogc` and the other does not, so the two promise different \
             effects for one symbol"
                .to_string(),
        );
    }
    if a.params.len() != b.params.len() {
        return Some(format!(
            "one declaration takes {} parameters and the other takes {}",
            a.params.len(),
            b.params.len()
        ));
    }
    if a.params != b.params || a.ret != b.ret {
        return Some("the two declarations give the symbol different types".to_string());
    }
    None
}

pub fn foreign_signatures(
    program: &TypedProgram,
) -> (std::collections::HashMap<String, ForeignSig>, Vec<ForeignClash>) {
    let mut sigs: std::collections::HashMap<String, ForeignSig> = Default::default();
    let mut clashes = Vec::new();
    build::for_each_fn_decl(&program.stmts, &mut |func, _parent| {
        let Some(foreign) = &func.foreign else {
            return;
        };
        let sig = ForeignSig {
            symbol: foreign.symbol.clone(),
            calling_conv: foreign.calling_conv,
            params: func.params.iter().map(|p| p.ty.clone()).collect(),
            ret: func.return_type.clone(),
            declared_nogc: func.declared_nogc,
            span: func.span,
        };
        if let Some(seen) = sigs.get(&func.name) {
            let reason = foreign_conflict(seen, &sig).unwrap_or_else(|| {
                "the symbol is declared twice in one unit".to_string()
            });
            clashes.push(ForeignClash {
                name: func.name.clone(),
                reason,
                span: func.span,
                same_unit: true,
            });
            return;
        }
        sigs.insert(func.name.clone(), sig);
    });
    (sigs, clashes)
}

// two units may declare one symbol, and they agree or the merge is refused
pub fn foreign_merge_clashes(
    own: &std::collections::HashMap<String, ForeignSig>,
    imported: &std::collections::HashMap<String, ForeignSig>,
) -> Vec<ForeignClash> {
    let mut names: Vec<&String> = own.keys().collect();
    names.sort();
    names
        .into_iter()
        .filter_map(|name| {
            let mine = own.get(name)?;
            let theirs = imported.get(name)?;
            let reason = foreign_conflict(mine, theirs)?;
            Some(ForeignClash {
                name: name.clone(),
                reason,
                span: mine.span,
                same_unit: false,
            })
        })
        .collect()
}

#[derive(Default, Clone)]
pub struct Imports {
    pub effects: std::collections::HashMap<String, EffectSet>,
    pub chains: std::collections::HashMap<String, Vec<Step>>,
    pub externs: std::collections::HashMap<String, ForeignSig>,
}

impl Imports {
    // a name resolves exactly when a summary arrived for it, so the two cannot drift apart
    pub fn names(&self) -> std::collections::HashSet<String> {
        self.effects.keys().cloned().collect()
    }
}

pub struct Published {
    pub effects: std::collections::HashMap<String, EffectSet>,
    pub chains: std::collections::HashMap<String, Vec<Step>>,
    pub externs: std::collections::HashMap<String, ForeignSig>,
    pub foreign_clashes: Vec<ForeignClash>,
}

pub fn check_with_imports(
    program: TypedProgram,
    imports: &Imports,
) -> Result<(Checked, Published), Vec<BirDiagnostic>> {
    let bir = build::build_program_with_imports(&program, &imports.names());
    let result = moves::check_program_with_chains(&bir, &imports.effects, &imports.chains);
    if !result.errors.is_empty() {
        return Err(result.errors);
    }
    let own: std::collections::HashSet<&str> = bir.bodies.iter().map(|b| b.name.as_str()).collect();
    let mut effects = result.effects;
    let mut chains = std::collections::HashMap::new();
    for body in &bir.bodies {
        if own.contains(body.name.as_str()) && effects.get(&body.name).is_some_and(|e| !e.is_nogc())
        {
            let steps = effects::managed_chain_with(&bir, &effects, &imports.chains, body);
            chains.insert(body.name.clone(), steps);
        }
    }
    effects.retain(|name, _| own.contains(name.as_str()));
    let (externs, mut foreign_clashes) = foreign_signatures(&program);
    foreign_clashes.extend(foreign_merge_clashes(&externs, &imports.externs));
    Ok((
        Checked(program),
        Published {
            effects,
            chains,
            externs,
            foreign_clashes,
        },
    ))
}

// a borrow or a slice (the single source for param/loan/store seeding)
pub(crate) fn is_ref_ty(ty: &InferType) -> bool {
    matches!(ty, InferType::Ref { .. } | InferType::Slice { .. })
}

pub type DropKey = (usize, usize);

pub fn drop_key(span: &aelys_syntax::Span) -> DropKey {
    (span.start, span.end)
}

#[derive(Clone)]
pub struct BirDiagnostic {
    pub code: &'static str,
    pub primary: (aelys_syntax::Span, String),
    // extra carets: borrow-created / last-used / moved-here / declared-here / destroyed-here
    pub secondaries: Vec<(aelys_syntax::Span, String)>,
    pub marker: &'static str,
    pub help: Option<String>,
    pub note: Option<String>,
    pub hint: Option<String>,
}

impl BirDiagnostic {
    pub(crate) fn new(
        code: &'static str,
        marker: &'static str,
        primary_span: aelys_syntax::Span,
        message: String,
    ) -> Self {
        Self {
            code,
            primary: (primary_span, message),
            secondaries: Vec::new(),
            marker,
            help: None,
            note: None,
            hint: None,
        }
    }

    pub(crate) fn with_secondary(mut self, span: aelys_syntax::Span, label: String) -> Self {
        self.secondaries.push((span, label));
        self
    }

    pub(crate) fn with_help(mut self, help: String) -> Self {
        self.help = Some(help);
        self
    }

    pub(crate) fn with_note(mut self, note: String) -> Self {
        self.note = Some(note);
        self
    }

    pub(crate) fn with_hint(mut self, hint: String) -> Self {
        self.hint = Some(hint);
        self
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BirLocalId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BirBlockId(pub u32);

pub struct BirProgram {
    pub bodies: Vec<BirBody>,
    pub externs: std::collections::HashMap<String, BirExtern>,
}

// holds the declaration and not its effect set, so the seed derives the set and a false nogc claim cannot survive
pub struct BirExtern {
    pub foreign: aelys_syntax::ForeignDecl,
    pub declared_nogc: bool,
}

pub struct BirBody {
    pub name: String,
    pub locals: Vec<BirLocal>,
    pub arg_count: usize,
    pub blocks: Vec<BirBlock>,
    pub entry: BirBlockId,
    pub span: aelys_syntax::Span,
    pub scope_exits: Vec<ScopeExit>,
    pub returns: Vec<ReturnPoint>,
    pub reassigns: Vec<Reassign>,
    pub is_toplevel: bool,
    pub return_type: InferType,
    pub build_errors: Vec<BirDiagnostic>,
    pub scope_deaths: Vec<ScopeDeath>,
    #[allow(dead_code)]
    pub intrinsic_effects: EffectSet,
    pub managed_witness: Option<(aelys_syntax::Span, String)>,
    pub declared_nogc: bool,
}

// every named non-parameter local dying at a lexical scope exit; read by the escape pass
pub struct ScopeDeath {
    pub block: BirBlockId,
    pub index: usize,
    pub locals: Vec<BirLocalId>,
    // the death-point caret for the escape-death diagnostic
    pub scope_span: aelys_syntax::Span,
}

pub struct ScopeExit {
    pub scope_span: aelys_syntax::Span,
    pub exit_block: BirBlockId,
    pub exit_index: usize,
    pub locals: Vec<BirLocalId>,
}

pub struct ReturnPoint {
    pub span: aelys_syntax::Span,
    pub block: BirBlockId,
    pub in_scope: Vec<BirLocalId>,
}

pub struct Reassign {
    pub span: aelys_syntax::Span,
    pub block: BirBlockId,
    pub stmt_index: usize,
    pub local: BirLocalId,
}

pub struct BirLocal {
    pub id: BirLocalId,
    pub name: Option<String>,
    pub ty: InferType,
    pub category: Category,
    pub decl_span: aelys_syntax::Span,
    pub mutable: bool,
}

pub struct BirBlock {
    pub id: BirBlockId,
    pub stmts: Vec<BirStmt>,
    pub term: BirTerminator,
    pub term_span: aelys_syntax::Span,
}

pub struct BirStmt {
    pub kind: BirStmtKind,
    pub span: aelys_syntax::Span,
}

pub enum BirStmtKind {
    Assign { dest: BirPlace, rvalue: BirRvalue },
    StorageLive(BirLocalId),
    StorageDead(BirLocalId),
    Drop(BirLocalId),
}

pub enum BirTerminator {
    Goto(BirBlockId),
    Branch {
        discr: BirOperand,
        targets: Vec<BirBlockId>,
    },
    Return(Option<BirOperand>),
    Unreachable,
}

pub enum BirRvalue {
    Use(BirOperand),
    Aggregate(Vec<BirOperand>),
    BinOp(BirOperand, BirOperand),
    UnOp(BirOperand),
    Call {
        callee: Option<String>,
        args: Vec<BirOperand>,
        indirect_nogc: bool,
    },
    Ref {
        place: BirPlace,
        mutable: bool,
    },
    #[allow(dead_code)]
    Reborrow {
        place: BirPlace,
        mutable: bool,
    },
}

pub enum BirOperand {
    Copy(BirPlace),
    Move(BirPlace),
    Const,
}

#[derive(Clone)]
pub struct BirPlace {
    pub local: BirLocalId,
    pub proj: Vec<BirProjection>,
}

#[derive(Clone)]
pub enum BirProjection {
    Field(String),
    Index,
    Deref,
}
