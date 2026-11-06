#!/usr/bin/env bash
set -u
CLI="${1:-}"
if [ -z "$CLI" ] || [ ! -x "$CLI" ]; then
    echo "memory_growth_sweep: usage: memory_growth_sweep.sh <aelys-cli>" >&2
    exit 2
fi
ROOT=$(cd "$(dirname "$0")/.." && pwd)
exec python3 - "$CLI" "$ROOT" <<'PY'
import os, shutil, statistics, subprocess, sys, tempfile
cli, root = sys.argv[1], sys.argv[2]
tdir = os.path.join(root, "scripts", "memory_growth")
small, large = 100000, 1000000
templates = sorted(f for f in os.listdir(tdir) if f.endswith(".tmpl"))
if not templates:
    print("memory_growth_sweep: no template"); sys.exit(2)

def rss(exe, alloc):
    env = dict(os.environ)
    env.pop("AELYS_ALLOC", None)
    if alloc == "malloc":
        env["AELYS_ALLOC"] = "malloc"
    vals = []
    for k in range(4):
        pid = os.fork()
        if pid == 0:
            fd = os.open(os.devnull, os.O_WRONLY)
            os.dup2(fd, 1)
            os.dup2(fd, 2)
            os.execve(exe, [exe], env)
        _, status, ru = os.wait4(pid, 0)
        if os.waitstatus_to_exitcode(status) != 0:
            return None
        if k:
            vals.append(ru.ru_maxrss)
    return statistics.median(vals)

work = tempfile.mkdtemp(prefix="aelys-memgrowth-")
shutil.copytree(os.path.join(root, "std"), os.path.join(work, "std"))
failures = 0
for t in templates:
    body = open(os.path.join(tdir, t)).read()
    for level in ("0", "2"):
        for rt in ("rc", "leak"):
            exes = {}
            for n in (small, large):
                src = os.path.join(work, f"{t}_{n}.aelys")
                open(src, "w").write(body.replace("NN", str(n)))
                exe = os.path.join(work, f"{t}_{n}_{level}_{rt}")
                c = subprocess.run([cli, "compile", f"-O{level}", "--runtime", rt, "-o", exe, src],
                                   capture_output=True, text=True)
                if c.returncode != 0:
                    print(f"FAIL {t} -O{level} {rt}: does not compile\n{c.stderr.strip()[:300]}")
                    failures += 1
                    break
                exes[n] = exe
            if len(exes) != 2:
                continue
            for alloc in ("immix", "malloc"):
                a, b = rss(exes[small], alloc), rss(exes[large], alloc)
                if a is None or b is None:
                    print(f"FAIL {t} -O{level} {rt}/{alloc}: nonzero exit")
                    failures += 1
                    continue
                ratio = b / a
                line = f"{t} -O{level} {rt}/{alloc}: {a/1024:.1f} MB -> {b/1024:.1f} MB (x{ratio:.2f})"
                if rt == "rc" and ratio > 1.2:
                    print("FAIL grows " + line); failures += 1
                elif rt == "leak" and ratio < 2.0:
                    print("FAIL blind " + line); failures += 1
                else:
                    print("ok " + line)
shutil.rmtree(work, ignore_errors=True)
print(f"memory_growth_sweep: {len(templates)} programs, {failures} failure(s)")
sys.exit(1 if failures else 0)
PY
