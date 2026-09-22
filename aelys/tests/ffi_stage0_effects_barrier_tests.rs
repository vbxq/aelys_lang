use std::collections::HashMap;
use std::sync::OnceLock;

use aelys_air::bir::effects::{EXTERN_DEFAULT, EXTERN_NOGC, effect_summaries_with_imports};
use aelys_air::bir::{
    BirBlock, BirBlockId, BirBody, BirExtern, BirLocalId, BirOperand, BirPlace, BirProgram,
    BirRvalue, BirStmt, BirStmtKind, BirTerminator, Effect, EffectSet,
};
use aelys_sema::InferType;
use aelys_syntax::{ForeignConv, ForeignDecl, Span};

fn declared(symbol: &str, declared_nogc: bool) -> BirExtern {
    BirExtern {
        foreign: ForeignDecl {
            symbol: symbol.to_string(),
            calling_conv: ForeignConv::C,
            is_unsafe: true,
            span: Span::dummy(),
        },
        declared_nogc,
    }
}

const SIX: [Effect; 6] = [
    Effect::Managed,
    Effect::Alloc,
    Effect::Panic,
    Effect::Unwind,
    Effect::Block,
    Effect::Io,
];

fn call_stmt(callee: Option<&str>, indirect_nogc: bool) -> BirStmt {
    BirStmt {
        kind: BirStmtKind::Assign {
            dest: BirPlace {
                local: BirLocalId(0),
                proj: Vec::new(),
            },
            rvalue: BirRvalue::Call {
                callee: callee.map(str::to_string),
                args: vec![BirOperand::Const],
                indirect_nogc,
            },
        },
        span: Span::dummy(),
    }
}

fn body(name: &str, stmts: Vec<BirStmt>) -> BirBody {
    BirBody {
        name: name.to_string(),
        locals: Vec::new(),
        arg_count: 0,
        blocks: vec![BirBlock {
            id: BirBlockId(0),
            stmts,
            term: BirTerminator::Return(None),
            term_span: Span::dummy(),
        }],
        entry: BirBlockId(0),
        span: Span::dummy(),
        scope_exits: Vec::new(),
        returns: Vec::new(),
        reassigns: Vec::new(),
        is_toplevel: false,
        return_type: InferType::I64,
        build_errors: Vec::new(),
        scope_deaths: Vec::new(),
        intrinsic_effects: EffectSet::EMPTY,
        managed_witness: None,
        declared_nogc: false,
    }
}

fn program() -> BirProgram {
    let mut externs = HashMap::new();
    externs.insert("c_ext".to_string(), declared("c_ext", false));
    externs.insert("c_ext_nogc".to_string(), declared("c_ext_nogc", true));
    BirProgram {
        bodies: vec![
            body("caller_of_unknown", vec![call_stmt(Some("c_absent"), false)]),
            body("caller_of_known", vec![call_stmt(Some("leaf"), false)]),
            body("leaf", Vec::new()),
            body("caller_indirect", vec![call_stmt(None, false)]),
            body("caller_of_extern", vec![call_stmt(Some("c_ext"), false)]),
            body(
                "caller_of_nogc_extern",
                vec![call_stmt(Some("c_ext_nogc"), false)],
            ),
        ],
        externs,
    }
}

// one program and one fixpoint run for the whole row, so no body can observe another's rerun
fn summaries() -> &'static HashMap<String, EffectSet> {
    static ONCE: OnceLock<HashMap<String, EffectSet>> = OnceLock::new();
    ONCE.get_or_init(|| effect_summaries_with_imports(&program(), &HashMap::new()))
}

fn summary_of(name: &str) -> EffectSet {
    *summaries()
        .get(name)
        .unwrap_or_else(|| panic!("no summary for `{name}`"))
}

fn pin(name: &str, expected: [bool; 6]) -> EffectSet {
    let eff = summary_of(name);
    for (effect, want) in SIX.iter().zip(expected) {
        assert_eq!(
            eff.contains(*effect),
            want,
            "{name}: {effect:?} should be {want}"
        );
    }
    eff
}

#[test]
fn caller_of_unknown_takes_the_barrier() {
    pin("caller_of_unknown", [true, true, true, false, false, false]);
}

#[test]
fn caller_of_known_stays_at_the_floor() {
    pin("caller_of_known", [false, false, false, false, false, false]);
}

#[test]
fn leaf_stays_at_the_floor() {
    pin("leaf", [false, false, false, false, false, false]);
}

#[test]
fn caller_indirect_takes_the_barrier() {
    pin("caller_indirect", [true, true, true, false, false, false]);
}

#[test]
fn caller_of_extern_takes_the_declared_default() {
    pin("caller_of_extern", [true, true, true, true, true, true]);
}

#[test]
fn caller_of_nogc_extern_takes_the_declared_nogc_default() {
    let eff = pin(
        "caller_of_nogc_extern",
        [false, true, true, true, true, true],
    );
    assert!(eff.is_nogc(), "a nogc extern leaves its caller nogc");
    assert_eq!(
        eff, EXTERN_NOGC,
        "the seed derives the nogc default from the declaration, it does not carry a set"
    );
}

#[test]
fn the_extern_default_is_not_the_barrier_value() {
    // top is crate private, and caller_of_unknown's summary is exactly it
    assert_ne!(
        EXTERN_DEFAULT,
        summary_of("caller_of_unknown"),
        "the declared default and the barrier must stay distinguishable"
    );
}

#[test]
fn the_barrier_is_not_what_a_known_callee_yields() {
    assert_ne!(
        summary_of("caller_of_unknown"),
        summary_of("caller_of_known"),
        "an absent callee and a known one must not summarise alike"
    );
}

// a second program, so the pathological name cannot enter the shared fixpoint the eight rows read
fn collided(with_declaration: bool) -> HashMap<String, EffectSet> {
    let mut externs = HashMap::new();
    if with_declaration {
        externs.insert("c_ext".to_string(), declared("c_ext", false));
    }
    let program = BirProgram {
        bodies: vec![body("c_ext", Vec::new())],
        externs,
    };
    effect_summaries_with_imports(&program, &HashMap::new())
}

// unreachable from source: one unit stops at , two units at , so this pins arithmetic
#[test]
fn a_body_homonymous_with_an_extern_is_seeded_by_the_declaration() {
    let alone = collided(false)["c_ext"];
    for effect in SIX {
        assert!(
            !alone.contains(effect),
            "c_ext: a call free body sits at the floor, {effect:?} should be absent"
        );
    }

    let seeded = collided(true)["c_ext"];
    assert_eq!(
        seeded, EXTERN_DEFAULT,
        "the seed unions the declared set into the body's own, so the body reports effects it \
         cannot perform"
    );
    assert!(
        !seeded.is_nogc(),
        "the pollution is conservative, so the body loses the nogc it earned"
    );
}
