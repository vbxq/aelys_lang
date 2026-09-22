""""generating rather than copying is what makes the guard's measured set equal the corpus's row set by construction, and what makes fixture drift impossible rather than merely unobserved."""

import os
import re
import sys

ESCAPES = {"n": "\n", "t": "\t", "r": "\r", "0": "\0", "\\": "\\", '"': '"', "'": "'"}

GROUPS = ("GROUP_A", "GROUP_B", "GROUP_C")


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


def consts(text):
    out = {}
    for m in re.finditer(r'const ([A-Z][A-Z0-9_]*): &str = ', text):
        value, _ = read_literal(text, m.end())
        out[m.group(1)] = value
    return out


def group_rows(text, group, named):
    start = text.index("const %s: &[Row] = &[" % group)
    block = text[start : text.index("\n];", start)]
    rows = []
    i = 0
    while True:
        k = block.find('id: "', i)
        if k < 0:
            return rows
        rid, j = read_literal(block, k + 4)
        j = block.index("src: ", j) + 5
        if block[j] == '"':
            body, i = read_literal(block, j)
        else:
            ## silently drops those rows reads as a clean sweep over a smaller corpus
            name = re.match(r"[A-Z][A-Z0-9_]*", block[j:])
            if not name:
                raise ValueError("%s: src is neither a literal nor a const name" % rid)
            body = named[name.group(0)]
            i = j + name.end()
        rows.append((rid, body))


def main():
    corpus = os.environ["CORPUS"]
    out = os.environ["FIX"]
    text = open(corpus).read()
    named = consts(text)
    total = 0
    counts = []
    for group in GROUPS:
        rows = group_rows(text, group, named)
        if not rows:
            sys.stderr.write("no rows parsed from %s\n" % group)
            return 1
        for rid, body in rows:
            with open(os.path.join(out, rid + ".aelys"), "w") as fh:
                fh.write(body)
        counts.append("%s %d" % (group[-1], len(rows)))
        total += len(rows)
    print("   %d fixtures (%s)" % (total, ", ".join(counts)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
