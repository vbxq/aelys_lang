#[path = "level_legs/mod.rs"]
mod level_legs;

use aelys_driver::lower_file_to_air;
use aelys_opt::OptimizationLevel;
use level_legs::{LEVELS, LevelLeg, leg_at, legs_of, level_disagreements};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

const INT_TYPES: &[(&str, u32, bool)] = &[
    ("i8", 8, true),
    ("i16", 16, true),
    ("i32", 32, true),
    ("i64", 64, true),
    ("u8", 8, false),
    ("u16", 16, false),
    ("u32", 32, false),
    ("u64", 64, false),
];

const CAST_TARGETS: &[&str] = &[
    "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64",
];

const INT_OPS: &[&str] = &[
    "+", "-", "*", "/", "%", "&", "|", "^", "<<", ">>", "==", "!=", "<", "<=", ">", ">=",
];

const FLOAT_OPS: &[&str] = &["+", "-", "*", "/", "%", "==", "!=", "<", "<=", ">", ">="];

const F64_VALUES: &[&str] = &[
    "0.0", "-0.0", "1.5", "-2.25", "7.0", "1e300", "-1e300", "1e-300", "3.0e38",
];

const F32_VALUES: &[&str] = &[
    "0.0",
    "1.5",
    "-2.25",
    "7.0",
    "3.0e38",
    "-3.0e38",
    "1e-30",
    "16777217.0",
    "0.1",
];

fn int_values(width: u32, signed: bool) -> Vec<i128> {
    let w = width as i128;
    let mut candidates = if signed {
        let lo = -(1i128 << (width - 1));
        let hi = (1i128 << (width - 1)) - 1;
        vec![lo, lo + 1, -1, 0, 1, 2, 7, 9, w - 1, w, w + 1, hi]
    } else {
        let hi = (1i128 << width) - 1;
        vec![0, 1, 2, 7, 9, w - 1, w, w + 1, hi - 1, hi]
    };
    let mut seen = Vec::new();
    candidates.retain(|v| {
        let fresh = !seen.contains(v);
        seen.push(*v);
        fresh
    });
    candidates
}

fn int_initializer(ty: &str, value: i128) -> String {
    if ty == "i64" && value == i64::MIN as i128 {
        return "-9223372036854775807 - 1".to_string();
    }
    if ty == "u64" && value > i64::MAX as i128 {
        return format!("{} + 9223372036854775807 + 1", value - (1i128 << 63));
    }
    value.to_string()
}

fn int_program(ty: &str, width: u32, signed: bool) -> (String, usize) {
    let values = int_values(width, signed);
    let mut lines = vec!["fn main() -> i64 {".to_string()];
    for (i, v) in values.iter().enumerate() {
        lines.push(format!("    let v{i}: {ty} = {}", int_initializer(ty, *v)));
    }
    let min = -(1i128 << (width - 1));
    let mut ops = 0;
    for (i, a) in values.iter().enumerate() {
        for (j, b) in values.iter().enumerate() {
            let parts: Vec<String> = INT_OPS
                .iter()
                .map(|op| {
                    let traps =
                        matches!(*op, "/" | "%") && (*b == 0 || (signed && *a == min && *b == -1));
                    if traps {
                        "_".to_string()
                    } else {
                        ops += 1;
                        format!("{{v{i} {op} v{j}}}")
                    }
                })
                .collect();
            lines.push(format!("    println(\"{}\")", parts.join(" ")));
        }
        let mut unary = vec![format!("{{~v{i}}}")];
        if signed {
            unary.push(format!("{{-v{i}}}"));
        }
        unary.extend(CAST_TARGETS.iter().map(|t| format!("{{v{i} as {t}}}")));
        lines.push(format!("    println(\"{}\")", unary.join(" ")));
    }
    lines.push("    return 0".to_string());
    lines.push("}".to_string());
    (lines.join("\n") + "\n", ops)
}

