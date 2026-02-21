use aelys_air::layout::compute_layouts;
use aelys_air::lower::lower;
use aelys_air::mono::monomorphize;
use aelys_air::print::print_program;
use aelys_air::*;
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_sema::TypeInference;
use aelys_syntax::Source;

fn lower_source(code: &str) -> AirProgram {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    let typed = TypeInference::infer_program(stmts, src).expect("sema failed");
    lower(&typed)
}

fn lower_optimized(code: &str) -> AirProgram {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    let typed = TypeInference::infer_program(stmts, src).expect("sema failed");
    let mut opt = aelys_opt::Optimizer::new(aelys_opt::OptimizationLevel::Standard);
    let optimized = opt.optimize(typed);
    lower(&optimized)
}

fn func<'a>(air: &'a AirProgram, name: &str) -> &'a AirFunction {
    air.functions
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("function `{}` not found in AIR", name))
}

fn collect_block_ids(f: &AirFunction) -> Vec<u32> {
    f.blocks.iter().map(|b| b.id.0).collect()
}

fn collect_branch_targets(f: &AirFunction) -> Vec<u32> {
    let mut targets = Vec::new();
    for block in &f.blocks {
        match &block.terminator {
            AirTerminator::Goto(id) => targets.push(id.0),
            AirTerminator::Branch {
                then_block,
                else_block,
                ..
            } => {
                targets.push(then_block.0);
                targets.push(else_block.0);
            }
            _ => {}
        }
    }
    targets
}

// bug fix: block ID coherence, all branch targets must reference existing blocks

#[test]
fn block_ids_coherent_in_if_statement() {
    let air = lower_source(
        r#"
fn test(n: i32) -> i32 {
    if n > 0 {
        return 1
    }
    return 0
}
"#,
    );
    let f = func(&air, "test");
    let ids: std::collections::HashSet<u32> = collect_block_ids(f).into_iter().collect();
    for target in collect_branch_targets(f) {
        assert!(
            ids.contains(&target),
            "branch target block{} does not exist, existing blocks: {:?}",
            target,
            ids
        );
    }
}

#[test]
fn block_ids_coherent_in_while_loop() {
    let air = lower_source(
        r#"
fn sum(n: i32) -> i32 {
    let total: i32 = 0
    let i: i32 = 0
    while i < n {
        total = total + i
        i = i + 1
    }
    return total
}
"#,
    );
    let f = func(&air, "sum");
    let ids: std::collections::HashSet<u32> = collect_block_ids(f).into_iter().collect();
    for target in collect_branch_targets(f) {
        assert!(
            ids.contains(&target),
            "branch target block{} does not exist in while loop, existing blocks: {:?}",
            target,
            ids
        );
    }
}

#[test]
fn block_ids_coherent_in_if_else() {
    let air = lower_source(
        r#"
fn abs(n: i32) -> i32 {
    if n < 0 {
        return 0 - n
    } else {
        return n
    }
}
"#,
    );
    let f = func(&air, "abs");
    let ids: std::collections::HashSet<u32> = collect_block_ids(f).into_iter().collect();
    for target in collect_branch_targets(f) {
        assert!(
            ids.contains(&target),
            "branch target block{} does not exist in if-else, existing blocks: {:?}",
            target,
            ids
        );
    }
}

#[test]
fn block_ids_coherent_in_nested_control_flow() {
    let air = lower_source(
        r#"
fn classify(n: i32) -> i32 {
    let result: i32 = 0
    if n > 0 {
        let i: i32 = 0
        while i < n {
            if i > 5 {
                result = result + 2
            } else {
                result = result + 1
            }
            i = i + 1
        }
    }
    return result
}
"#,
    );
    let f = func(&air, "classify");
    let ids: std::collections::HashSet<u32> = collect_block_ids(f).into_iter().collect();
    for target in collect_branch_targets(f) {
        assert!(
            ids.contains(&target),
            "branch target block{} does not exist in nested control flow, existing blocks: {:?}",
            target,
            ids
        );
    }
}

// bug fix: Integer literal narrowing in binary operations

#[test]
fn binop_operands_have_consistent_types_i32() {
    let air = lower_source(
        r#"
fn test(n: i32) -> bool {
    return n <= 1
}
"#,
    );
    let printed = print_program(&air);
    assert!(
        !printed.contains("1i64"),
        "literal 1 should be narrowed to i32 when compared with i32 parameter, got:\n{}",
        printed
    );
}

#[test]
fn binop_sub_narrowed_to_i32() {
    let air = lower_source(
        r#"
fn test(n: i32) -> i32 {
    return n - 1
}
"#,
    );
    let printed = print_program(&air);
    assert!(
        !printed.contains("1i64"),
        "literal 1 in subtraction should be narrowed to i32, got:\n{}",
        printed
    );
}

#[test]
fn fibonacci_types_consistent() {
    let air = lower_source(
        r#"
fn fibonacci(n: i32) -> i64 {
    if n <= 1 {
        return n as i64
    }
    return fibonacci(n - 1) + fibonacci(n - 2)
}
"#,
    );
    let printed = print_program(&air);
    let has_mixed = printed
        .lines()
        .any(|line| line.contains("binop") && line.contains("i32") && line.contains("i64"));
    assert!(
        !has_mixed,
        "fibonacci should not have binops mixing i32 and i64 operands, got:\n{}",
        printed
    );
}

// bug fix: Constant folder type preservation

#[test]
fn const_fold_preserves_i32_type() {
    let air = lower_optimized(
        r#"
fn test(n: i32) -> i32 {
    let x: i32 = n + 1
    return x + 2
}
"#,
    );
    let printed = print_program(&air);
    let has_i64_in_binop = printed
        .lines()
        .any(|line| line.contains("binop") && line.contains("i32") && line.contains("i64"));
    assert!(
        !has_i64_in_binop,
        "folded/propagated constants in i32 context should stay i32, got:\n{}",
        printed
    );
}

