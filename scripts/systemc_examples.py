#!/usr/bin/env python3
"""Run the Accellera SystemC 2.3.4 examples under xezim and compare each with
the same example built against the SystemC reference kernel (IEEE 1666).

Prints one line per example: PASS, DIFF (first differing line), FAIL (first
error) or TIMEOUT. Reference binaries and outputs are cached in the work
directory; xezim's outputs land there too (`out/<name>.out`, `.err`).

The examples come from the SystemC source release (`examples/` of
https://github.com/accellera-official/systemc/archive/refs/tags/2.3.4.tar.gz);
the reference kernel is the installed SystemC (`g++ ... -lsystemc`).

usage: systemc_examples.py --examples DIR --work DIR [--xezim PATH]
                           [--timeout S] [name ...]
"""
import argparse, os, re, subprocess, sys

EX = REF = OUT = None

def tlm(name, objs, top=None):
    d = f"tlm/{name}"
    srcs = []
    for o in objs:
        for sub in (f"{d}/src", "tlm/common/src"):
            if os.path.exists(os.path.join(EX, sub, o + ".cpp")):
                srcs.append(f"{sub}/{o}.cpp")
                break
        else:
            raise SystemExit(f"{name}: no source for {o}")
    return dict(dir=f"{d}/src", srcs=srcs, inc=[f"{d}/include", "tlm/common/include"])

def wild(d):
    return sorted(f"{d}/{f}" for f in os.listdir(os.path.join(EX, d)) if f.endswith(".cpp"))

def ex(d, files=None, inc=()):
    return dict(dir=d, srcs=[f"{d}/{f}" for f in files] if files else wild(d), inc=list(inc))

def manifest():
    """Each example: the directory it runs in, its sources and include
    directories (from its Makefile)."""
    C = "initiator_top memory report select_initiator traffic_generator".split()
    return {
        "dpipe": ex("sysc/2.1/dpipe", ["main.cpp"]),
        "forkjoin": ex("sysc/2.1/forkjoin", ["forkjoin.cpp"]),
        "reset_signal_is": ex("sysc/2.1/reset_signal_is", ["reset_signal_is.cpp"]),
        "sc_export": ex("sysc/2.1/sc_export", ["main.cpp"]),
        "sc_report": ex("sysc/2.1/sc_report", ["main.cpp"]),
        "scx_barrier": ex("sysc/2.1/scx_barrier", ["main.cpp"]),
        "scx_mutex_w_policy": ex("sysc/2.1/scx_mutex_w_policy", ["scx_mutex_w_policy.cpp"]),
        "specialized_signals": ex("sysc/2.1/specialized_signals"),
        "sc_rvd": ex("sysc/2.3/sc_rvd", ["main.cpp"], ["sysc/2.3/include"]),
        "sc_ttd": ex("sysc/2.3/sc_ttd", ["main.cpp"], ["sysc/2.3/include"]),
        "simple_async": ex("sysc/2.3/simple_async", ["main.cpp"]),
        "fft_flpt": ex("sysc/fft/fft_flpt"),
        "fft_fxpt": ex("sysc/fft/fft_fxpt"),
        "fir": ex("sysc/fir", ["stimulus.cpp", "display.cpp", "fir.cpp", "main.cpp"]),
        "fir_rtl": ex("sysc/fir", ["stimulus.cpp", "display.cpp", "fir_fsm.cpp", "fir_data.cpp", "main_rtl.cpp"]),
        "pipe": ex("sysc/pipe"),
        "pkt_switch": ex("sysc/pkt_switch"),
        "risc_cpu": ex("sysc/risc_cpu"),
        "rsa": ex("sysc/rsa", ["rsa.cpp"]),
        "simple_bus": ex("sysc/simple_bus"),
        "simple_fifo": ex("sysc/simple_fifo"),
        "simple_perf": ex("sysc/simple_perf"),
        "at_1_phase": tlm("at_1_phase", ["at_1_phase", "at_1_phase_top", "at_target_1_phase"] + C),
        "at_2_phase": tlm("at_2_phase", ["at_2_phase", "at_2_phase_top", "at_target_2_phase"] + C),
        "at_4_phase": tlm("at_4_phase", ["at_4_phase", "at_4_phase_top", "at_target_4_phase"] + C),
        "at_extension_optional": tlm("at_extension_optional", ["at_extension_optional", "at_extension_optional_top", "at_target_2_phase", "at_target_4_phase", "extension_initiator_id"] + C),
        "at_mixed_targets": tlm("at_mixed_targets", ["at_mixed_targets", "at_mixed_targets_top", "at_target_1_phase", "at_target_2_phase", "at_target_4_phase"] + C),
        "at_ooo": tlm("at_ooo", ["at_ooo", "at_ooo_top", "at_target_2_phase", "at_target_ooo_2_phase"] + C),
        "lt": tlm("lt", "lt lt_top lt_initiator at_target_1_phase lt_target initiator_top memory report traffic_generator".split()),
        "lt_dmi": tlm("lt_dmi", "lt_dmi lt_dmi_top initiator_top dmi_memory lt_dmi_initiator lt_dmi_target memory report traffic_generator".split()),
        "lt_extension_mandatory": tlm("lt_extension_mandatory", "lt_extension_mandatory lt_extension_mandatory_top lt_initiator_extension_mandatory lt_target_extension_mandatory extension_initiator_id report".split()),
        "lt_mixed_endian": tlm("lt_mixed_endian", "lt lt_top initiator_top me_traffic_generator at_target_1_phase at_target_4_phase lt_initiator memory report".split()),
        "lt_temporal_decouple": tlm("lt_temporal_decouple", "lt_temporal_decouple lt_temporal_decouple_top initiator_top td_initiator_top lt_initiator lt_synch_target lt_target lt_td_initiator memory report traffic_generator".split()),
    }