fn float_program(ty: &str, values: &[&str]) -> (String, usize, usize) {
    let narrow = ty == "f32";
    let round = |x: f64| if narrow { x as f32 as f64 } else { x };
    let parsed: Vec<f64> = values
        .iter()
        .map(|v| round(v.parse::<f64>().expect("float value")))
        .collect();
    let mut lines = vec!["fn main() -> i64 {".to_string()];
    for (i, v) in values.iter().enumerate() {
        lines.push(format!("    let v{i}: {ty} = {v}"));
    }
    let mut ops = 0;
    let mut unfolded = 0;
    for (i, a) in parsed.iter().enumerate() {
        for (j, b) in parsed.iter().enumerate() {
            let parts: Vec<String> = FLOAT_OPS
                .iter()
                .map(|op| format!("{{v{i} {op} v{j}}}"))
                .collect();
            ops += FLOAT_OPS.len();
            for op in ["+", "-", "*", "/", "%"] {
                let result = match op {
                    "+" => a + b,
                    "-" => a - b,
                    "*" => a * b,
                    "/" => a / b,
                    _ => a % b,
                };
                let result = round(result);
                if (matches!(op, "/" | "%") && *b == 0.0) || !result.is_finite() {
                    unfolded += 1;
                }
            }
            lines.push(format!("    println(\"{}\")", parts.join(" ")));
        }
        let mut unary = vec![format!("{{-v{i}}}")];
        unary.extend(CAST_TARGETS.iter().map(|t| format!("{{v{i} as {t}}}")));
        lines.push(format!("    println(\"{}\")", unary.join(" ")));
    }
    lines.push("    return 0".to_string());
    lines.push("}".to_string());
    (lines.join("\n") + "\n", ops, unfolded)
}

fn binops_in_air(root: &Path, level: OptimizationLevel) -> usize {
    let air = lower_file_to_air(root, level).expect("the differential program lowers");
    aelys_air::print::print_program(&air)
        .lines()
        .filter(|line| line.contains("= binop "))
        .count()
}

fn differential(
    dir: &Path,
    name: &str,
    src: &str,
    ops: usize,
    unfolded_at_o1: usize,
) -> Vec<String> {
    let mut failures = Vec::new();
    let legs = legs_of(dir, name, src, None, &[]);
    for (tag, leg) in &legs {
        match leg {
            LevelLeg::Ran {
                exit: 0, stderr, ..
            } if stderr.is_empty() => {}
            other => failures.push(format!("{name} at {tag}: {}", other.render())),
        }
    }
    failures.extend(level_disagreements(name, &legs));
    let probe = dir.join(format!("{name}_air")).join("root.aelys");
    fs::create_dir_all(probe.parent().expect("air dir")).expect("air dir");
    fs::write(&probe, src).expect("write air probe");
    let at_o0 = binops_in_air(&probe, OptimizationLevel::None);
    let at_o1 = binops_in_air(&probe, OptimizationLevel::Basic);
    if at_o0 < ops {
        failures.push(format!(
            "{name}: -O0 holds {at_o0} binops for {ops} operations"
        ));
    }
    if at_o1 != unfolded_at_o1 {
        failures.push(format!(
            "{name}: -O1 holds {at_o1} binops, the folder leaves exactly {unfolded_at_o1}"
        ));
    }
    failures
}

#[test]
fn release01_55_the_constant_folder_answers_what_the_runtime_answers() {
    let dir = tempdir().expect("tempdir");
    let mut failures = Vec::new();
    let mut programs = 0;
    for (ty, width, signed) in INT_TYPES {
        let (src, ops) = int_program(ty, *width, *signed);
        failures.extend(differential(dir.path(), &format!("int_{ty}"), &src, ops, 0));
        programs += 1;
    }
    for (ty, values) in [("f32", F32_VALUES), ("f64", F64_VALUES)] {
        let (src, ops, unfolded) = float_program(ty, values);
        assert!(
            unfolded > 0 && unfolded < ops,
            "{ty}: {unfolded} unfolded of {ops}"
        );
        failures.extend(differential(
            dir.path(),
            &format!("flt_{ty}"),
            &src,
            ops,
            unfolded,
        ));
        programs += 1;
    }
    assert_eq!(programs, 10, "release01_55: programs run");
    assert!(
        failures.is_empty(),
        "release01_55:\n{}",
        failures.join("\n")
    );
}

