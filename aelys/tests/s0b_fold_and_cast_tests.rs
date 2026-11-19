use tempfile::tempdir;

mod common;
use common::{Cli, Leg};

// the value each level has to answer was computed outside this compiler, in `f32`, and the four levels only agree here by being right
const F32_FOLD: &str = "\
fn main() -> i64 {
    let sum: f32 = 0.1 + 0.2
    println(sum)
    let difference: f32 = 1.0 - 0.9
    println(difference)
    let product: f32 = 0.1 * 0.1
    println(product)
    let quotient: f32 = 1.0 / 3.0
    println(quotient)
    return 0
}
";

const F32_FOLD_ANSWER: &str = "0.30000001192092896\n\
     0.10000002384185791\n\
     0.010000000707805157\n\
     0.3333333432674408\n";

// `0.100000001` and `0.1` are the same `f32`, so a comparison folded on the wider literal answers for two numbers the program never holds
const F32_COMPARE: &str = "\
fn main() -> i64 {
    let tenth: f32 = 0.1
    let nudged: f32 = 0.100000001
    if tenth == nudged { println(1) } else { println(0) }
    if tenth < nudged { println(1) } else { println(0) }
    return 0
}
";

const F32_COMPARE_ANSWER: &str = "1\n0\n";

const FLOAT_TO_INT: &str = "\
fn main() -> i64 {
    let huge: f64 = 1e20
    let sunk: f64 = 0.0 - 1e20
    let below: f64 = 0.0 - 5.0
    let undefined: f64 = 0.0 / 0.0
    let narrow: f32 = 1e20
    println(huge as i32)
    println(huge as i64)
    println(huge as u8)
    println(sunk as i32)
    println(sunk as i64)
    println(below as u32)
    println(undefined as i64)
    println(narrow as i64)
    println(42.5 as i64)
    return 0
}
";

const FLOAT_TO_INT_ANSWER: &str = "2147483647\n\
     9223372036854775807\n\
     255\n\
     -2147483648\n\
     -9223372036854775808\n\
     0\n\
     0\n\
     9223372036854775807\n\
     42\n";

fn answers_everywhere(what: &str, src: &str, expected: &str) {
    let cli = Cli::located();
    let scratch = tempdir().expect("tempdir");
    let legs = cli.legs(scratch.path(), src);
    let shown = legs
        .iter()
        .map(|(level, leg)| format!("{level}: {}", leg.render()))
        .collect::<Vec<_>>()
        .join(" | ");
    for (level, leg) in &legs {
        let Leg::Ran(result) = leg else {
            panic!("{what} at {level} has to run to answer anything: {shown}");
        };
        assert_eq!(
            result.stdout, expected,
            "{what} at {level} answered something else than the value computed outside this \
             compiler: {shown}"
        );
    }
    let (_, first) = &legs[0];
    assert!(
        legs.iter().all(|(_, l)| l == first),
        "{what} answers differently at different -O levels: {shown}"
    );
}

#[test]
fn s0b_f32_arithmetic_folds_to_the_f32_value_at_every_level() {
    answers_everywhere("the f32 fold", F32_FOLD, F32_FOLD_ANSWER);
}

#[test]
fn s0b_an_f32_comparison_is_decided_on_the_f32_values_at_every_level() {
    answers_everywhere("the f32 comparison", F32_COMPARE, F32_COMPARE_ANSWER);
}

#[test]
fn s0b_a_float_to_int_cast_saturates_the_same_at_every_level() {
    answers_everywhere("the float to int cast", FLOAT_TO_INT, FLOAT_TO_INT_ANSWER);
}
