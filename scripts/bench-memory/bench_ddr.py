#!/usr/bin/env python3
"""Time and check a local copy of the Micron DDR3 v1.74 model."""

import argparse
import hashlib
import os
from pathlib import Path
import re
import subprocess

MODEL_FILES = {
    "ddr3.sv": "1a061f47ec3178c60a05e79be283ccf6dab5f4ccba64a2707eb2fc68f26d1c67",
    "8192Mb_ddr3_parameters.vh": "1799d431e891749f033d74aa9809ae3a9d588751d33d6be2a2d8d3359d26bfea",
    "sim_defines.vh": "7b3dcb722f3e09b0279ef5d982aaee3deaf0a3246c966b2619726d4eb5d5f331",
}
BENCH = Path(__file__).resolve().with_name("ddr_tb.sv")


def prepare_models(source, output):
    files = {name: (source / name).read_bytes() for name in MODEL_FILES}
    for name, digest in MODEL_FILES.items():
        if hashlib.sha256(files[name]).hexdigest() != digest:
            raise ValueError(f"Unexpected revision of {source / name}")

    original = files["ddr3.sv"].decode().split("\n", 1)[1]
    start = original.index("    task data_task;")
    end = original.index("    endtask", start)
    task = original[start:end]
    # The pinned model has one 64-iteration diagnostic loop per clock phase.
    high, low = re.finditer(
        r"^ {16}for \(i=0; i<64; i=i\+1\) begin\n.*?^ {16}end\n",
        task, re.MULTILINE | re.DOTALL,
    )
    for scan, phase in ((low, "low"), (high, "high")):
        body = "".join("    " + line for line in scan[0].splitlines(keepends=True))
        replacement = (
            "                // Run the lane checks whenever either condition may be active.\n"
            "                if (((dq_in_valid && dll_locked) !== 1'b0) ||\n"
            f"                    (((|check_write_dqs_{phase}) && !DLL_OFF) !== 1'b0)) begin\n"
            f"{body}"
            "                end else begin\n"
            "                    i = 64;\n"
            "                end\n"
        )
        task = task[:scan.start()] + replacement + task[scan.end():]

    for variant, model in (("original", original), ("guarded", original[:start] + task + original[end:])):
        directory = output / variant
        directory.mkdir()
        (directory / "ddr3.sv").write_text(model)
        for name in MODEL_FILES.keys() - {"ddr3.sv"}:
            (directory / name).write_bytes(files[name])


def run(directory, command, *, timeout=180):
    directory.mkdir(exist_ok=True)
    env = {name: value for name, value in os.environ.items() if not name.startswith("XEZIM_")}
    env["XEZIM_JIT"] = "0"
    with (directory / "stdout.log").open("w") as stdout, (directory / "stderr.log").open("w") as stderr:
        subprocess.run(command, cwd=directory, env=env,
                       stdout=stdout, stderr=stderr, check=True, timeout=timeout)
    return (directory / "stdout.log").read_text(), (directory / "stderr.log").read_text()


def signature(stdout, stderr, count):
    checksum = sum((0x639A ^ (burst * 0x11B) ^ (beat * 0x2471)) & 0xFFFF
                   for burst in range(count) for beat in range(8)) & 0xFFFFFFFF
    summary = f"DDR_READBACK bursts={count} beats={count * 8} checksum={checksum:08x}"
    finish = re.search(r"Simulation finished at time (\d+) \(\$finish called\)|\$finish called at (\d+)", stdout)
    if summary not in stdout or "TEST PASSED" not in stdout or not finish or re.search(
        r"ERROR:|\bFATAL\b", stdout + stderr, re.IGNORECASE
    ):
        raise RuntimeError("DDR simulation failed its completion or readback checks")
    return summary, finish[1] or finish[2]


def reference(directory, model, stimulus):
    directory.mkdir()
    with (directory / "compile.log").open("w") as log:
        subprocess.run(["iverilog", "-g2012", "-s", "ddr_tb", "-I", model, "-o", "run.vvp",
                        model / "ddr3.sv", BENCH], cwd=directory, stdout=log, stderr=log,
                       check=True, timeout=60)
    return run(directory, ["vvp", "-n", "run.vvp", *stimulus])


def xezim(directory, model, binary, stimulus, cpu):
    affinity = [] if cpu is None else ["taskset", "-c", str(cpu)]
    return run(directory, [*affinity, "/usr/bin/time", "-o", "time.txt",
                           "-f", "wall=%e rss_kib=%M exit=%x", binary, "--simulate",
                           "--no-cache", "--verbose", "--error-exit", "--relax-use-before-decl",
                           "-s", "ddr_tb", "-I", model, model / "ddr3.sv", BENCH,
                           "--max-time", "3ms", *stimulus])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model_dir", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("xezim", type=Path)
    parser.add_argument("--compare-xezim", type=Path, help="Compare two binaries using the original model")
    parser.add_argument("--bursts", type=int, default=128, choices=(4, 128, 512))
    parser.add_argument("--pairs", type=int, default=3)
    parser.add_argument("--cpu", type=int)
    args = parser.parse_args()
    if args.pairs < 1:
        parser.error("--pairs must be positive")
    output, original_binary = args.output.resolve(), args.xezim.resolve(strict=True)
    compare_binary = args.compare_xezim.resolve(strict=True) if args.compare_xezim else original_binary
    output.mkdir()
    prepare_models(args.model_dir, output)
    original = output / "original"
    candidate = original if args.compare_xezim else output / "guarded"
    stimulus = ["+model_data+.", f"+bursts={args.bursts}"]
    oracle = signature(*reference(output / "reference", original, stimulus), args.bursts)

    fault = ["+model_data+.", "+bursts=4", "+dqs_phase=-600"]
    ref_errors, _ = reference(output / "fault-reference", original, fault)
    expected = [line for line in ref_errors.splitlines() if "ERROR:" in line]
    if len(expected) != 16:
        raise RuntimeError("DDR fault control failed to produce 16 timing diagnostics")
    variants = [("baseline", original, original_binary), ("candidate", candidate, compare_binary)]
    for label, model, binary in variants:
        stdout, _ = xezim(output / f"fault-{label}", model, binary, fault, args.cpu)
        if [line for line in stdout.splitlines() if "ERROR:" in line] != expected:
            raise RuntimeError(f"DDR timing diagnostics differ for {label}")

    for pair in range(args.pairs):
        for label, model, binary in (variants if pair % 2 == 0 else reversed(variants)):
            directory = output / f"{pair + 1}-{label}"
            stdout, stderr = xezim(directory, model, binary, stimulus, args.cpu)
            if signature(stdout, stderr, args.bursts) != oracle:
                raise RuntimeError(f"DDR readback or finish time differs for {directory}")
            phase = re.search(r"^\[PHASE\] simulation: ([\d.]+)ms$", stderr, re.MULTILINE)
            if not phase:
                raise RuntimeError(f"Missing simulation timing for {directory}")
            print(f"{directory.name} simulation_ms={phase[1]} {(directory / 'time.txt').read_text().strip()}",
                  flush=True)


if __name__ == "__main__":
    main()
