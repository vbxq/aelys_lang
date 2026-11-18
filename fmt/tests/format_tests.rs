use std::path::{Path, PathBuf};

use aelys_fmt::{Error, format_source};

fn files(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(d) = pending.pop() {
        for entry in std::fs::read_dir(d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.to_string_lossy().ends_with(suffix) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn comment_count(src: &str) -> usize {
    src.matches("//").count() + src.matches("/*").count()
}

#[test]
fn corpus_formats_idempotently_and_keeps_comments() {
    let corpus = files(&tests_dir().join("corpus"), "");
    assert!(corpus.len() > 100, "corpus has {} files", corpus.len());
    let mut failures = Vec::new();
    for path in &corpus {
        let src = std::fs::read_to_string(path).unwrap();
        match format_source(&src) {
            Ok(out) => {
                assert_eq!(format_source(&out).as_ref(), Ok(&out), "{}", path.display());
                assert_eq!(
                    comment_count(&src),
                    comment_count(&out),
                    "{}",
                    path.display()
                );
                assert!(out.lines().all(|l| !l.ends_with(' ')), "{}", path.display());
            }
            Err(e) => failures.push(format!("{}: {e}", path.display())),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn golden_outputs_match_and_are_fixed_points() {
    let inputs = files(&tests_dir().join("golden"), ".in.aelys");
    assert_eq!(inputs.len(), 10);
    for input in inputs {
        let expected_path =
            PathBuf::from(input.to_string_lossy().replace(".in.aelys", ".out.aelys"));
        let src = std::fs::read_to_string(&input).unwrap();
        let expected = std::fs::read_to_string(&expected_path).unwrap();
        let out = format_source(&src).unwrap_or_else(|e| panic!("{}: {e}", input.display()));
        assert_eq!(out, expected, "{}", input.display());
        assert_eq!(
            format_source(&expected).as_ref(),
            Ok(&expected),
            "{}",
            expected_path.display()
        );
    }
}

const JOINED: [&str; 22] = [
    "::", "->", "=>", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=", "%=", "^=", "&=",
    "|=", "<<", ">>", "..", "/*", "*/",
];

fn insertion_points(src: &str) -> Vec<usize> {
    let b = src.as_bytes();
    let mut points = Vec::new();
    let mut in_str = false;
    let mut in_comment = false;
    let class = |c: u8| {
        if c.is_ascii_whitespace() {
            0
        } else if c.is_ascii_alphanumeric() || c == b'_' {
            1
        } else {
            2
        }
    };
    for i in 1..b.len() {
        let (p, c) = (b[i - 1], b[i]);
        if in_comment {
            in_comment = p != b'\n';
            continue;
        }
        if p == b'"' {
            in_str = !in_str;
        }
        if in_str {
            continue;
        }
        if p == b'/' && c == b'/' {
            in_comment = true;
            continue;
        }
        let split = class(p) != class(c) || class(p) == 2 && !JOINED.contains(&&src[i - 1..=i]);
        if split && class(p) != 0 && p != b'\'' && c != b'\'' {
            points.push(i);
        }
    }
    points
}

#[test]
fn a_comment_in_every_gap_survives() {
    let src = std::fs::read_to_string(tests_dir().join("corpus/adversarial/grammar_cover.aelys"))
        .unwrap();
    let points = insertion_points(&src);
    assert!(points.len() > 300, "{} insertion points", points.len());
    for (n, &at) in points.iter().enumerate() {
        for comment in [format!(" /* c{n} */ "), format!(" // c{n}\n")] {
            let edited = format!("{}{comment}{}", &src[..at], &src[at..]);
            match format_source(&edited) {
                Ok(out) => assert!(out.contains(comment.trim()), "lost {comment:?} at {at}"),
                Err(e) => panic!("{e} with {comment:?} inserted at byte {at}:\n{edited}"),
            }
        }
    }
}

#[test]
fn invalid_input_is_refused_with_a_position() {
    for (src, line) in [
        ("fn main() {\n    let x = ;\n}\n", 2),
        ("fn main() {\n    foo(1, 2\n}\n", 3),
        ("struct S {\n    a: i64\n", 3),
        ("fn f() { \"unterminated }\n", 1),
        ("fn main() {\n    let a = 1\n    let b = 2;\n}\n", 3),
    ] {
        match format_source(src) {
            Err(e @ Error::Syntax { .. }) => assert_eq!(e.line_col(src).0, line, "{src:?}: {e}"),
            other => panic!("{src:?} gave {other:?}"),
        }
    }
}

#[test]
fn unmigrated_corpus_files_are_refused() {
    let refused = files(&tests_dir().join("invalid"), "");
    assert_eq!(refused.len(), 3);
    for path in refused {
        let src = std::fs::read_to_string(&path).unwrap();
        assert!(
            matches!(format_source(&src), Err(Error::Syntax { .. })),
            "{}",
            path.display()
        );
    }
}

#[test]
fn deep_nesting_does_not_overflow_the_stack() {
    let handle = std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(|| {
            let calls = format!(
                "fn f() {{\n    {}1{};\n}}\n",
                "g(".repeat(200),
                ")".repeat(200)
            );
            let blocks = format!(
                "fn f() {{\n    {}1{}\n}}\n",
                "{ ".repeat(150),
                " }".repeat(150)
            );
            let too_deep = format!(
                "fn f() {{\n    {}1{};\n}}\n",
                "(".repeat(5000),
                ")".repeat(5000)
            );
            assert!(format_source(&calls).is_ok());
            assert!(format_source(&blocks).is_ok());
            assert!(matches!(
                format_source(&too_deep),
                Err(Error::Syntax { .. })
            ));
        })
        .unwrap();
    handle.join().unwrap();
}

#[test]
fn layout_is_canonical_whatever_the_input_layout() {
    let compact = "fn f(a: i64) -> i64 { if a > 0 { a } else { 0 - a } }\n";
    let sprawling = "fn f(\n  a: i64,\n)\n-> i64\n{\n\n\n  if a > 0\n  {\n    a\n  }\n  else { 0 -\n a }\n}\n\n\n";
    assert_eq!(format_source(compact), format_source(sprawling));
}
