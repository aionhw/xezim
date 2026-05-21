# C910 `hello` on xezim — with XTrace scoped to CPU 0

This directory runs the **XuanTie-C910** `hello_world` test on the **xezim**
SystemVerilog simulator and dumps an **XTrace** signal trace **restricted to
CPU core 0** (the `x_ct_top_0` instance).

Two scripts:

| Script                       | Purpose                                                        |
|------------------------------|----------------------------------------------------------------|
| `setup.sh`                   | Clone the required repos and build `xezim`.                     |
| `run_c910_hello_xtrace.sh`   | Stage inputs, run the test, dump the cpu0-scoped XTrace.        |

## Quick start

```bash
cd xuantie_c910
./setup.sh                       # clone repos + cargo build --release  (a few min)
./run_c910_hello_xtrace.sh       # run the test  (~3-4 min) with xtrace
or
./run_c910_hello.sh       # run the test  (~3-4 min) without xtrace
```

On success the run script prints:

```
==> result:
    [XTrace] dumping 403486 signals (scopes=1)
    Hello Friend!
    *    simulation finished successfully        *
    TEST PASSED
    Simulation finished at time 44695

==> xtrace file: 687081359 bytes  .../work/c910_hello_cpu0.xt
```

## Prerequisites

- `git`, `bash`, `python3` with the **`yaml`** module (`pip install pyyaml`)
- `cargo` / Rust toolchain (to build xezim)
- `stdbuf` (coreutils — used for the line-buffered log)
- ~2 GB disk for the repos, ~700 MB for the trace, ~9 GB RAM for the run

## What `setup.sh` does

Clones three repos as **siblings** under `./deps/` (the layout matters —
xezim's `Cargo.toml` points at `../xezim-core` and
`../xezim-core/xezim-parser` by relative path):

```
deps/
  xezim/        git@github-xezim:aionhw/xezim        @ main   the simulator
  xezim-core/   git@github-xezim-core:aionhw/xezim-core @ main engine + xezim-parser
  rtlmeter/     https://github.com/verilator/rtlmeter @ main  C910 RTL + tests
```

Then runs `cargo build --release` in `deps/xezim/`, producing
`deps/xezim/target/release/xezim`.

The C910 RTL **and** the `hello` test image (`inst.pat` / `data.pat`) are
committed inside the rtlmeter repo at `designs/XuanTie-C910/`, so there is no
separate `openc910` checkout.

URLs / branches are overridable:

```bash
XEZIM_URL=git@github.com:aionhw/xezim.git \
XEZIM_CORE_URL=git@github.com:aionhw/xezim-core.git \
DEPS_DIR=/scratch/c910deps  ./setup.sh
```

> The default xezim URLs use the SSH host aliases `github-xezim` /
> `github-xezim-core`. If you do not have those in `~/.ssh/config`, override
> `XEZIM_URL` / `XEZIM_CORE_URL` with the plain `git@github.com:…` or
> `https://github.com/…` form.

**Using an xezim that is already built** (skip `setup.sh`): point the run
script straight at it —

```bash
XEZIM=/path/to/xezim/target/release/xezim \
DESIGN_DIR=/path/to/rtlmeter/designs/XuanTie-C910 \
  ./run_c910_hello_xtrace.sh
```

## What `run_c910_hello_xtrace.sh` does

1. Resolves `XEZIM`, `DESIGN_DIR`, `WORK_DIR` (symlinks resolved with
   `pwd -P` so absolute filelist paths stay valid).
2. Creates `WORK_DIR` (`./work` by default) and stages `data.pat` +
   `inst.pat` from `tests/hello/`.
3. Writes the empty `__rtlmeter_top_include.vh` stub — see
   *The `__rtlmeter` stub* below.
4. Generates `c910.fl` from `descriptor.yaml::compile.verilogSourceFiles`
   (absolute paths).
5. Runs:
   ```
   xezim --simulate --max-time 80000000 -s tb \
         --xtrace      work/c910_hello_cpu0.xt \
         --xtrace-scope x_soc.x_cpu_sub_system_axi.x_rv_integration_platform.x_cpu_top.x_ct_top_0 \
         -I work -I <DESIGN_DIR>/src \
         -f c910.fl
   ```
6. Greps the log for the result, exits 0 on `TEST PASSED`.

### How the cpu0 scope works

The C910 SoC instance path down to a CPU core is:

```
tb
└─ x_soc
   └─ x_cpu_sub_system_axi
      └─ x_rv_integration_platform
         └─ x_cpu_top                 (module openC910)
            ├─ x_ct_top_0             ◄── CPU core 0   (this is "cpu0")
            └─ x_ct_top_1                 CPU core 1
```

`xezim --xtrace-scope <hier>` keeps a signal in the dump when its dotted
hierarchical name **equals** `<hier>` or **starts with** `"<hier>."`. Passing

```
x_soc.x_cpu_sub_system_axi.x_rv_integration_platform.x_cpu_top.x_ct_top_0
```

therefore captures core 0's entire subtree and nothing else. The flag is
repeatable; the work-dir scope name comes from the `CPU0_SCOPE` env var if
you need to override it (e.g. swap `x_ct_top_0` → `x_ct_top_1` to trace core 1).

