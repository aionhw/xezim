#!/usr/bin/env python3
r"""Compare frozen xezim builds with interleaved, CPU-pinned simulation runs.

Example:
  python3 scripts/bench-compiler-builds.py --work-dir /tmp/compiler-bench \
    --candidate thin=target/release --candidate fat=target/flags-fat/release \
    --candidate pgo=target/opt-pgo-thin/release --cpu 0

Run after builds and tests finish. Each candidate directory must contain xezim
and xezim-bench from the same sources/features. The first candidate is the
baseline. All five host workloads verify results against Rust mirrors; the
four SystemVerilog cases require PASS and identical checksums/work counts.
Small-case golden checksums are checked before timing. SRAM model cases add
equivalent flat/deep hierarchical memory and banked NBA writes. Their memory
score is reported separately from the general controls. No timing threshold
is used as a correctness test. Results describe these measured workloads only.

--external-case accepts a JSON object or list with name, marker, top, sources,
include_dirs, defines and args. Paths are relative to the descriptor. Optional
extra_inputs are hashed without compilation. preflight_/benchmark_ defines,
args and checksum fields select short checks and full runs independently;
golden_checksum supplies a common expected checksum when both use one size.
"""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import resource
import signal
import statistics
import subprocess
import time


HOST_CASES = ["comb_alu", "wide_vec", "mem_array", "event_ctrl", "class_queue"]
SV_CASES = [
    ("b1_comb_mesh", "bench_comb", "0000005ffa263660"),
    ("b2_pipeline", "bench_pipe", "000003ec5a89fc8a"),
    ("b3_memory", "bench_mem", "000026ca96f1e2d7"),
    ("b4_oop_tb", "bench_oop", "0000040d0e280e3d"),
]
MEMORY_CASES = [
    ("hier_memory_flat", "hier_memory", ["MEMORY_FLAT"], "2be5bc5bd87d06a6"),
    ("hier_memory_deep", "hier_memory", [], "2be5bc5bd87d06a6"),
    ("banked_nba_memory", "banked_nba_memory", [], "f77d2184671edf01"),
]
# Independently mirrored in Python; banked_nba_memory also checks these in
# the SV fixture. Flat and deep hierarchy use the same memory operations.
MEMORY_GOLDENS = {
    "hier_memory": {"small": "2be5bc5bd87d06a6", "default": "87cf62d3b69873b0", "large": "5856063627c12844"},
    "banked_nba_memory": {"small": "f77d2184671edf01", "default": "9225d1e80fa2baa1", "large": "fbf772066a8b73fd"},
}


def candidate_arg(value):
    label, sep, directory = value.partition("=")
    if not sep or not re.fullmatch(r"[A-Za-z0-9_-]+", label):
        raise argparse.ArgumentTypeError("expected LABEL=/directory/containing/binaries")
    path = Path(directory).resolve()
    for name in ["xezim", "xezim-bench"]:
        if not (path / name).is_file() or not os.access(path / name, os.X_OK):
            raise argparse.ArgumentTypeError(f"not executable: {path / name}")
    return label, path


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def run(command, path, cpu, timeout, env):
    def child_setup():
        os.sched_setaffinity(0, {cpu})
        resource.setrlimit(resource.RLIMIT_CORE, (0, 0))

    started = time.monotonic()
    with path.open("wb") as output:
        child = subprocess.Popen(command, stdout=output, stderr=subprocess.STDOUT,
                                 env=env, preexec_fn=child_setup, start_new_session=True)
        while True:
            pid, status, usage = os.wait4(child.pid, os.WNOHANG)
            if pid:
                break
            if time.monotonic() - started > timeout:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                _, status, usage = os.wait4(child.pid, 0)
                child.returncode = os.waitstatus_to_exitcode(status)
                raise RuntimeError(f"timeout after {timeout}s: {path}")
            time.sleep(0.01)
        child.returncode = os.waitstatus_to_exitcode(status)
    if child.returncode:
        raise RuntimeError(f"exit {child.returncode}: {path}")
    return path.read_text(errors="replace"), {
        "wall_ms": round((time.monotonic() - started) * 1000, 3),
        "user_ms": round(usage.ru_utime * 1000, 3),
        "system_ms": round(usage.ru_stime * 1000, 3),
        "peak_rss_kib": usage.ru_maxrss,
    }