def strip_banner(text):
    lines = text.split("\n")
    for i, l in enumerate(lines):
        if "ALL RIGHTS RESERVED" in l:
            rest = lines[i + 1:]
            while rest and rest[0].strip() == "":
                rest = rest[1:]
            return "\n".join(rest)
    return text

def reference(name, e, timeout):
    os.makedirs(REF, exist_ok=True)
    exe = os.path.join(REF, name)
    out = os.path.join(REF, name + ".out")
    if not os.path.exists(exe):
        cmd = ["g++", "-std=c++17", "-O1", "-w", "-o", exe] + [os.path.join(EX, s) for s in e["srcs"]]
        cmd += [f"-I{os.path.join(EX, i)}" for i in e["inc"]] + ["-lsystemc"]
        r = subprocess.run(cmd, capture_output=True, text=True)
        if r.returncode:
            raise RuntimeError("reference build failed: " + r.stderr[:500])
    if not os.path.exists(out):
        r = subprocess.run([exe], cwd=os.path.join(EX, e["dir"]), capture_output=True, text=True, timeout=timeout)
        with open(out, "w") as f:
            f.write(strip_banner(r.stdout) + f"\n[exit {r.returncode}]\n")
    return open(out).read()

XEZ_NOISE = re.compile(r"^(=== xezim|git |Simulation finished at time|\[xezim\])")

def run_xezim(xezim, name, e, timeout):
    cmd = [xezim, "--no-cache"] + [f"-I{os.path.join(EX, i)}" for i in e["inc"]]
    cmd += [os.path.join(EX, s) for s in e["srcs"]]
    try:
        r = subprocess.run(cmd, cwd=os.path.join(EX, e["dir"]), capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return None, "timeout"
    out = "\n".join(l for l in r.stdout.split("\n") if not XEZ_NOISE.match(l)).lstrip("\n")
    return out.rstrip() + f"\n[exit {r.returncode}]\n", r.stderr

def norm(x):
    return re.sub(r"\s*\n\[exit", "\n[exit", x.strip())

def first_error(stderr, stdout):
    for l in (stderr + "\n" + (stdout or "")).split("\n"):
        if "error" in l.lower():
            return l.strip()[:200]
    return (stderr.strip().split("\n") or [""])[0][:200]

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--examples", required=True, help="the examples/ directory of the SystemC sources")
    ap.add_argument("--work", required=True, help="cache and output directory")
    ap.add_argument("--xezim", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "../target/debug/xezim"))
    ap.add_argument("--timeout", type=int, default=300)
    ap.add_argument("names", nargs="*")
    a = ap.parse_args()
    global EX, REF, OUT
    EX = os.path.abspath(a.examples)
    REF = os.path.join(os.path.abspath(a.work), "ref")
    OUT = os.path.join(os.path.abspath(a.work), "out")
    os.makedirs(OUT, exist_ok=True)
    examples = manifest()
    names = a.names or list(examples)
    npass = 0
    for n in names:
        e = examples[n]
        try:
            ref = reference(n, e, a.timeout)
        except Exception as x:
            print(f"{n:24} REFERR {x}")
            continue
        got, err = run_xezim(a.xezim, n, e, a.timeout)
        with open(os.path.join(OUT, n + ".out"), "w") as f:
            f.write(got or "")
        with open(os.path.join(OUT, n + ".err"), "w") as f:
            f.write(err or "")
        if got is None:
            print(f"{n:24} TIMEOUT")
        elif norm(got) == norm(ref):
            npass += 1
            print(f"{n:24} PASS")
        elif got.count("\n") <= 2:
            print(f"{n:24} FAIL   {first_error(err, got)}")
        else:
            rl, gl = norm(ref).split("\n"), norm(got).split("\n")
            k = next((i for i, (x, y) in enumerate(zip(rl, gl)) if x != y), min(len(rl), len(gl)))
            print(f"{n:24} DIFF   line {k + 1}: ref {rl[k][:70]!r} got {gl[k][:70] if k < len(gl) else '<eof>'!r}")
        sys.stdout.flush()
    print(f"-- {npass}/{len(names)} pass")

main()