In the resulting `.xt` dictionary every traced module sits under
`/tb/.../x_cpu_top/x_ct_top_0/...`; only the **5 ancestor scopes** on the
path from `tb` down to the core appear outside it (they are needed to root
the hierarchy).

## Configuration (env vars)

| Variable      | Default                                              | Meaning                                  |
|---------------|------------------------------------------------------|------------------------------------------|
| `DEPS_DIR`    | `./deps`                                             | where `setup.sh` clones / `run` looks    |
| `XEZIM`       | `${DEPS_DIR}/xezim/target/release/xezim`             | xezim binary                             |
| `DESIGN_DIR`  | `${DEPS_DIR}/rtlmeter/designs/XuanTie-C910`          | C910 design root (`src/`, `tests/`)      |
| `WORK_DIR`    | `./work`                                             | staged inputs + outputs                  |
| `MAX_TIME`    | `80000000`                                           | ns of sim time before xezim aborts       |
| `CPU0_SCOPE`  | `x_soc.…​.x_cpu_top.x_ct_top_0`                       | hierarchical scope passed to `--xtrace-scope` |

## Output

Files in `WORK_DIR` after a successful run:

| File                    | Contents                                                  |
|-------------------------|-----------------------------------------------------------|
| `c910_hello_cpu0.xt`    | XTrace dump — signals under cpu0 only                     |
| `c910_hello_cpu0.log`   | full xezim stdout, phase timing, PROF stats               |
| `c910.fl`               | generated absolute-path filelist                          |
| `data.pat`, `inst.pat`  | staged test image                                         |
| `__rtlmeter_top_include.vh` | empty stub (the workaround)                           |

## Reference results

Verified with the `main`-branch xezim build (`xezim 0.1.2`, XTrace `1.0`):

| Metric                 | Value                                          |
|------------------------|------------------------------------------------|
| Result                 | **TEST PASSED**, `Hello Friend!`               |
| `$finish` sim time     | **44 695 ns**                                  |
| Signals dumped (cpu0)  | **403 486** (`scopes=1`)                       |
| Trace size             | ≈ **687 MB** (`c910_hello_cpu0.xt`, text)      |
| Dictionary records     | 3 896 `M` (5 ancestors + 3 891 under cpu0), 403 486 `S` |
| Wall time              | ≈ 3 min 30 s                                   |
| Peak RSS               | ≈ 9 GB                                         |

Any deviation in the sim time (44 695 ns) means the run took a different
path. The trace is large because XTrace's `minimal` profile is plain text;
post-process it if size matters (see below).

## The `__rtlmeter` stub

`tb.v` does `` `include "__rtlmeter_top_include.vh" ``. The *real* include
lives in `rtlmeter/rtl/` and instantiates `__rtlmeter_utils`, whose body has:

```verilog
longint unsigned max_cycles = '1;
```

xezim evaluates that `'1` to **0** (a known xezim bug). The
`cycles >= max_cycles` watchdog inside `__rtlmeter_utils` then fires `$finish`
at **t = 0**, before the CPU executes a single instruction —
`RTLMeter: +max_cycles reached, exiting`, `TEST FAILED`.

The fix is the single **modification** this flow needs: an *empty*
`__rtlmeter_top_include.vh` staged into `WORK_DIR`. Because `WORK_DIR` is the
first `-I` entry and `rtlmeter/rtl/` is deliberately **not** on the include
path, `tb.v` finds the empty stub and compiles without that module. The run
script creates this file automatically — no repo is patched.

> **Do not add `rtlmeter/rtl/` to `-I`.** It would shadow the stub with the
> broken real include and re-trigger the t=0 `$finish`.

## Gotchas

- **xezim build must be current.** XTrace samples signals every cycle; an old
  xezim binary mishandles that against the NBA region and the hello test ends
  in `TEST FAILED` at sim time ≈ 45 565 ns with corrupted output
  (`Hello Frie*****…`). Rebuild with `cargo build --release`.
- **Symlinked repos.** The scripts resolve `DESIGN_DIR` with `cd … && pwd -P`
  before writing the filelist, so a symlinked `rtlmeter` does not break the
  absolute `-f` / `-I` paths.
- **`MAX_TIME`** default `80000000` ns is well past hello's 44 695 ns finish.
  Keep it generous; on a real stall the testbench's own watchdog reports
  `TEST FAILED` long before `MAX_TIME` is hit.

## Adapting

- **Trace CPU core 1** instead: `CPU0_SCOPE=…​.x_cpu_top.x_ct_top_1 ./run_c910_hello_xtrace.sh`
- **Trace the whole design**: drop `--xtrace-scope` from the script (expect a
  multi-GB trace).
- **A different test** (`memcpy`, `cmark`): stage that test's `data.pat` /
  `inst.pat` instead of `tests/hello/`. Note `memcpy` on c910 hits a separate,
  unrelated xezim bug (PC-FIFO entry release) around 46 µs.
- **Shrink the trace**: the `xtrace_opt/` crate elsewhere in this repo
  rewrites a `.xt` into a compact text or ULEB128 binary form.