def host_results(output):
    lines = [line.removeprefix("BENCH_JSON ") for line in output.splitlines()
             if line.startswith("BENCH_JSON ")]
    if len(lines) != 1:
        raise RuntimeError("expected one BENCH_JSON line")
    return json.loads(lines[0])["results"]


def sv_result(output, name):
    status = re.findall(rf"^BENCH {name} (PASS|FAIL)$", output, re.MULTILINE)
    checksum = re.findall(rf"^CHECKSUM {name} ([0-9a-f]+)$", output, re.MULTILINE)
    work = re.findall(rf"^WORK {name} (\d+) (\w+)$", output, re.MULTILINE)
    phases = re.findall(r"\[PHASE\] simulation: ([0-9.]+)ms", output)
    if status != ["PASS"] or len(checksum) != 1 or len(work) != 1 or len(phases) != 1:
        raise RuntimeError(f"missing PASS/checksum/work/simulation phase for {name}")
    if any(line.startswith("FAIL") for line in output.splitlines()):
        raise RuntimeError(f"failure diagnostic from {name}")
    return float(phases[0]), (checksum[0].zfill(16), work[0])


def ensure_idle():
    busy = []
    for process in Path("/proc").glob("[0-9]*/comm"):
        try:
            name = process.read_text().strip()
        except OSError:
            continue
        if name in {"cargo", "rustc", "rust-lld", "ld.lld", "cc1", "cc1plus", "clang",
                    "iverilog", "vvp", "xezim", "xezim-bench"}:
            try:
                if (process.parent / "stat").read_text().rsplit(")", 1)[1].split()[0] == "Z":
                    continue
            except OSError:
                continue
            busy.append(f"{process.parent.name}:{name}")
    if busy:
        raise RuntimeError("wait for builds/tests before benchmarking: " + ", ".join(busy))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--work-dir", required=True, type=Path)
    parser.add_argument("--candidate", required=True, action="append", type=candidate_arg)
    parser.add_argument("--cpu", type=int, default=0)
    parser.add_argument("--rounds", type=int, default=5)
    parser.add_argument("--cycles", type=int, default=200_000)
    parser.add_argument("--min-host-ms", type=float, default=1000, help="calibrate baseline cycles for at least this runtime")
    parser.add_argument("--max-cycles", type=int, default=2_000_000)
    parser.add_argument("--timeout", type=float, default=120, help="wall seconds per process")
    parser.add_argument("--size", choices=["small", "default", "large"], default="default")
    parser.add_argument("--memory-size", choices=["small", "default", "large"], default="large")
    parser.add_argument("--external-case", type=Path, action="append", default=[],
                        help="JSON descriptor(s) for held-out model/testbench inputs")
    args = parser.parse_args()
    if not hasattr(os, "sched_setaffinity") or args.cpu not in os.sched_getaffinity(0):
        parser.error("--cpu must be allowed by this Linux process's affinity")
    if min(args.rounds, args.cycles, args.max_cycles, args.timeout) <= 0 or args.min_host_ms < 0:
        parser.error("rounds, cycles, maximum cycles and timeout must be positive")
    if len({label for label, _ in args.candidate}) != len(args.candidate):
        parser.error("candidate labels must be unique")
    if args.cycles > args.max_cycles:
        parser.error("--cycles cannot exceed --max-cycles")
    ensure_idle()
    args.work_dir.mkdir(parents=True, exist_ok=True)
    repo = Path(__file__).resolve().parent.parent
    external_cases = []
    for descriptor in args.external_case:
        data = json.loads(descriptor.read_text())
        for case in data if isinstance(data, list) else [data]:
            if not re.fullmatch(r"[A-Za-z0-9_-]+", case["name"]):
                parser.error("external case name must be alphanumeric")
            case = dict(case)
            case["sources"] = [str((descriptor.resolve().parent / name).resolve()) for name in case["sources"]]
            case["include_dirs"] = [str((descriptor.resolve().parent / name).resolve()) for name in case.get("include_dirs", [])]
            case["extra_inputs"] = [str((descriptor.resolve().parent / name).resolve()) for name in case.get("extra_inputs", [])]
            for source in case["sources"] + case["extra_inputs"]:
                if not Path(source).is_file():
                    parser.error(f"external source missing: {source}")
            case["descriptor"] = str(descriptor.resolve())
            external_cases.append(case)
    names = HOST_CASES + [name for name, _, _ in SV_CASES] + [case[0] for case in MEMORY_CASES]
    names += [case["name"] for case in external_cases]
    if len(set(names)) != len(names):
        parser.error("workload names must be unique")
    env = os.environ.copy()
    # Keep caches and profiling out of comparisons. The deterministic host
    # workloads use no RNG; standalone SV runs also request seed 1 explicitly.
    for key in ["XEZIM_VERBOSE", "XEZIM_SIM_DEBUG", "XEZIM_PROFILE_REPORT", "XEZIM_PROFILE_TIMING",
                "LLVM_PROFILE_FILE", "XEZIM_COMPILE_PHASES", "XEZIM_REPORT_STATS",
                "XEZIM_DEBUG_NAMED_ARRAYS", "XEZIM_UNRESOLVED_DUMP", "XEZIM_COMPILE_FAIL_STATS",
                "XEZIM_PROC_LOOP_STATS", "XEZIM_RSS_TRACE", "XEZIM_MEM_CENSUS", "XEZIM_NAME_STATS"]:
        env.pop(key, None)
    env.update(XEZIM_NO_CACHE="1", XEZIM_METHOD_CACHE="0", XEZIM_NO_NATIVE_CACHE="1", XEZIM_JIT="0")
    identities = {label: {name: digest(path / name) for name in ["xezim", "xezim-bench"]}
                  for label, path in args.candidate}
    metadata = {"cpu": args.cpu, "allowed_cpus": sorted(os.sched_getaffinity(0)), "rounds": args.rounds,
                "size": args.size, "memory_size": args.memory_size, "seed": 1,
                "binaries": identities, "cycles": {},
                "candidate_paths": {label: str(path) for label, path in args.candidate},
                "external_cases": external_cases,
                "external_source_inputs": {source: digest(Path(source))
                                           for case in external_cases for source in case["sources"] + case["extra_inputs"]},
                "source_inputs": {str(path.relative_to(repo)): digest(path)
                                  for path in [*(repo / "bench/crosstool" / f"{name}.sv" for name, _, _ in SV_CASES),
                                               *(repo / "scripts/pgo-train" / f"{name}.sv" for name in MEMORY_GOLDENS)]}}
    expected_sv = {}

    def external_run(label, directory, case, tag, preflight=False):
        stage = "preflight" if preflight else "benchmark"
        path = args.work_dir / f"{tag}.{label}.external-{case['name']}.log"
        command = [str(directory / "xezim"), "--simulate", "--no-cache", "--verbose",
                   "-s", case.get("top", "tb"), "--max-time", "100000000", "+seed=1"]
        for include in case.get("include_dirs", []):
            command.extend(["-I", include])
        command.extend("-D" + define for define in case.get("defines", []) + case.get(stage + "_defines", []))
        command.extend(case["sources"])
        command.extend(case.get("args", []) + case.get(stage + "_args", []))
        output, measured = run(command, path, args.cpu, args.timeout, env)
        for pattern in case.get("reject_patterns", []):
            if re.search(pattern, output):
                raise RuntimeError(f"model diagnostic matched {pattern!r}: {path}")
        duration, semantic = sv_result(output, case.get("marker", case["name"]))
        if not preflight and duration <= 0:
            raise RuntimeError(f"model simulation too short to measure: {case['name']}; increase its workload")
        expected = case.get(stage + "_checksum", case.get("golden_checksum"))
        if expected is not None and semantic[0] != expected.lower().zfill(16):
            raise RuntimeError(f"external model golden mismatch: {label}/{case['name']}: {semantic[0]} != {expected}")
        measured.update(simulation_ms=duration, checksum=semantic[0], work=semantic[1],
                        correctness="model PASS/checksum/work matched", log=str(path))
        return measured, semantic

    def host_run(label, directory, name, cycles, tag):
        path = args.work_dir / f"{tag}.{label}.host-{name}.log"
        output, measured = run([str(directory / "xezim-bench"), "--cycles", str(cycles),
                                "--repeats", "1", "--filter", name, "--json-only"],
                               path, args.cpu, args.timeout, env)
        results = host_results(output)
        if len(results) != 1 or results[0]["name"] != name or results[0]["cycles"] != cycles:
            raise RuntimeError(f"wrong host workload/cycle count: {path}")
        measured.update(simulation_ms=results[0]["run_ms"], compile_ms=results[0]["compile_ms"],
                        cycles=cycles, correctness="Rust mirror passed", log=str(path))
        return measured

    def sv_run(label, directory, name, top, size, tag, memory_case=None):
        path = args.work_dir / f"{tag}.{label}.sv-{name}.log"
        marker = memory_case[1] if memory_case else name
        source = (repo / "scripts/pgo-train" / f"{marker}.sv") if memory_case else (repo / "bench/crosstool" / f"{name}.sv")
        command = [str(directory / "xezim"), "--simulate", "--no-cache", "--verbose", "-s", top,
                   "--max-time", "100000000", str(source), "+seed=1"]
        if memory_case:
            command.extend("-D" + define for define in memory_case[2])
        if size != "default":
            command.append(("-DMEMORY_" if memory_case else "-DBENCH_") + size.upper())
        output, measured = run(command, path, args.cpu, args.timeout, env)
        duration, semantic = sv_result(output, marker)
        if tag != "preflight" and duration <= 0:
            raise RuntimeError(f"simulation too short to measure: {name}; use a larger size")
        if memory_case and semantic[0] != MEMORY_GOLDENS[marker][size]:
            raise RuntimeError(f"independent memory checksum mismatch: {label}/{name}: {semantic[0]}")
        measured.update(simulation_ms=duration, checksum=semantic[0], work=semantic[1],
                        correctness="PASS/checksum/work matched", log=str(path))
        return measured, semantic

    for label, directory in args.candidate:
        ensure_idle()
        for name in HOST_CASES:
            host_run(label, directory, name, 1000, "warmup")
        for name, top, golden in SV_CASES:
            _, semantic = sv_run(label, directory, name, top, "small", "preflight")
            if semantic[0] != golden:
                raise RuntimeError(f"small golden checksum mismatch: {label}/{name}: {semantic[0]} != {golden}")
        for memory_case in MEMORY_CASES:
            name, marker, _, golden = memory_case
            _, semantic = sv_run(label, directory, name, "tb", "small", "preflight", memory_case)
            if golden is not None and semantic[0] != golden:
                raise RuntimeError(f"small memory checksum mismatch: {label}/{name}: {semantic[0]} != {golden}")
        for case in external_cases:
            _, semantic = external_run(label, directory, case, "preflight", preflight=True)
            expected = expected_sv.setdefault("preflight:" + case.get("marker", case["name"]), semantic)
            if semantic != expected:
                raise RuntimeError(f"external preflight checksum/work mismatch: {label}/{case['name']}")
        print(f"preflight passed: {label}", flush=True)

    baseline_label, baseline_dir = args.candidate[0]
    for name in HOST_CASES:
        cycles = args.cycles
        pilot = host_run(baseline_label, baseline_dir, name, cycles, "calibrate")
        if name in {"mem_array", "wide_vec"} and 0 < pilot["simulation_ms"] < args.min_host_ms:
            cycles = min(args.max_cycles, max(cycles, math.ceil(cycles * args.min_host_ms / pilot["simulation_ms"])))
        metadata["cycles"][name] = cycles
    (args.work_dir / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")

    rows = []
    with (args.work_dir / "runs.jsonl").open("w") as output:
        for round_index in range(args.rounds):
            ensure_idle()
            offset = round_index % len(args.candidate)
            ordered = args.candidate[offset:] + args.candidate[:offset]
            if round_index % 2:
                ordered = list(reversed(ordered))
            cases = [("host", name, None, None) for name in HOST_CASES]
            cases += [("sv", name, top, None) for name, top, _ in SV_CASES]
            cases += [("sv", case[0], "tb", case) for case in MEMORY_CASES]
            cases += [("external", case["name"], case.get("top", "tb"), case) for case in external_cases]
            for kind, name, top, memory_case in cases:
                for label, directory in ordered:
                    tag = f"round-{round_index + 1}"
                    if kind == "host":
                        row = host_run(label, directory, name, metadata["cycles"][name], tag)
                    elif kind == "sv":
                        row, semantic = sv_run(label, directory, name, top,
                                               args.memory_size if memory_case else args.size, tag, memory_case)
                        comparison_name = memory_case[1] if memory_case else name
                        expected = expected_sv.setdefault(comparison_name, semantic)
                        if semantic != expected:
                            raise RuntimeError(f"checksum/work mismatch: {label}/{name}: {semantic} != {expected}")
                    else:
                        row, semantic = external_run(label, directory, memory_case, tag)
                        comparison_name = "external:" + memory_case.get("marker", name)
                        expected = expected_sv.setdefault(comparison_name, semantic)
                        if semantic != expected:
                            raise RuntimeError(f"external model checksum/work mismatch: {label}/{name}: {semantic} != {expected}")
                    row.update(candidate=label, kind=kind, workload=name, round=round_index + 1)
                    row["memory_focus"] = name in {"mem_array", "wide_vec", "b3_memory"} or memory_case is not None
                    rows.append(row)
                    output.write(json.dumps(row, sort_keys=True) + "\n")
                    output.flush()
            print(f"round {round_index + 1}/{args.rounds} complete", flush=True)

    # Verify callers did not replace a candidate while it was measured.
    for label, directory in args.candidate:
        for name, expected in identities[label].items():
            if digest(directory / name) != expected:
                raise RuntimeError(f"candidate changed during measurement: {label}/{name}")
    for source, expected in metadata["source_inputs"].items():
        if digest(repo / source) != expected:
            raise RuntimeError(f"benchmark input changed during measurement: {source}")
    for source, expected in metadata["external_source_inputs"].items():
        if digest(Path(source)) != expected:
            raise RuntimeError(f"external model input changed during measurement: {source}")
    medians = {}
    for row in rows:
        medians.setdefault(row["candidate"], {}).setdefault(row["workload"], []).append(row["simulation_ms"])
    medians = {label: {name: statistics.median(values) for name, values in cases.items()}
               for label, cases in medians.items()}
    speedups = {label: math.exp(statistics.mean(math.log(medians[baseline_label][name] / duration)
                                               for name, duration in cases.items()))
                for label, cases in medians.items()}
    memory_names = {row["workload"] for row in rows if row["memory_focus"]}
    memory_speedups = {label: math.exp(statistics.mean(math.log(medians[baseline_label][name] / duration)
                                                      for name, duration in cases.items() if name in memory_names))
                      for label, cases in medians.items()}
    memory_round_speedups = {}
    for label in medians:
        values = []
        for round_index in range(1, args.rounds + 1):
            paired = {row["candidate"]: {} for row in rows}
            for row in rows:
                if row["round"] == round_index and row["memory_focus"]:
                    paired[row["candidate"]][row["workload"]] = row["simulation_ms"]
            values.append(math.exp(statistics.mean(math.log(paired[baseline_label][name] / duration)
                                                   for name, duration in paired[label].items())))
        memory_round_speedups[label] = values
    external_names = {case["name"] for case in external_cases}
    external_speedups = {label: math.exp(statistics.mean(math.log(medians[baseline_label][name] / duration)
                                                        for name, duration in cases.items() if name in external_names))
                        for label, cases in medians.items()} if external_cases else {}
    summary = {"median_simulation_ms": medians, "geometric_mean_speedup": speedups,
               "memory_geometric_mean_speedup": memory_speedups,
               "memory_speedup_by_round": memory_round_speedups,
               "external_memory_geometric_mean_speedup": external_speedups,
               "fastest_measured_memory": max(external_speedups or memory_speedups, key=(external_speedups or memory_speedups).get)}
    (args.work_dir / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