// bug fix: Constant propagation in loop conditions

#[test]
fn const_prop_does_not_substitute_in_while_condition() {
    let air = lower_optimized(
        r#"
fn sum(n: i32) -> i32 {
    let total: i32 = 0
    let i: i32 = 0
    while i < n {
        total = total + i
        i = i + 1
    }
    return total
}
"#,
    );
    let f = func(&air, "sum");
    let printed = print_program(&air);
    let header_block = f
        .blocks
        .iter()
        .find(|b| matches!(&b.terminator, AirTerminator::Branch { .. }));
    assert!(
        header_block.is_some(),
        "while loop should have a branch block"
    );
    let header = header_block.unwrap();
    let has_const_in_cmp = header.stmts.iter().any(|s| {
        if let AirStmtKind::Assign {
            rvalue: Rvalue::BinaryOp(_, left, right),
            ..
        } = &s.kind
        {
            matches!(left, Operand::Const(_)) && matches!(right, Operand::Copy(_))
        } else {
            false
        }
    });
    assert!(
        !has_const_in_cmp,
        "loop condition should not have constant-propagated loop variable as const operand, got:\n{}",
        printed
    );
}

// bug fix: Function types in type annotations

#[test]
fn parse_function_type_in_return_position() {
    let src = Source::new(
        "<test>",
        r#"
fn make_adder(x: i32) -> fn(i32) -> i32 {
    let add = fn(y: i32) -> i32 { return x + y }
    return add
}
"#,
    );
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let result = Parser::new(tokens, src.clone()).parse();
    assert!(
        result.is_ok(),
        "parsing fn(i32) -> i32 as return type should succeed, got: {:?}",
        result.err()
    );
}

#[test]
fn parse_function_type_no_params() {
    let src = Source::new(
        "<test>",
        r#"
fn get_counter() -> fn() -> i32 {
    let count: i32 = 0
    let f = fn() -> i32 { return count }
    return f
}
"#,
    );
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let result = Parser::new(tokens, src.clone()).parse();
    assert!(
        result.is_ok(),
        "parsing fn() -> i32 as return type should succeed, got: {:?}",
        result.err()
    );
}

#[test]
fn parse_function_type_in_parameter() {
    let src = Source::new(
        "<test>",
        r#"
fn apply(f: fn(i32) -> i32, x: i32) -> i32 {
    return f(x)
}
"#,
    );
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let result = Parser::new(tokens, src.clone()).parse();
    assert!(
        result.is_ok(),
        "parsing fn(i32) -> i32 as parameter type should succeed, got: {:?}",
        result.err()
    );
}

// bug fix: Cast from generic type parameter

#[test]
fn generic_cast_does_not_error_at_sema() {
    let src = Source::new(
        "<test>",
        r#"
fn to_float<T>(x: T) -> f64 {
    return x as f64
}
"#,
    );
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    let result = TypeInference::infer_program(stmts, src);
    assert!(
        result.is_ok(),
        "casting generic T to f64 should not produce a sema error, got: {:?}",
        result.err()
    );
}

#[test]
fn generic_cast_monomorphizes_correctly() {
    let air = lower_source(
        r#"
fn to_float<T>(x: T) -> f64 {
    return x as f64
}
fn caller() -> f64 {
    let v: i32 = 42
    return to_float(v)
}
"#,
    );
    let mut program = air;
    compute_layouts(&mut program);
    let program = monomorphize(program);
    let mono_fn = program
        .functions
        .iter()
        .find(|f| f.name.contains("__mono_to_float"));
    assert!(
        mono_fn.is_some(),
        "to_float should be monomorphized, found: {:?}",
        program
            .functions
            .iter()
            .map(|f| &f.name)
            .collect::<Vec<_>>()
    );
}

// Integration: full pipeline test
#[test]
fn vec2_length_correct_air() {
    let air = lower_source(
        r#"
struct Vec2 {
    x: f64,
    y: f64,
}

fn vec2_length(v: Vec2) -> f64 {
    return v.x * v.x + v.y * v.y
}
"#,
    );
    let f = func(&air, "vec2_length");
    assert!(f.ret_ty == AirType::F64, "return type should be f64");
    for block in &f.blocks {
        for stmt in &block.stmts {
            if let AirStmtKind::Assign {
                rvalue: Rvalue::BinaryOp(_, left, right),
                ..
            } = &stmt.kind
            {
                let left_is_f64 = matches!(left, Operand::Copy(id) if f.locals.iter().any(|l| l.id == *id && l.ty == AirType::F64));
                let right_is_f64 = matches!(right, Operand::Copy(id) if f.locals.iter().any(|l| l.id == *id && l.ty == AirType::F64));
                assert!(
                    left_is_f64 && right_is_f64,
                    "all binop operands in vec2_length should be f64"
                );
            }
        }
    }
}

#[test]
fn sum_loop_block_ids_valid() {
    let air = lower_source(
        r#"
fn sum(n: i32) -> i64 {
    let acc: i64 = 0
    let i: i32 = 0
    while i < n {
        acc = acc + i as i64
        i = i + 1
    }
    return acc
}
"#,
    );
    let f = func(&air, "sum");
    let ids: std::collections::HashSet<u32> = collect_block_ids(f).into_iter().collect();
    for target in collect_branch_targets(f) {
        assert!(
            ids.contains(&target),
            "sum: branch target block{} does not exist, existing blocks: {:?}",
            target,
            ids
        );
    }
}
