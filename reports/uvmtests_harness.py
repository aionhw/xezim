#!/usr/bin/env python3
"""
Run the real Accellera uvm-tests suite against xezim and measure the pass rate.

Each test's test.sv is compiled together with the UVM library. The pass
criterion is the standard uvm-tests convention: the log must contain
"UVM TEST PASSED" and must NOT contain "UVM TEST FAILED".

To work around xezim not routing `program`-block initial blocks to the UVM
phaser, `program top` is rewritten to `module top` (the documented xezim UVM
flow uses modules). test.defines / test.plusargs / *.comp.args are honored.
"""
import os, re, subprocess, sys, shutil, tempfile, json
from pathlib import Path

XEZIM   = "/home/tom/prog/git/xezim/xezim/target/release/xezim"
UVM     = "/home/tom/prog/git/xezim/1800.2-2020.3.1"
UVM_SRC = UVM + "/src"
TESTS   = Path("/home/tom/prog/git/uvm-tests/tests")
TIMEOUT = 90

def collect():
    return sorted(TESTS.rglob("test.sv"))

def read_args_file(p):
    if not p.exists(): return []
    out = []
    for ln in p.read_text(errors="replace").splitlines():
        ln = ln.strip()
        if ln and not ln.startswith("#"):
            out.append(ln)
    return out

def rewrite_program(src):
    # Convert a top-level `program` block to `module` so xezim routes run_test()
    # to its native phaser. Match the common uvm-tests forms.
    src = re.sub(r'\bprogram\s+(top|tb)\s*;', r'module \1;', src)
    src = re.sub(r'\bprogram\s+(top|tb)\s*\(', r'module \1(', src)
    src = re.sub(r'\bendprogram\b', r'endmodule', src)
    return src

def run_one(testsv):
    d = testsv.parent
    defines  = read_args_file(d / "test.defines")
    plusargs = read_args_file(d / "test.plusargs")
    # tool-agnostic-ish: grab questa comp args (most complete) if no generic
    comp = read_args_file(d / "test.comp.args")
    if not comp:
        comp = read_args_file(d / "questa.comp.args")
    # plusargs from files use +name form already

    src = testsv.read_text(errors="replace")
    # compile-fail tests: marker comment
    expects_compile_fail = "UVM TEST COMPILE-TIME FAILURE" in src
    expects_run_fail     = "UVM TEST RUN-TIME FAILURE" in src

    rewritten = rewrite_program(src)

    with tempfile.TemporaryDirectory() as td:
        tsv = Path(td) / "test.sv"
        tsv.write_text(rewritten)
        cmd = [XEZIM, "--simulate", "--sv2017", "-s", "top",
               "-I", UVM_SRC, "-I", str(d),
               "-D", "UVM_NO_DPI", "-D", "UVM_REPORT_DISABLE_FILE_LINE"]
        # add common include dirs (parent + any sibling 'common' dirs) for multi-file tests
        for extra in [d.parent, d / "common", d.parent / "common"]:
            if extra.is_dir():
                cmd += ["-I", str(extra)]
        # restore the pre-7fc8187 default (PURE_SV_LRM=off) so UVM run_phase executes
        env = dict(os.environ); env["PURE_SV_LRM"] = "0"
        for df in defines + comp:
            if df.startswith("+define+"):
                cmd += ["-D", df[len("+define+"):]]
            elif df.startswith("+incdir+"):
                pass
            else:
                cmd += [df]
        cmd += [UVM_SRC + "/uvm_pkg.sv", str(tsv)]
        cmd += ["+UVM_TESTNAME=test"] + plusargs
        try:
            p = subprocess.run(cmd, capture_output=True, text=True,
                               timeout=TIMEOUT, cwd=td, env=env)
        except subprocess.TimeoutExpired:
            return ("TIMEOUT", "")
        out = p.stdout + p.stderr
        if expects_compile_fail:
            # we expect xezim to report an error (non-zero / diagnostics)
            if re.search(r'error:|Error:|UVM TEST FAILED', out, re.I) or p.returncode != 0:
                return ("PASS", "compile-fail (error correctly detected)")
            return ("FAIL", "expected compile error but none produced")
        if "UVM TEST PASSED" in out and "UVM TEST FAILED" not in out:
            return ("PASS", "")
        if "UVM TEST FAILED" in out:
            return ("FAIL", first_reason(out))
        # no verdict
        return ("NOVERDICT", first_reason(out))

def first_reason(out):
    for ln in out.splitlines():
        if re.search(r'(Error:|error:|UVM_FATAL|UVM_ERROR|NOCOMP|INVTST|NOTEST|Undefined|cannot|not declared|panic)', ln, re.I):
            return ln.strip()[:140]
    m = re.search(r'Simulation finished at time (\d+)', out)
    if m: return "no verdict; sim finished at time " + m.group(1)
    return "no UVM TEST PASSED/FAILED verdict"

def main():
    from concurrent.futures import ProcessPoolExecutor, as_completed
    import multiprocessing as mp
    tests = collect()
    # exclude the XXfail group (designed to fail the harness) by default
    tests = [t for t in tests if "/XXfail/" not in str(t)]
    results = {}
    counts = {"PASS":0,"FAIL":0,"NOVERDICT":0,"TIMEOUT":0,"SKIP":0}
    nproc = min(8, mp.cpu_count())
    with ProcessPoolExecutor(max_workers=nproc) as ex:
        fut = {ex.submit(run_one, t): t for t in tests}
        done = 0
        for f in as_completed(fut):
            t = fut[f]
            rel = str(t.relative_to(TESTS))
            st, why = f.result()
            results[rel] = (st, why)
            counts[st] = counts.get(st,0)+1
            done += 1
            if done % 50 == 0:
                sys.stderr.write(f"  ...{done}/{len(tests)} done\n"); sys.stderr.flush()
    total = len(tests)
    passed = counts["PASS"]
    print(f"\n===== uvm-tests vs xezim (UVM={UVM}, PURE_SV_LRM=0) =====")
    print(f"Total tests run : {total}")
    for k in ["PASS","FAIL","NOVERDICT","TIMEOUT"]:
        print(f"  {k:9s}: {counts[k]:4d}  ({100.0*counts[k]/total:5.1f}%)")
    print(f"  PASS rate    : {passed}/{total} = {100.0*passed/total:.1f}%")
    print(f"\n----- first 40 non-PASS (category: reason) -----")
    shown=0
    for rel,(st,why) in results.items():
        if st!="PASS":
            print(f"  [{st:8s}] {rel}  ::  {why}")
            shown+=1
            if shown>=40: break
    Path("/tmp/uvmtests_results.json").write_text(json.dumps(results, indent=0))

if __name__ == "__main__":
    main()
