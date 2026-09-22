""""`d1`, `d2` and `d3` are the three cow detach sites; `e_param` is the clause that makes a `vec` taken by value an effect."""

import os
import pathlib
import sys

ROOT = pathlib.Path(os.environ.get("AELYS_ROOT", pathlib.Path(__file__).resolve().parent.parent))
PLACE = ROOT / "air/src/lower/place.rs"
EXPR = ROOT / "air/src/lower/expr.rs"
EFFECTS = ROOT / "air/src/bir/effects.rs"

SWITCHES = {
    ## the detach in `place_addr`'s index arm: every store at depth through a vec element
    "D1": (
        PLACE,
        "if mode == PlaceMode::Store && Self::roots_a_vec(&object.ty) {\n"
        "                    self.emit_cow_detach(base.ptr, sp);\n                }",
        "if false {\n                    self.emit_cow_detach(base.ptr, sp);\n                }",
    ),
    ## the detach at the formation of a mutable view
    "D2": (
        EXPR,
        "if matches!(result_ty, InferType::Slice { mutable: true, .. })\n"
        "                    && Self::roots_a_vec(&object.ty)\n                {\n"
        "                    self.emit_cow_detach(addr.ptr, sp);\n                }",
        "if false\n                {\n                    self.emit_cow_detach(addr.ptr, sp);\n"
        "                }",
    ),
    ## the detach in `lower_index_assign`: a direct `v[i] = x`
    "D3": (
        EXPR,
        "if Self::roots_a_vec(&object.ty) {\n            self.emit_cow_detach(base.ptr, sp);\n"
        "        }",
        "if false {\n            self.emit_cow_detach(base.ptr, sp);\n        }",
    ),
    "E_PARAM": (
        EFFECTS,
        "        if category(&p.ty, tt) == Category::Managed {",
        "        if false {",
    ),
}


def main():
    for name in sys.argv[1:]:
        path, old, new = SWITCHES[name]
        text = path.read_text()
        count = text.count(old)
        if count != 1:
            sys.exit("%s: its target text occurs %d times, not once" % (name, count))
        path.write_text(text.replace(old, new))
        print("ablated", name)


if __name__ == "__main__":
    main()
