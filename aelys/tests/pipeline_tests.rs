//! Tests for the compilation pipeline.

use aelys_driver::pipeline::{
    CompilerStage, LexerStage, ParserStage, Pipeline, TypeInferenceStage, VMStage,
    standard_pipeline,
};
use aelys_driver::run_source;

#[test]
fn test_standard_pipeline_simple_arithmetic() {
    let mut pipeline = standard_pipeline();
    let result = pipeline.execute_str("test", "1 + 2");
    assert!(result.is_ok());
    assert_eq!(result.unwrap().as_int(), Some(3));
}

#[test]
fn test_standard_pipeline_expression() {
    let mut pipeline = standard_pipeline();
    let result = pipeline.execute_str("test", "(10 + 5) * 2");
    assert!(result.is_ok());
    assert_eq!(result.unwrap().as_int(), Some(30));
}

#[test]
fn test_standard_pipeline_let_binding() {
    let mut pipeline = standard_pipeline();
    let result = pipeline.execute_str("test", "let x = 42\nx");
    assert!(result.is_ok());
    assert_eq!(result.unwrap().as_int(), Some(42));
}

#[test]
fn test_standard_pipeline_function() {
    let mut pipeline = standard_pipeline();
    let result = pipeline.execute_str(
        "test",
        r#"
        fn add(a, b) {
            a + b
        }
        add(3, 4)
    "#,
    );
    assert!(result.is_ok());
    assert_eq!(result.unwrap().as_int(), Some(7));
}

#[test]
fn test_pipeline_caching() {
    let mut pipeline = standard_pipeline();

    // First execution
    let result1 = pipeline.execute_str("test", "1 + 2");
    assert!(result1.is_ok());
    assert_eq!(result1.unwrap().as_int(), Some(3));

    // Cache should have entries now (lexer, parser, type_inference, compiler)
    // VM is not cached because it has side effects
    assert!(pipeline.cache_size() > 0);
    let cache_size_after_first = pipeline.cache_size();

    // Second execution with same source - should use cache
    let result2 = pipeline.execute_str("test", "1 + 2");
    assert!(result2.is_ok());
    assert_eq!(result2.unwrap().as_int(), Some(3));

    // Cache size should be the same (no new entries)
    assert_eq!(pipeline.cache_size(), cache_size_after_first);
}

#[test]
fn test_pipeline_cache_invalidation() {
    let mut pipeline = standard_pipeline();

    // First execution
    let result1 = pipeline.execute_str("test", "1 + 2");
    assert!(result1.is_ok());
    assert_eq!(result1.unwrap().as_int(), Some(3));
    let cache_size_first = pipeline.cache_size();

    // Different source - new cache entries
    let result2 = pipeline.execute_str("test", "10 + 20");
    assert!(result2.is_ok());
    assert_eq!(result2.unwrap().as_int(), Some(30));

    // Cache should have more entries now
    assert!(pipeline.cache_size() > cache_size_first);
}

#[test]
fn test_pipeline_clear_cache() {
    let mut pipeline = standard_pipeline();

    let result = pipeline.execute_str("test", "1 + 2");
    assert!(result.is_ok());
    assert!(pipeline.cache_size() > 0);

    pipeline.clear_cache();
    assert_eq!(pipeline.cache_size(), 0);
}

#[test]
fn test_custom_pipeline() {
    let mut pipeline = Pipeline::new();
    pipeline.add_stage(Box::new(LexerStage));
    pipeline.add_stage(Box::new(ParserStage));
    pipeline.add_stage(Box::new(TypeInferenceStage::new()));
    pipeline.add_stage(Box::new(CompilerStage::new()));
    pipeline.add_stage(Box::new(VMStage::new()));

    let result = pipeline.execute_str("test", "5 * 5");
    assert!(result.is_ok());
    assert_eq!(result.unwrap().as_int(), Some(25));
}

#[test]
fn test_pipeline_syntax_error() {
    let mut pipeline = standard_pipeline();
    let result = pipeline.execute_str("test", "1 +");
    assert!(result.is_err());
}

#[test]
fn test_pipeline_type_error() {
    let mut pipeline = standard_pipeline();
    // This should cause a type error - undefined variable
    let result = pipeline.execute_str("test", "undefined_var");
    assert!(result.is_err());
}

#[test]
fn test_driver_run_source() {
    let result = run_source("1 + 2", "<driver>", None).expect("driver run should succeed");
    assert_eq!(result.as_int(), Some(3));
}