#[test]
fn release01_56_a_trap_the_folder_meets_is_left_to_the_runtime() {
    let dir = tempdir().expect("tempdir");
    let mut failures = Vec::new();
    let mut cases = 0;
    for (ty, width, signed) in INT_TYPES {
        let mut programs: Vec<(String, String, &str)> = Vec::new();
        for op in ["/", "%"] {
            let tag = if op == "/" { "div" } else { "rem" };
            programs.push((
                format!("{ty}_{tag}_zero"),
                format!("let a: {ty} = 7\n    let b: {ty} = 0"),
                "division by zero\n",
            ));
            if *signed {
                let min = int_initializer(ty, -(1i128 << (width - 1)));
                programs.push((
                    format!("{ty}_{tag}_min_by_minus_one"),
                    format!("let a: {ty} = {min}\n    let b: {ty} = -1"),
                    "division overflow\n",
                ));
            }
            for (name, lets, message) in programs.drain(..) {
                let src = format!(
                    "fn main() -> i64 {{\n    {lets}\n    println(\"{{a {op} b}}\")\n    return 0\n}}\n"
                );
                let expected = LevelLeg::Ran {
                    stdout: String::new(),
                    stderr: message.to_string(),
                    exit: 134,
                };
                for (tag, leg) in legs_of(dir.path(), &name, &src, None, &[]) {
                    if leg != expected {
                        failures.push(format!("{name} at {tag}: {}", leg.render()));
                    }
                }
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 24, "release01_56: trap programs run");
    assert!(
        failures.is_empty(),
        "release01_56:\n{}",
        failures.join("\n")
    );
}

fn fixtures() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/release01_levels");
    let mut out: Vec<(String, String)> = fs::read_dir(&dir)
        .expect("fixture dir")
        .map(|entry| entry.expect("fixture entry").path())
        .filter(|path| path.extension().is_some_and(|e| e == "case"))
        .map(|path| {
            let stem = path
                .file_stem()
                .expect("stem")
                .to_string_lossy()
                .into_owned();
            let src = fs::read_to_string(&path).expect("read fixture");
            (stem, src)
        })
        .collect();
    out.sort();
    out
}

enum Expect {
    Out(&'static str),
    Panic(&'static str),
    Code(&'static str),
}

const PINNED: &[(&str, Expect)] = &[
    ("m_a_oob", Expect::Panic("index out of bounds\n")),
    ("m_a_stmt_idx", Expect::Panic("index out of bounds\n")),
    ("m_b_div0", Expect::Panic("division by zero\n")),
    ("m_b_stmt_div", Expect::Panic("division by zero\n")),
    ("m_c2_1", Expect::Out("2\n")),
    ("m_c2_2", Expect::Out("20\n")),
    ("m_c_inline_arg", Expect::Panic("index out of bounds\n")),
    ("m_c_vec_oob", Expect::Panic("index out of bounds\n")),
    ("m_d_null_rc", Expect::Panic("null pointer dereference\n")),
    ("m_e_i8_div", Expect::Panic("division overflow\n")),
    ("m_e_mod0", Expect::Panic("division by zero\n")),
    ("m_f_min_div", Expect::Panic("division overflow\n")),
    ("m_f_oob_const", Expect::Code("E0433")),
    ("m_h_shift_u8_var", Expect::Out("2\n2\n")),
    ("m_i_dead_if_trap", Expect::Panic("division by zero\n")),
    ("m_nogc_member_through_borrowed_rc", Expect::Code("E0410")),
    ("m_o_if_lit_init", Expect::Code("E0301")),
    ("p_f1_i16_rem", Expect::Panic("division overflow\n")),
    (
        "p_f2_u64_cmp",
        Expect::Out("true 6148914691236517205 15 1 false\n"),
    ),
    ("p_f3_i8_shr", Expect::Out("-64 0 32767 65534\n")),
    ("p_f4_div0_const", Expect::Panic("division by zero\n")),
    (
        "p_f5_nan_unequal",
        Expect::Out("true true true true false false true false false true\n"),
    ),
    ("p_i1_cond_arg", Expect::Panic("index out of bounds\n")),
    ("p_i2_order", Expect::Panic("division by zero\n")),
    ("p_i3_and_arg", Expect::Panic("index out of bounds\n")),
    ("p_i4_stmt_call", Expect::Panic("index out of bounds\n")),
    ("p_l1_struct", Expect::Panic("index out of bounds\n")),
    ("p_l2_arr", Expect::Panic("division by zero\n")),
    ("p_l3_and", Expect::Panic("division by zero\n")),
    ("p_l4_match", Expect::Panic("division by zero\n")),
    ("p_l5_rem_min", Expect::Panic("division overflow\n")),
    ("p_l6_deref", Expect::Panic("null pointer dereference\n")),
    (
        "p_s1_member_stmt",
        Expect::Panic("null pointer dereference\n"),
    ),
    ("p_s2_add_stmt", Expect::Panic("index out of bounds\n")),
    ("p_s3_if_stmt", Expect::Panic("division by zero\n")),
    ("p_s4_neg_cast_stmt", Expect::Panic("index out of bounds\n")),
    ("p_s5_block_stmt", Expect::Panic("division by zero\n")),
    ("v1_big_stack_array", Expect::Code("E0904")),
    ("v1_lambda_unused", Expect::Code("E0304")),
    ("v1_rc_field_break", Expect::Code("E0902")),
    ("v1_vec_in_generic_payload", Expect::Code("E0412")),
];

const SAMPLED_CODES: usize = 68;

fn expectation_failure(name: &str, leg: &LevelLeg, expect: &Expect) -> Option<String> {
    let met = match (expect, leg) {
        (
            Expect::Out(out),
            LevelLeg::Ran {
                stdout,
                stderr,
                exit,
            },
        ) => stdout == out && stderr.is_empty() && *exit == 0,
        (
            Expect::Panic(message),
            LevelLeg::Ran {
                stdout,
                stderr,
                exit,
            },
        ) => stdout.is_empty() && stderr == message && *exit == 134,
        (Expect::Code(code), leg) => return code_failure(name, leg, code),
        _ => false,
    };
    (!met).then(|| format!("{name} at -O0: {}", leg.render()))
}

fn code_failure(name: &str, leg: &LevelLeg, code: &str) -> Option<String> {
    let met = leg.refusal().is_some_and(|rendered| {
        rendered.contains(&format!("[{code}]")) && !rendered.contains("[E0432]")
    });
    (!met).then(|| format!("{name} at -O0: {}", leg.render()))
}

#[test]
fn release01_57_verdict_and_behaviour_are_the_same_at_every_level() {
    let dir = tempdir().expect("tempdir");
    let mut failures = Vec::new();
    let mut legs_run = 0;
    let mut pinned = 0;
    let mut codes = BTreeSet::new();
    for (name, src) in fixtures() {
        let legs = legs_of(dir.path(), &name, &src, None, &[]);
        legs_run += legs.len();
        failures.extend(level_disagreements(&name, &legs));
        let at_o0 = &legs[0].1;
        if let Some((_, expect)) = PINNED.iter().find(|(stem, _)| *stem == name) {
            pinned += 1;
            failures.extend(expectation_failure(&name, at_o0, expect));
        } else if let Some(code) = name.strip_prefix("c_").and_then(|rest| rest.get(..5)) {
            codes.insert(code.to_string());
            failures.extend(code_failure(&name, at_o0, code));
        } else {
            failures.push(format!("{name}: a fixture with no expectation"));
        }
    }
    assert!(
        failures.is_empty(),
        "release01_57:\n{}",
        failures.join("\n")
    );
    assert_eq!(pinned, PINNED.len(), "release01_57: pinned members run");
    assert_eq!(
        codes.len(),
        SAMPLED_CODES,
        "release01_57: refusal codes sampled"
    );
    assert_eq!(
        legs_run,
        (PINNED.len() + SAMPLED_CODES) * LEVELS.len(),
        "release01_57: legs run"
    );
}

#[test]
fn release01_58_the_level_comparator_reports_every_differing_pair() {
    let ran = |stdout: &str, stderr: &str, exit: i32| LevelLeg::Ran {
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
        exit,
    };
    let pairs = [
        ("stdout", ran("1\n", "", 0), ran("0\n", "", 0)),
        ("exit", ran("1\n", "", 0), ran("1\n", "", 1)),
        (
            "panic message",
            ran("", "index out of bounds\n", 134),
            ran("", "division by zero\n", 134),
        ),
        (
            "verdict",
            ran("", "", 0),
            LevelLeg::Refused("error[E0301]".into()),
        ),
        (
            "code",
            LevelLeg::Refused("error[E0412]".into()),
            LevelLeg::Refused("error[E0432]".into()),
        ),
        (
            "span",
            LevelLeg::Refused("error[E0301]\n --> <stage>/root.aelys:2:5".into()),
            LevelLeg::Refused("error[E0301]\n --> <stage>/root.aelys:3:5".into()),
        ),
        ("termination", ran("", "", 0), LevelLeg::DidNotTerminate),
    ];
    for (what, a, b) in pairs {
        let legs = [
            ("-O0", a.clone()),
            ("-O1", a.clone()),
            ("-O2", b),
            ("-O3", a.clone()),
        ];
        let found = level_disagreements(what, &legs);
        assert_eq!(
            found.len(),
            1,
            "a pair differing by {what} is reported once: {found:?}"
        );
        let same = [
            ("-O0", a.clone()),
            ("-O1", a.clone()),
            ("-O2", a.clone()),
            ("-O3", a),
        ];
        assert!(
            level_disagreements(what, &same).is_empty(),
            "equal legs differing by {what}"
        );
    }
    let dir = tempdir().expect("tempdir");
    let one = leg_at(
        &dir.path().join("a"),
        "fn main() -> i64 { let x: u8 = 1000\n return 0 }\n",
        None,
        OptimizationLevel::None,
        &[],
    );
    let two = leg_at(
        &dir.path().join("b"),
        "fn main() -> i64 { let x: u8 = 1000\n return 0 }\n",
        None,
        OptimizationLevel::None,
        &[],
    );
    assert!(
        one.refusal().is_some(),
        "the control program is refused: {}",
        one.render()
    );
    assert_eq!(
        one, two,
        "the stage directory is cut out of a rendered refusal"
    );
}

#[test]
fn release01_59_a_reference_refusal_is_the_verdict_at_every_level() {
    let dir = tempdir().expect("tempdir");
    let mut failures = Vec::new();
    let mut members = 0;
    for (name, src) in fixtures()
        .into_iter()
        .filter(|(name, _)| name.starts_with("v1_"))
    {
        members += 1;
        let legs = legs_of(dir.path(), &name, &src, None, &[]);
        for (tag, leg) in &legs {
            match leg.refusal() {
                Some(rendered) if !rendered.contains("E0432") => {}
                _ => failures.push(format!("{name} at {tag}: {}", leg.render())),
            }
        }
        failures.extend(level_disagreements(&name, &legs));
    }
    assert_eq!(members, 4, "release01_59: members");
    assert!(
        failures.is_empty(),
        "release01_59:\n{}",
        failures.join("\n")
    );
}

const KEPT_DROPS: &[(&str, &str, &str)] = &[
    (
        "shared_vec_written",
        "fn main() -> i64 { let mut v: Vec<i64> = vec[1, 2]\n    let w = v\n    v[0] = 5\n    println(\"{v[0]}\")\n    return 0 }\n",
        "[rc] allocs=2 frees=2\n[raw] allocs=0 frees=0\n",
    ),
    (
        "shared_vec_pushed_in_loop",
        "fn main() -> i64 { let mut v: Vec<i64> = vec[1, 2]\n    for i in 0..3 { let w = v\n        Vec::push(v, i) }\n    println(\"{v.len}\")\n    return 0 }\n",
        "[rc] allocs=4 frees=4\n[raw] allocs=0 frees=0\n",
    ),
];

#[test]
fn release01_60_an_unused_binding_of_a_managed_value_is_kept_at_every_level() {
    let dir = tempdir().expect("tempdir");
    let mut failures = Vec::new();
    for (name, src, counters) in KEPT_DROPS {
        let expected = LevelLeg::Ran {
            stdout: "5\n".to_string(),
            stderr: counters.to_string(),
            exit: 0,
        };
        for (tag, leg) in legs_of(dir.path(), name, src, None, &[("AELYS_RC_STATS", "1")]) {
            if leg != expected {
                failures.push(format!("{name} at {tag}: {}", leg.render()));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "release01_60:\n{}",
        failures.join("\n")
    );
}

#[test]
fn release01_61_e0432_is_raised_only_when_the_optimized_half_alone_refuses() {
    let dir = tempdir().expect("tempdir");
    let src = "fn main() -> i64 {\n    println(\"1\")\n    return 0\n}\n";
    aelys_air::ablation::set_optimized_stage_refuses(true);
    let ablated = legs_of(dir.path(), "ablated", src, None, &[]);
    aelys_air::ablation::set_optimized_stage_refuses(false);
    let repaired = legs_of(dir.path(), "repaired", src, None, &[]);
    let runs = LevelLeg::Ran {
        stdout: "1\n".to_string(),
        stderr: String::new(),
        exit: 0,
    };
    assert_eq!(ablated[0].1, runs, "-O0 has no optimized half to refuse");
    for (tag, leg) in &ablated[1..] {
        let rendered = leg
            .refusal()
            .unwrap_or_else(|| panic!("ablated at {tag}: {}", leg.render()));
        for fragment in [
            "error[E0432]",
            "the optimized stage is refused by the ablation switch",
            &format!(
                "raised by the optimized pipeline at {tag} while the reference pipeline at -O0 accepts"
            ),
            "compiler defect",
        ] {
            assert!(
                rendered.contains(fragment),
                "ablated at {tag}: E0432 never says {fragment:?}\n{rendered}"
            );
        }
    }
    for (tag, leg) in &repaired {
        assert_eq!(leg, &runs, "repaired at {tag}");
    }
}
