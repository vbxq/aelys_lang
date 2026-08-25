
import os
import re
import sys

ESCAPES = {"n": "\n", "t": "\t", "r": "\r", "0": "\0", "\\": "\\", '"': '"', "'": "'"}

DECLARES = re.compile(r"^\s*(nogc\s+)?(fn|struct|enum|let)\b", re.M)


def read_literal(src, i):
    if src[i] != '"':
        raise ValueError("not a string literal at %d" % i)
    i += 1
    out = []
    while src[i] != '"':
        c = src[i]
        if c != "\\":
            out.append(c)
            i += 1
            continue
        n = src[i + 1]
        if n == "\n":
            i += 2
            while src[i] in " \t":
                i += 1
            continue
        if n == "x":
            out.append(chr(int(src[i + 2 : i + 4], 16)))
            i += 4
            continue
        out.append(ESCAPES[n])
        i += 2
    return "".join(out), i + 1


def rows(text):
    out = {}
    for m in re.finditer(r"^const ([A-Z][A-Z0-9_]*): &str = ", text, re.M):
        try:
            value, _ = read_literal(text, m.end())
        except (ValueError, IndexError, KeyError):
            continue
        out[m.group(1)] = value
    return out


def is_program(src):
    return bool(DECLARES.search(src)) and "fn main(" in src


def main():
    files = [a for a in sys.argv[1:] if not a.startswith("--")]
    listing = "--unextracted" in sys.argv
    outdir = os.environ.get("FIX", "")
    if not listing and not outdir:
        sys.exit("FIX must name an output directory")
    total = kept = 0
    for path in files:
        text = open(path, encoding="utf-8").read()
        stem = os.path.basename(path).replace("_tests.rs", "").replace(".rs", "")
        for name, src in rows(text).items():
            total += 1
            if not is_program(src):
                if listing:
                    print("FRAGMENT %s %s" % (stem, name))
                continue
            kept += 1
            if not listing:
                with open(os.path.join(outdir, "%s__%s.aelys" % (stem, name)), "w") as f:
                    f.write(src)
    print("rows=%d fragments=%d" % (kept, total - kept), file=sys.stderr)


if __name__ == "__main__":
    main()
