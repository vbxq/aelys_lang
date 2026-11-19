"""generated cow probes: programs that alias a managed vec and write through an owner"""

import os
import sys

PROBES = {
    "cow_bare_index": """fn main() -> i64 {
    let mut v: Vec<i64> = vec[7919]
    let w: Vec<i64> = v
    v[0] = 101
    println(v[0])
    println(w[0])
    return 0
}
""",
    "cow_field_under_index": """struct C { f: i64 }
fn main() -> i64 {
    let mut v: Vec<C> = vec[C { f: 7919 }]
    let w: Vec<C> = v
    v[0].f = 101
    println(v[0].f)
    println(w[0].f)
    return 0
}
""",
    "cow_field_depth2": """struct In { g: i64 }
struct C { f: In }
fn main() -> i64 {
    let mut v: Vec<C> = vec[C { f: In { g: 7919 } }]
    let w: Vec<C> = v
    v[0].f.g = 101
    println(v[0].f.g)
    println(w[0].f.g)
    return 0
}
""",
    "cow_index_under_field_under_index": """struct C { a: [i64; 2] }
fn main() -> i64 {
    let mut v: Vec<C> = vec[C { a: [7919, 2] }]
    let w: Vec<C> = v
    v[0].a[0] = 101
    println(v[0].a[0])
    println(w[0].a[0])
    return 0
}
""",
    "cow_compound_index": """fn main() -> i64 {
    let mut v: Vec<i64> = vec[7919]
    let w: Vec<i64> = v
    v[0] += 1
    println(v[0])
    println(w[0])
    return 0
}
""",
    "cow_compound_field": """struct C { f: i64 }
fn main() -> i64 {
    let mut v: Vec<C> = vec[C { f: 7919 }]
    let w: Vec<C> = v
    v[0].f += 1
    println(v[0].f)
    println(w[0].f)
    return 0
}
""",
    "cow_grouping": """struct C { f: i64 }
fn main() -> i64 {
    let mut v: Vec<C> = vec[C { f: 7919 }]
    let w: Vec<C> = v
    (v)[0].f = 101
    println(v[0].f)
    println(w[0].f)
    return 0
}
""",
    "cow_write_in_loop": """struct C { f: i64 }
fn main() -> i64 {
    let mut v: Vec<C> = vec[C { f: 7919 }]
    let w: Vec<C> = v
    let mut i: i64 = 0
    while i < 1 {
        v[0].f = 101
        i = i + 1
    }
    println(v[0].f)
    println(w[0].f)
    return 0
}
""",
    "cow_write_under_if": """struct C { f: i64 }
fn main() -> i64 {
    let mut v: Vec<C> = vec[C { f: 7919 }]
    let w: Vec<C> = v
    if 1 == 1 {
        v[0].f = 101
    }
    println(v[0].f)
    println(w[0].f)
    return 0
}
""",
    "cow_cross_frame_ref": """struct C { f: i64 }
fn put(p: &mut Vec<C>) -> i64 {
    (*p)[0].f = 101
    return 0
}
fn main() -> i64 {
    let mut v: Vec<C> = vec[C { f: 7919 }]
    let w: Vec<C> = v
    let q = put(&mut v)
    println(v[0].f)
    println(w[0].f)
    return 0
}
""",
    "cow_alias_after_view": """fn main() -> i64 {
    let mut v: Vec<i64> = vec[7919]
    let s = v[0..1]
    let w: Vec<i64> = v
    s[0] = 101
    println(v[0])
    println(w[0])
    return 0
}
""",
    "cow_view_then_write": """fn main() -> i64 {
    let mut v: Vec<i64> = vec[7919]
    let w: Vec<i64> = v
    let s = v[0..1]
    s[0] = 101
    println(v[0])
    println(w[0])
    return 0
}
""",
    "cow_nogc_via_view": """nogc fn put(d: &mut [i64]) -> i64 {
    d[0] = 101
    return 0
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[7919]
    let w: Vec<i64> = v
    let q = put(v[0..1])
    println(v[0])
    println(w[0])
    return 0
}
""",
    "cow_index_side_effect": """struct C { f: i64 }
fn idx() -> i64 { return 0 }
fn main() -> i64 {
    let mut v: Vec<C> = vec[C { f: 7919 }]
    let w: Vec<C> = v
    v[idx()].f = 101
    println(v[0].f)
    println(w[0].f)
    return 0
}
""",
    "cow_two_writes_one_detach": """struct C { f: i64 }
fn main() -> i64 {
    let mut v: Vec<C> = vec[C { f: 7919 }]
    let w: Vec<C> = v
    v[0].f = 50
    v[0].f = 101
    println(v[0].f)
    println(w[0].f)
    return 0
}
""",
    "cow_alias_in_branch": """struct C { f: i64 }
fn main() -> i64 {
    let mut v: Vec<C> = vec[C { f: 7919 }]
    let mut w: Vec<C> = vec[C { f: 0 }]
    if 1 == 1 {
        w = v
    }
    v[0].f = 101
    println(v[0].f)
    println(w[0].f)
    return 0
}
""",
    "cow_alias_via_call_arg": """struct C { f: i64 }
fn keep(k: Vec<C>) -> i64 { return k[0].f }
fn main() -> i64 {
    let mut v: Vec<C> = vec[C { f: 7919 }]
    let w: Vec<C> = v
    let r = keep(w)
    v[0].f = 101
    println(v[0].f)
    println(w[0].f)
    return 0
}
""",
    "cow_rhs_reads_other_owner": """struct C { f: i64 }
fn main() -> i64 {
    let mut v: Vec<C> = vec[C { f: 7919 }]
    let w: Vec<C> = v
    v[0].f = w[0].f - 7818
    println(v[0].f)
    println(w[0].f)
    return 0
}
""",
}


def main():
    outdir = os.environ.get("FIX", "")
    if not outdir:
        sys.exit("FIX must name an output directory")
    for name, src in PROBES.items():
        with open(os.path.join(outdir, name + ".aelys"), "w") as f:
            f.write(src)
    print("%d cow probes" % len(PROBES), file=sys.stderr)


if __name__ == "__main__":
    main()
