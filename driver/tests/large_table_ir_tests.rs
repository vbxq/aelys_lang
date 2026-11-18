use aelys_driver::compile_file_with_llvm;
use aelys_opt::OptimizationLevel;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn table_program(main: &str) -> String {
    let mut src =
        String::from("struct Row { a: i32, b: i32, c: i64 }\nstatic ROWS: [Row; 1000] = [\n");
    for i in 0..1000 {
        src.push_str(&format!(
            "    Row {{ a: {i}, b: {}, c: {} }},\n",
            i % 7,
            i * 3
        ));
    }
    src.push_str("];\n");
    src.push_str(main);
    src
}

fn emitted_ir(name: &str, source: &str) -> String {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("aelys_driver_{name}_{stamp}.aelys"));
    fs::write(&path, source).unwrap();
    compile_file_with_llvm(&path, OptimizationLevel::None, true).unwrap();
    let ll = path.with_extension("ll");
    let ir = fs::read_to_string(&ll).unwrap();
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&ll);
    ir
}

#[test]
fn a_global_table_is_never_loaded_as_one_aggregate() {
    let mains = [
        "fn main() {\n    let mut s: i64 = 0;\n    for i in 0..1000 {\n        s += ROWS[i].c;\n    }\n    println!(\"{}\", s % 7);\n}\n",
        "fn main() {\n    let mut s: i64 = 0;\n    for r in ROWS {\n        s += r.c;\n    }\n    println!(\"{}\", s % 7);\n}\n",
        "fn main() {\n    let t = ROWS;\n    println!(\"{}\", t[999].c % 7);\n}\n",
    ];
    for (k, main) in mains.iter().enumerate() {
        let ir = emitted_ir(&format!("table{k}"), &table_program(main));
        assert!(
            !ir.contains("load [1000 x"),
            "form {k} loads the whole table"
        );
    }
}
