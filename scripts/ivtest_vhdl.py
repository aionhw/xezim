#!/usr/bin/env python3
"""Run Icarus Verilog's VHDL regression (ivtest regress-vhdl.list) with xezim.

ivtest is GPL and not part of xezim: point this at a checkout of
https://github.com/steveicarus/ivtest (or iverilog's ivtest directory).
Each test is a Verilog testbench (<dir>/<name>.v) instantiating VHDL
entities. A test passes when xezim exits 0 and prints PASSED and no FAILED.
Tests that need Icarus VPI modules (-mvhdl_sys, -mvhdl_textio) are skipped.
Logs go to $IVT_OUT (default /tmp/ivt_out).

usage: ivtest_vhdl.py <ivtest dir> <xezim binary> [name regex] [-j N]
"""
import os, re, subprocess, sys, concurrent.futures

ROOT, XEZIM = sys.argv[1], sys.argv[2]
FILT = sys.argv[3] if len(sys.argv) > 3 and not sys.argv[3].startswith("-") else None
JOBS = 4
if "-j" in sys.argv:
    JOBS = int(sys.argv[sys.argv.index("-j") + 1])
OUT = os.environ.get("IVT_OUT", "/tmp/ivt_out")
os.makedirs(OUT, exist_ok=True)
TIMEOUT = int(os.environ.get("IVT_TIMEOUT", "120"))


def tests():
    seen = set()
    for line in open(os.path.join(ROOT, "regress-vhdl.list")):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        parts = line.split()
        if len(parts) < 3:
            continue
        name, spec, d = parts[0], parts[1], parts[2]
        if name in seen:
            continue
        seen.add(name)
        yield name, spec.split(","), d


def run(t):
    name, spec, d = t
    typ = spec[0]
    files, pre = [], []
    for a in spec[1:]:
        if not a:
            continue
        if a.startswith("-m"):
            return name, "SKIP", "icarus vpi " + a
        if a.startswith("-g"):
            continue
        if a.startswith("-f"):
            for l in open(os.path.join(ROOT, a[2:])):
                l = l.strip()
                m = re.match(r"\+timescale\+(\S+)", l)
                if m:
                    ts = os.path.join(OUT, name + "_ts.v")
                    open(ts, "w").write("`timescale %s\n" % m.group(1))
                    pre.append(ts)
                elif l:
                    files.append(l)
            continue
        files.append(a)
    tb = None
    for ext in (".v", ".sv"):
        p = os.path.join(d, name + ext)
        if os.path.isfile(os.path.join(ROOT, p)):
            tb = p
    if tb is None and not any(f.endswith(".v") for f in files):
        return name, "SKIP", "no testbench"
    srcs = pre + ([tb] if tb else []) + files
    try:
        r = subprocess.run([XEZIM] + srcs, capture_output=True, timeout=TIMEOUT, cwd=ROOT)
        out = (r.stdout + r.stderr).decode("utf-8", "replace")
        rc = r.returncode
    except subprocess.TimeoutExpired:
        out, rc = "<TIMEOUT>", None
    open(os.path.join(OUT, name + ".log"), "w").write(out)
    if typ != "normal":
        return name, "SKIP", "type " + typ
    ok = rc == 0 and "PASSED" in out and "FAILED" not in out and "FAIL" not in out.replace("FAILED", "")
    if ok:
        return name, "PASS", ""
    lines = [l for l in out.splitlines() if l.strip() and not l.startswith("===") and not l.startswith("git ")]
    why = "timeout" if rc is None else (lines[0][:150] if lines else "rc=%s" % rc)
    for l in lines:
        if "error" in l.lower():
            why = l[:150]
            break
    return name, "FAIL", why


ts = [t for t in tests() if FILT is None or re.search(FILT, t[0])]
res = []
with concurrent.futures.ThreadPoolExecutor(JOBS) as ex:
    for r in ex.map(run, ts):
        res.append(r)
        print("%-28s %s %s" % r, flush=True)
n = {k: sum(1 for r in res if r[1] == k) for k in ("PASS", "FAIL", "SKIP")}
print("PASS %(PASS)d FAIL %(FAIL)d SKIP %(SKIP)d" % n)
