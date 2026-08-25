"""Ablate one clause of the effect walk, in place, so the guard can measure what each clause holds.

One switch per clause. A switch whose anchor is not in the source is *inapplicable*, not a no-op:
`--applicable` lists the ids the current source admits, and the driver asserts that list rather
than discovering it, because an ablation that silently stops applying reads as a clean matrix.

The caller restores the file and rebuilds; leaving an ablated binary in target/release is how a
later reading gets attributed to the shipped rule.
"""

import os
import sys

NODE_RULE_PRE = """    if category(&expr.ty, tt) == Category::Managed {
        set.insert(Effect::Managed);
        let name = match &expr.kind {
            TypedExprKind::Identifier(n) => format!("the managed value `{}`", n),
            _ => "a managed value".to_string(),
        };
        w.record(RANK_TYPE, expr.span, name);
    }
"""

NODE_RULE_POST = """    if pos == Pos::Value && category(&expr.ty, tt) == Category::Managed {
        set.insert(Effect::Managed);
        let name = match &expr.kind {
            TypedExprKind::Identifier(n) => format!("the managed value `{}`", n),
            _ => "a managed value".to_string(),
        };
        w.record(RANK_TYPE, expr.span, name);
    }
"""

STORE_CLAUSE_INDEX = """            if peels_to_vec(&object.ty) || chain_crosses_vec(object) {
                set.insert(Effect::Managed);
                w.record(
                    RANK_INTRINSIC,
                    expr.span,
                    "a store into a buffer that may be shared".to_string(),
                );
            }
"""

STORE_CLAUSE_FIELD = """            if chain_crosses_vec(object) {
                set.insert(Effect::Managed);
                w.record(
                    RANK_INTRINSIC,
                    expr.span,
                    "a store into a buffer that may be shared".to_string(),
                );
            }
"""

DEAD_POS = "    let _ = pos;\n"

# reports rather than silently treating as a no-op
SWITCHES = {
    "K9PRE": [(NODE_RULE_PRE, "    let _ = (tt, &expr.ty);\n")],
    "K9": [(NODE_RULE_POST, "    let _ = (tt, &expr.ty, pos);\n")],
    "KP": [
        (
            "    if pos == Pos::Value && category(&expr.ty, tt) == Category::Managed {\n",
            "    if category(&expr.ty, tt) == Category::Managed {\n" + DEAD_POS,
        )
    ],
    "K1": [
        (
            "TypedExprKind::Member { object, .. } => walk_expr(object, tt, set, w, Pos::Base),",
            "TypedExprKind::Member { object, .. } => walk_expr(object, tt, set, w, Pos::Value),",
        )
    ],
    "K2": [
        (
            "            walk_expr(object, tt, set, w, base_unless_inside_a_buffer(object));\n"
            "            walk_expr(index, tt, set, w, Pos::Value);\n",
            "            walk_expr(object, tt, set, w, Pos::Value);\n"
            "            walk_expr(index, tt, set, w, Pos::Value);\n",
        ),
        (
            "            walk_expr(object, tt, set, w, base_unless_inside_a_buffer(object));\n"
            "            walk_expr(range, tt, set, w, Pos::Value);\n",
            "            walk_expr(object, tt, set, w, Pos::Value);\n"
            "            walk_expr(range, tt, set, w, Pos::Value);\n",
        ),
    ],
    "K3": [
        (
            "            walk_expr(object, tt, set, w, Pos::Base);\n"
            "            walk_expr(value, tt, set, w, Pos::Value);\n",
            "            walk_expr(object, tt, set, w, Pos::Value);\n"
            "            walk_expr(value, tt, set, w, Pos::Value);\n",
        )
    ],
    "K4": [
        (
            "            walk_expr(operand, tt, set, w, base_unless_inside_a_buffer(operand))\n",
            "            walk_expr(operand, tt, set, w, Pos::Value)\n",
        )
    ],
    "K5": [(STORE_CLAUSE_INDEX, ""), (STORE_CLAUSE_FIELD, "")],
    "K6": [
        (
            "TypedExprKind::Grouping(inner) => walk_expr(inner, tt, set, w, pos),",
            "TypedExprKind::Grouping(inner) => walk_expr(inner, tt, set, w, Pos::Value),",
        )
    ],
    # the chain_crosses_vec condition on edges 2, 3 and 6, which is fence hygiene rather than
    "K7": [
        (
            "fn base_unless_inside_a_buffer(object: &TypedExpr) -> Pos {\n"
            "    if chain_crosses_vec(object) {\n"
            "        Pos::Value\n"
            "    } else {\n"
            "        Pos::Base\n"
            "    }\n"
            "}\n",
            "fn base_unless_inside_a_buffer(object: &TypedExpr) -> Pos {\n"
            "    let _ = object;\n"
            "    Pos::Base\n"
            "}\n",
        )
    ],
    "KI": [
        (
            "            walk_expr(object, tt, set, w, Pos::Base);\n"
            "            walk_expr(index, tt, set, w, Pos::Value);\n"
            "            walk_expr(value, tt, set, w, Pos::Value);\n",
            "            walk_expr(object, tt, set, w, Pos::Value);\n"
            "            walk_expr(index, tt, set, w, Pos::Value);\n"
            "            walk_expr(value, tt, set, w, Pos::Value);\n",
        )
    ],
}

SWITCHES["K9K5"] = SWITCHES["K9"] + SWITCHES["K5"]


def main():
    path = os.environ["AELYS_EFFECTS"]
    src = open(path).read()

    if "--applicable" in sys.argv:
        for name in sorted(SWITCHES):
            if all(src.count(needle) == 1 for needle, _ in SWITCHES[name]):
                print(name)
        return 0

    name = os.environ["AELYS_ABLATION"]
    if name not in SWITCHES:
        sys.stderr.write("unknown ablation %s; known: %s\n" % (name, " ".join(sorted(SWITCHES))))
        return 2
    for needle, replacement in SWITCHES[name]:
        if src.count(needle) != 1:
            sys.stderr.write(
                "%s's clause is not where this guard expects it (%d occurrences); update the patch\n"
                % (name, src.count(needle))
            )
            return 1
        src = src.replace(needle, replacement)
    open(path, "w").write(src)
    return 0


if __name__ == "__main__":
    sys.exit(main())
