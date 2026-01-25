use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::Source;
use aelys_syntax::Span;

#[test]
fn test_compile_error_format() {
    let source = Source::new("test.aelys", "let x = 10\nprint(username)\nlet y = 20");
    let error = CompileError::new(
        CompileErrorKind::UndefinedVariable("username".to_string()),
        Span::new(17, 25, 2, 7),
        source,
    );

    let output = format!("{}", error);

    assert!(output.contains("error"), "Should contain 'error'");
    assert!(
        output.contains("undefined variable"),
        "Should describe the error"
    );
    assert!(output.contains("username"), "Should contain variable name");
    assert!(output.contains("test.aelys:2:7"), "Should contain location");
    assert!(
        output.contains("print(username)"),
        "Should contain source line"
    );
    assert!(output.contains("^^^^^^^^"), "Should have caret annotation");
}

#[test]
fn test_runtime_error_with_stack_trace() {
    use aelys_common::error::{RuntimeError, RuntimeErrorKind, StackFrame};

    let source = Source::new("test.aelys", "fn foo() {\n    1 / 0\n}\nfoo()");
    let error = RuntimeError::new(
        RuntimeErrorKind::DivisionByZero,
        vec![
            StackFrame {
                function_name: Some("foo".to_string()),
                line: 2,
                column: 5,
            },
            StackFrame {
                function_name: None,
                line: 4,
                column: 1,
            },
        ],
        source,
    );

    let output = format!("{}", error);

    assert!(output.contains("division by zero"), "Should describe error");
    assert!(output.contains("stack trace"), "Should have stack trace");
    assert!(output.contains("foo"), "Should show function name");
}

#[test]
fn error_formatting_is_stable() {
    let err = aelys_common::error::CompileError::new(
        aelys_common::error::CompileErrorKind::InvalidCharacter('?'),
        aelys_syntax::Span::dummy(),
        aelys_syntax::Source::new("<test>", ""),
    );
    let msg = format!("{}", err);
    assert!(msg.contains("invalid character"));
}
