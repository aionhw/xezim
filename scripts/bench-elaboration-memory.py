#!/usr/bin/env python3
r"""Generate and measure the synthetic wildcard-import workload from issue #721.

Example (Linux/macOS, standard library only):
  python3 scripts/bench-elaboration-memory.py --work-dir /tmp/elab-memory \
    --binary before=/tmp/xezim-before --binary after=target/debug/xezim \
    --case 2000,20,1 --case 10000,100,1 --case 10000,100,100

Each case is TYPEDEFS,INSTANCES,DEFINITIONS. Changing definitions independently
distinguishes retained per-instance import history from per-definition scope
copies. All leaf definitions are instantiated and all instances check imported
type widths. Add --package-parameters --vary-overrides to exercise retained
prepared-module cache keys with large shared parameter contexts and distinct
instance values. RSS is observational; this benchmark imposes no CI memory or
time threshold. It models the reported shape, not the unavailable RTL design.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import resource
import signal
import subprocess
import sys
import time


def case_arg(value):
    try:
        typedefs, instances, definitions = map(int, value.split(","))
    except ValueError as error:
        raise argparse.ArgumentTypeError("expected TYPEDEFS,INSTANCES,DEFINITIONS") from error
    if min(typedefs, instances, definitions) <= 0 or definitions > instances:
        raise argparse.ArgumentTypeError("counts must be positive and definitions <= instances")
    return typedefs, instances, definitions


def binary_arg(value):
    label, sep, filename = value.partition("=")
    if not sep or not label or any(c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-" for c in label):
        raise argparse.ArgumentTypeError("expected LABEL=/path/to/xezim, with an alphanumeric label")
    binary = Path(filename).resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise argparse.ArgumentTypeError(f"not an executable: {binary}")
    return label, binary


def design(case, imports, relay, package_parameters, vary_overrides):
    typedefs, instances, definitions = case
    lines = ["package types_pkg;"]
    for i in range(typedefs):
        if package_parameters:
            lines.extend([
                f"  localparam int P_{i} = {i};",
                f"  typedef logic [31 + (P_{i} % 8):0] t_{i};",
            ])
        else:
            lines.append(f"  typedef logic [31:0] t_{i};")
    if package_parameters:
        # Typedef ranges may already be folded when the scope scanner runs.
        # An uncalled function keeps every P_i visibly named in package syntax,
        # so the prepared scopes retain the large shared parameter context.
        lines.extend(["  function automatic int parameter_context();", "    int value = 0;"])
        lines.extend(f"    value ^= P_{i};" for i in range(typedefs))
        lines.extend(["    return value;", "  endfunction"])
    lines.append("endpackage")
    package = "types_pkg"
    if relay:
        lines.extend(["package relay_pkg;", "  import types_pkg::*;", "endpackage"])
        package = "relay_pkg"
    for i in range(definitions):
        parameter = " #(parameter int INSTANCE_ID = 0)" if vary_overrides else ""
        lines.append(f"module leaf_{i}{parameter}(output logic [31:0] value);")
        lines.extend(f"  import {package}::*;" for _ in range(imports))
        value_expr = f"$bits(t_0) + $bits(t_{typedefs - 1})"
        if vary_overrides:
            value_expr += " + INSTANCE_ID"
        lines.extend([
            f"  assign value = {value_expr};",
            "endmodule",
        ])
    lines.extend(["module tb;", f"  wire [31:0] value [0:{instances - 1}];"])
    for i in range(instances):
        override = f" #(.INSTANCE_ID({i}))" if vary_overrides else ""
        lines.append(f"  leaf_{i % definitions}{override} u_{i}(.value(value[{i}]));")
    base_value = 64 + ((typedefs - 1) % 8 if package_parameters else 0)
    checksum = base_value * instances + (instances * (instances - 1) // 2 if vary_overrides else 0)
    marker = (f"BENCH_OK typedefs={typedefs} instances={instances} "
              f"definitions={definitions} checksum={checksum}")
    expected_value = f"({base_value} + i)" if vary_overrides else str(base_value)
    lines.extend([
        "  initial begin",
        "    #1;",
        f"    for (int i = 0; i < {instances}; i++) begin",
        f'      if (value[i] !== {expected_value}) $fatal(1, "incorrect imported type width at %0d: %0d", i, value[i]);',
        "    end",
        f'    $display("{marker}");',
        "    $finish;",
        "  end",
        "endmodule",
    ])
    return "\n".join(lines) + "\n", marker


def measure(binary, source, output_path, timeout, memory_limit_mib):
    def child_limits():
        limit = memory_limit_mib * 1024 * 1024
        resource.setrlimit(resource.RLIMIT_AS, (limit, limit))

    start = time.monotonic()
    env = os.environ.copy()
    # These shallow generated modules need no large recursion stack. The
    # default reserves 1 GiB for each of two worker stacks and would exhaust
    # the address-space cap before the import workload can run.
    env["XEZIM_STACK_MB"] = "64"
    with output_path.open("wb") as output:
        child = subprocess.Popen(
            [str(binary), "--simulate", "--no-cache", "-s", "tb", str(source)],
            stdout=output, stderr=subprocess.STDOUT, start_new_session=True,
            preexec_fn=child_limits, env=env,
        )
        timed_out = False
        while True:
            pid, status, usage = os.wait4(child.pid, os.WNOHANG)
            if pid:
                break
            if time.monotonic() - start > timeout:
                timed_out = True
                os.killpg(child.pid, signal.SIGKILL)
                _, status, usage = os.wait4(child.pid, 0)
                break
            time.sleep(0.02)
        child.returncode = os.waitstatus_to_exitcode(status)
    rss_kib = usage.ru_maxrss / (1024 if sys.platform == "darwin" else 1)
    return {
        "exit_code": child.returncode,
        "timed_out": timed_out,
        "peak_rss_kib": int(rss_kib),
        "elapsed_seconds": round(time.monotonic() - start, 3),
        "user_seconds": round(usage.ru_utime, 3),
        "system_seconds": round(usage.ru_stime, 3),
        "stack_mib": 64,
        "address_space_limit_mib": memory_limit_mib,
        "timeout_seconds": timeout,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--work-dir", type=Path, required=True, help="generated sources and captured logs")
    parser.add_argument("--binary", action="append", type=binary_arg, default=[], help="LABEL=/path/to/xezim (repeatable)")
    parser.add_argument("--case", action="append", type=case_arg, help="TYPEDEFS,INSTANCES,DEFINITIONS (repeatable)")
    parser.add_argument("--imports", type=int, default=2, help="wildcard imports in each leaf (default: 2)")
    parser.add_argument("--relay", action="store_true", help="import through a second package")
    parser.add_argument("--package-parameters", action="store_true", help="use a package parameter in each typedef width")
    parser.add_argument("--vary-overrides", action="store_true", help="give each leaf instance a distinct parameter value")
    parser.add_argument("--repeat", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=120, help="seconds per child process")
    parser.add_argument("--memory-limit-mib", type=int, default=3072, help="child address-space limit, not an RSS assertion")
    parser.add_argument("--generate-only", action="store_true")
    args = parser.parse_args()
    if not hasattr(os, "wait4"):
        parser.error("requires POSIX os.wait4 for per-process peak RSS")
    if min(args.imports, args.repeat, args.timeout, args.memory_limit_mib) <= 0:
        parser.error("imports, repeat, timeout and memory limit must be positive")
    if not args.binary and not args.generate_only:
        parser.error("provide --binary or --generate-only")
    if len({label for label, _ in args.binary}) != len(args.binary):
        parser.error("binary labels must be unique")
    cases = args.case or [(2000, 20, 1), (10000, 20, 1), (10000, 100, 1), (10000, 100, 100)]
    args.work_dir.mkdir(parents=True, exist_ok=True)
    failed = False
    for case in cases:
        name = "types{}_instances{}_defs{}".format(*case)
        source = (args.work_dir / f"{name}.sv").resolve()
        text, marker = design(case, args.imports, args.relay, args.package_parameters, args.vary_overrides)
        source.write_text(text)
        if args.generate_only:
            print(source)
            continue
        expected_output = None
        for label, binary in args.binary:
            for repetition in range(args.repeat):
                output_path = args.work_dir / f"{name}.{label}.{repetition}.log"
                result = measure(binary, source, output_path, args.timeout, args.memory_limit_mib)
                output = output_path.read_text(errors="replace")
                # Build banners differ between executable snapshots. Every
                # subsequent simulation/diagnostic line must remain identical.
                normalized = "\n".join(line for line in output.splitlines()
                                       if not line.startswith(("=== xezim ", "git ")))
                valid = result["exit_code"] == 0 and marker in normalized.splitlines()
                if valid and expected_output is None:
                    expected_output = normalized
                identical = valid and normalized == expected_output
                failed |= not identical
                result.update({
                    "case": name, "binary": label, "binary_path": str(binary),
                    "repetition": repetition, "imports": args.imports, "relay": args.relay,
                    "package_parameters": args.package_parameters, "vary_overrides": args.vary_overrides,
                    "output_matches": identical, "source": str(source), "log": str(output_path),
                    "source_sha256": hashlib.sha256(text.encode()).hexdigest(),
                    "simulation_output_sha256": hashlib.sha256(normalized.encode()).hexdigest(),
                })
                print(json.dumps(result, sort_keys=True), flush=True)
    return int(failed)


if __name__ == "__main__":
    sys.exit(main())
