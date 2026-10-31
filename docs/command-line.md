# Command-line reference

`xezim <source files> [+plusargs] [options]` parses, elaborates and simulates
in one step. This page lists the options, the environment variables, and the
compile and simulate spellings of other simulators that xezim accepts. For
how to use them together, see the [user guide](user-guide.md).

* [Options](#options)
* [Environment variables](#environment-variables)
* [Command lines from other simulators](#command-lines-from-other-simulators)

## Options

| Option | Purpose |
|---|---|
| `-D<MACRO>[=val]` | Define a preprocessor macro |
| `-I<dir>` | Add an include directory |
| `-uvm`, `--uvm` | Add the UVM library that `XEZIM_UVM_DIR` names: its `src` directory goes on the include path and its `uvm_pkg.sv` becomes the first source file, unless the file list already has a `uvm_pkg.sv`. Works in every mode and inside `-f`/`-F` files. Without `-uvm` nothing is added. See the [UVM guide](uvm-guide.md#adding-the-uvm-library-with--uvm) |
| `--simulate` | Run the simulation (vs `--parse` / `--compile` / `--preprocess`) |
| `-s <module>` | Select a top-level module. Repeat for multiple roots (e.g. `-s hdl_top -s hvl_top`); each is a root of its own in `%m`, messages, `$root` paths and waveform scopes, as when several tops are found automatically. A bare module name that is not a file does the same (see [below](#command-lines-from-other-simulators)) |
| `--no-strict-top` | When an `-s` names no module, warn and find the tops automatically instead of exiting 1 |
| `--sv2017` | Parse with the IEEE 1800-2017 grammar instead of the default 1800-2023 |
| `--no-strict` (`--lenient`) | Accept some LRM-illegal constructs that are errors by default, among them a hierarchical name that does not resolve (§23.8), which is then left unchecked |
| `--dpi-lib <path>` | Load a DPI-C shared library (`.so`/`.dylib`/`.dll`). Repeatable. See the [DPI and VPI guide](dpi-guide.md) |
| `--vpi-lib <path>` (`-m`) | Load a VPI module and run its `vlog_startup_routines`. Repeatable. See the [DPI and VPI guide](dpi-guide.md) |
| `--module-timescale [mods=]<unit>/<prec>` | Assign a timescale to modules with no explicit source-level one. Repeatable. See the [user guide](user-guide.md#module-timescale-extension) |
| `--dump-timescales` | Print every module's resolved timescale before the run (no source `$printtimescale` needed); modules with no `` `timescale `` are flagged |
| `--max-time <N>[ps\|ns\|us\|ms\|s]` | Stop simulation after `N` of simulated time (default `100ms`) — **nanoseconds** when no unit is given. The cap is resolved to whole nanoseconds (a sub-ns value rounds to the nearest one; below half a nanosecond is rejected) and then converted to the design's tick, so the same `--max-time` covers the same simulated time whatever the precision |
| `+trace`, `+<plusarg>` | Passed through to `$value$plusargs` / `$test$plusargs` |
| `+seed=<n>` | Seed the RNG for a reproducible run (same seed ⇒ byte-identical output; affects e.g. the number of packets a random UVM test collects) |
| `--sdf <file>` `--sdf-{min,typ,max}` | Annotate SDF delays (IOPATH/INTERCONNECT) and TIMINGCHECK limits |
| `--upf <file>`, `--upf-top <instance>` | Apply IEEE 1801 power intent. See the [UPF guide](upf-guide.md) |
| `--sim-debug` | Print `[DEBUG]` / `[OPT]` diagnostics (`--sim_debug` still accepted); implies `--verbose`'s engine lines |
| `--verbose` | Internal engine lines, off by default: the version banner, `[PHASE]` timings, the end-of-run engine counters (`[PROF]`/`[FUSE]`/`[EVENT-EDGE]`/`[COV]`), compile-time notes such as `[EDGE-MERGE]` and `[CACHE]` hits, and `--compile`'s design summary; plus per-file compile progress (each file as it is parsed, and the modules/blocks it contributed). Same as `XEZIM_VERBOSE=1` |
| `--dump-files-list` | Print the fully resolved file list after `-f` expansion, then exit — confirms *which* sources a build actually reads |
| `--dump-merged-sv <file>` | Write the sources as one preprocessed, self-contained `.sv`. With `-s <top>`, keeps only the files that top needs. See the [user guide](user-guide.md#reducing-a-multi-file-build) |
| `--artifact-compression <none\|1-22>` | Compression level for the `-o` compiled artifact (`none` writes it raw) |
| `--cache-dir <dir>` | Select the automatic elaborated-design cache directory |
| `--no-cache` | Disable the automatic elaborated-design cache |
| `-l`, `--log <file>` | Redirect all stdout/stderr — including DPI/VPI C output — to a log file |
| `--tee <file>` | Copy stdout/stderr, including DPI/VPI C output, to a log file while retaining each stream on the console |
| `-v <file>` | Library file: modules compiled only to resolve unresolved instantiations |
| `-y <dir>` | Library directory: `<module>.<ext>` loaded on demand |
| `+libext+<ext>+…` | Extension list for `-y` search (replaces the default `.v`/`.sv`/`.V`) |
| `+nospecify` | Suppress specify-block path delays and timing checks — zero-delay gate simulation (`-nospecify` also accepted) |
| `+notimingcheck` | Disable the specify-block timing checks (`$setup`, `$hold`, `$width`, …; also `+notimingchecks`/`-notimingchecks`). A violation otherwise prints one `** Error:` line (counted by `--error-exit`) and toggles the check's notifier |
| `+no_notifier` / `+no_tchk_msg` | Report timing violations without toggling notifiers / toggle notifiers without reporting |
| `+delay_mode_zero` / `+delay_mode_unit`, `+mindelays` / `+typdelays` / `+maxdelays` | Gate-level delay modes and min/typ/max selection |
| `-vcd [dumpon]` | Enable source-driven VCD dumping, like `--wave`; the optional compatibility token `dumpon` is consumed |
| `--wave` | Compile the model with waveform support, enabling `$dumpfile`/`$dumpvars` (off by default; `--fst`/`--xtrace` imply it) |
| `--fst <file>` | Emit an FST (GTKWave binary) waveform dump |
| `--fst-scope <hier>` | Restrict the FST dump to signals under `<hier>` (repeatable). `LEVEL:<hier>` limits the depth: `1:top.u1` dumps only `top.u1`'s own signals, `2:top.u1` one level of children too, and `0:` or no prefix every level (§21.7.1.4) |
| `-fst_scope_file <file>` (also `--fst-scope-file`, `=` form) | FST scopes from a file, each optionally `LEVEL:SCOPE`: one or more per line, separated by spaces or commas, with `#` or `//` comments. Each adds a `--fst-scope`. Works in `-f`/`-F` files, where a relative path resolves as given, else next to the args file |
| `--fst-repack[=N]` | gzip-wrap the finished FST dump on a clean close, as GTKWave's repack-on-close does (deflate level `N`, 1..9, default 6). About 1.5x smaller on top of the per-block LZ4; readers inflate it on open. A run that dies before the close leaves the unwrapped, crash-safe dump |
| `-xezim_env <file>` (also `--xezim-env`, `=` form) | Set `XEZIM_*` variables from a file before xezim reads any of them: `NAME=value`, `export NAME=value`, `setenv NAME value`, `NAME value`, `unsetenv NAME`; `#`/`//` comments, quoted values. Values override the shell; several files apply in order. Command line only (an args file is read too late) |
| `--xtrace <file>` | Emit an XTrace v1.0 dump (`.zst`/`.zstd` ⇒ zstd-compressed) |
| `--xtrace-scope <hier>` | Restrict the XTrace dump to signals under `<hier>` (repeatable) |
| `--relax-implicit-static` | Accept `int x = ...;` inside a static task/function (§6.21) with a warning instead of an error — for third-party sources you cannot edit |
| `--relax-driver-conflicts` | Report a variable with conflicting drivers — two continuous assigns, a continuous assign plus procedural writes, or an `always_comb`/`always_latch`/`always_ff` variable also written by another process (§6.5, §9.2.2) — as a warning instead of an error, so existing designs (e.g. an `initial` block that initializes an `always_ff` register) still run |
| `--relax-duplicate-decls` | Ignore a second declaration of a name in a task, function, class method or `begin`/`fork` block, keeping the first's type and initializer (§3.13), with a warning — for sources written against tools that accept it |
| `--relax-enum-no-base` | Read `enum [N:0] {...}`, a packed dimension with no base type, as `enum bit [N:0]` (§6.19), with a warning — for sources written against tools that accept it |
| `--relax-static-const` | Read `static const` / `automatic const` in a task, function or block as `const static` (§6.20.6), with a warning — for sources written against tools that accept it |
| `--relax-event-this` | Read `@this.ev` without parentheses as `@(this.ev)` (§9.4.2), with a warning — for sources written against tools that accept it |
| `--relax-casex-inside` | Read `casex (e) inside` and `casez (e) inside` as `case (e) inside` (§12.5.4), with a warning — for sources written against tools that accept it |
| `--relax-macro-space` | Read white space between a backtick and a macro or directive name (`` ` NAME ``) as none (§22.5.1), with a warning — for sources written against tools that accept it |
| `--relax-nul-bytes` | Read a NUL byte in the source as white space (§5.3), with a warning — for sources written against tools that accept it |
| `--relax-cross-hier-item` | Accept a hierarchical reference as a cross item (`cross vif.mode, cp;`), with an implicit coverpoint (§19.6), with a warning — for sources written against tools that accept it |
| `--keywords <ver>` | Keyword set outside `` `begin_keywords `` regions (§22.14): `1364-1995`, `1364-2001`, `1364-2005`, `1800-2005` … `1800-2023`, each also with `-noconfig`; an explicit region still wins |
| `--no-config-keywords` | Read the ten configuration keywords (`instance`, `cell`, `use`, `library`, …) as names outside `` `begin_keywords `` regions |
| `--error-exit` | Exit nonzero if any `$error` was reported (`$fatal` always does) |
| `--code-coverage[=<kinds>]` | Collect code coverage: `stmt`, `branch`, `toggle` (comma-separated) or `all` (the default). Results go to `xezim_cov.json` next to the functional coverage; `--verbose` adds a per-instance summary. See the [coverage guide](coverage-guide.md#code-coverage) |
| `--code-coverage-scope=<path>[,<path>...]` | Only cover these instance subtrees (`tb.dut`) and packages (repeatable) |
| `--profile` | Print the `[PROF]` end-of-run profile report (edge-block, settle and timing counters) together with the `--verbose` engine lines. Same as `XEZIM_PROFILE_REPORT=1`. Adds overhead |
| `--show-env-avail` | List every `XEZIM_*` environment variable with a one-line description, then exit |

## Environment variables

Selected variables; `xezim --show-env-avail` lists them all. Off by default
unless noted.

| Variable | Effect |
|---|---|
| `XEZIM_EVENT_EDGE=0` | On by default: clocked flop fires whose data inputs have not changed are skipped (1.13–1.30× wall on the C910/C906 benchmarks). `0` turns it off |
| `XEZIM_PACKED_MEM=0` | On by default: large integral memories (elements up to 64 bits, arrays of more than 100,000 cells) live in a packed arena. `0` turns it off |
| `XEZIM_CLASS_GC=0` | On by default: class objects that no handle reaches are reclaimed, with their owned storage, at a time advance once 20,000 objects (or the previous live count, or half the objects ever allocated, if larger) have been allocated since the last collection, and never while a DPI task is suspended (its C stack cannot be traced). `0` turns it off; `stress` collects at every time advance that allocated an object |
| `XEZIM_COMPILE_METHODS=1` | On by default (see `XEZIM_METHOD_TIER`): compile class methods, wait-free class tasks and package or module functions to bytecode and execute them. `0` forces the AST interpreter |
| `XEZIM_METHOD_TIER=N` | Compile a class method (function or wait-free task) or a package/module function only after N calls (default 1000; `0` compiles on the first call). A subroutine that never crosses the threshold stays on the interpreter with no compilation overhead |
| `XEZIM_FAST_CALLS=1` | On by default: direct VM-to-VM dispatch of compiled methods without re-entering the interpreter's formal binding loop. `0` disables it |
| `XEZIM_METHOD_CACHE=<dir>` | Persistent compiled-method cache directory across runs. `1` uses `~/.cache/xezim/method-cache`; unset (default) disables it. Corrupt entries transparently trigger recompilation |
| `XEZIM_METHOD_PROFILE=1` | In-process sampling profiler for class methods. When `1`, prints AST-vs-bytecode execution-time histograms on simulation completion (default off) |
| `XEZIM_FALLBACK_SITES=1` | Log each construct handed to the AST interpreter: the reason, source byte span and scope of the compile-fail / plan-decline site (default off) |
| `XEZIM_JIT=1` | Compile bytecode blocks to machine code in-process (needs a `--features jit` build) |
| `XEZIM_AOT=1` | Compile eligible blocks to native code via generated Rust + `rustc` instead of cranelift. **Requires `XEZIM_JIT=1` as well** — on its own it is a no-op. Needs `--features jit`. See the [user guide](user-guide.md#native-compilation) |
| `XEZIM_AOT_OPT=0..3` | `rustc` optimization level for the generated crate (default 2) |
| `XEZIM_PROC_FSM=1` | Compile blocking `always` bodies into bytecode state machines with wait instructions |
| `XEZIM_NO_NATIVE_CACHE=1` | Disable the persistent native-library cache (`~/.cache/xezim/native`) |
| `XEZIM_REGIONS=1` | Fuse dependency-connected compiled combinational entries into region blocks (experimental; currently net-negative on the benchmark set) |
| `XEZIM_STUCK_CLOCK=1` | Flag a process parked on a clock/reset that never changes while the design keeps churning edges (`abort` variant for CI) |
| `XEZIM_INIT_REG=0\|random` | Give registers (the flops of compiled clocked blocks) a defined value at time 0 instead of x: `0`, or `random`, a per-register pattern that changes with the run's seed (`+seed=<n>`, `-sv_seed <n>`; no seed and seed 1 give the same pattern), for reset-bug hunting |
| `XEZIM_INIT_ZERO=1` | Coerce X-initialized signals/arrays to 0 (required for some C910/C906 workloads, e.g. CoreMark) |
| `XEZIM_FST_FLUSH_SECS=N` | `--fst`: write the in-memory value-change block to the file at least every N wall seconds (default 2; fractions allowed; `0` flushes only when the block reaches 64 MB). The first block is written as soon as the run starts. A run that is killed at any moment (`kill -9`, out of memory, a second Ctrl-C) keeps everything up to the last flush, and the file stays readable: each block reaches the file whole or not at all |
| `XEZIM_PROGRESS=N` | Emit a `[PROGRESS]` line every N wall-seconds (sim_time, iters, edges_fired, nba_q) |
| `XEZIM_CACHE_DIR=<dir>` | Override the elaborated-design cache directory |
| `XEZIM_NO_CACHE=1` | Disable the automatic elaborated-design cache |
| `XEZIM_COMPILE_PHASES=1` | Report detailed simulator compilation phase timings |
| `XEZIM_ALLOW_IMPLICIT_STATIC=1` | Same as `--relax-implicit-static` |
| `XEZIM_RELAX_DRIVER_CONFLICTS=1` | Same as `--relax-driver-conflicts` |
| `XEZIM_RELAX_DUPLICATE_DECLS=1` | Same as `--relax-duplicate-decls` |
| `XEZIM_RELAX_ENUM_NO_BASE=1` | Same as `--relax-enum-no-base` |
| `XEZIM_RELAX_STATIC_CONST=1` | Same as `--relax-static-const` |
| `XEZIM_RELAX_EVENT_THIS=1` | Same as `--relax-event-this` |
| `XEZIM_RELAX_CASEX_INSIDE=1` | Same as `--relax-casex-inside` |
| `XEZIM_RELAX_MACRO_SPACE=1` | Same as `--relax-macro-space` |
| `XEZIM_RELAX_NUL_BYTES=1` | Same as `--relax-nul-bytes` |
| `XEZIM_RELAX_CROSS_HIER_ITEM=1` | Same as `--relax-cross-hier-item` |
| `XEZIM_KEYWORDS=<ver>` | Same as `--keywords <ver>` |
| `XEZIM_NO_CONFIG_KEYWORDS=1` | Same as `--no-config-keywords` |
| `XEZIM_PROFILE_REPORT=1` | Same as `--profile` |
| `XEZIM_VERBOSE=1` | Same as `--verbose` (for scripts that grep `[PHASE]`/`[PROF]` lines without adding a flag) |
| `XEZIM_CODE_COVERAGE=<kinds>` | Same as `--code-coverage=<kinds>`; the flag wins |
| `XEZIM_COV_DB=<path>` | Write the coverage results somewhere other than `xezim_cov.json` |
| `XEZIM_MAX_INST_DEPTH=N` | Instantiation-depth cap (default 200) — turns unbounded recursive instantiation into a clean error instead of memory exhaustion |
| `XEZIM_STACK_MB=N` | Stack size of the simulation worker thread (default 1024; `0` runs on the main thread) |
| `XEZIM_UVM_DIR=<dir>` | The UVM library `-uvm` adds: a `src` directory, a release root holding `src/`, or a checkout holding several releases (`1.1d`, `1.2`, `1800.2-2017`, `1800.2-2020`) |
| `XEZIM_UVM_VERSION=<release>` | With a multi-release `XEZIM_UVM_DIR`, the release subdirectory to use (default: the newest present) |
| `XEZIM_VALUE_TRACE=<substr>[,...]` | Print every committed change of signals whose hierarchical name contains a pattern: time, name, old→new value, dispatch phase, writing process origin (file:line). NBA commits are labeled `nba` |
| `XEZIM_VALUE_TRACE_LIMIT=N` | Cap value-trace output lines (default 20000) |

## Command lines from other simulators

One xezim invocation accepts the usual compile and simulate spellings of
commercial simulators, so a flow's existing arguments can be pasted into a
single command. Libraries are not persistent — every run compiles its
sources — so the library options are accepted and ignored:

```bash
xezim -sv +define+UVM_NO_DPI+DEPTH=4 +incdir+tb+rtl -F files.f -work work \
      -c -quiet -lib work hdl_top hvl_top +UVM_TESTNAME=my_test \
      -sv_seed 42 -gDEPTH=8 -l run.log -do "run -all; quit -f"
```

| Spelling | Behaviour in xezim |
|---|---|
| `<top> …`, `work.<top>` | A bare design-unit name that is not an existing file names a top module, same as `-s <top>`. Several give several tops. `<lib>.<top>` works for `work` and libraries named by an earlier `-work`/`-L`/`-lib` |
| `+define+A+B=1`, `+incdir+d1+d2` | Several macros / directories in one flag |
| `-f <file>`, `-file <file>` | Args file; relative file names resolve as given, else against the args file's directory |
| `-F <file>` | Same as `-f`, except that `+incdir+` directories inside it resolve against the args file's directory first, so `+incdir+.` names the file's own directory |
| `-do "<cmds>"`, `-do <file>` | A subset of the command language: `run -all` (until `$finish` or no events remain), `run <n><unit>` (`fs`…`sec`; several `run`s add up; the run ends at that time, which `final` blocks and the closing line report), `quit`/`exit` (`-f`, `-force`), `do <file>`, separated by `;` or newlines, `#` comments. `log`, `add wave`, `coverage save` and `coverage report` are accepted with one warning each and do nothing. Any other command is an error. A script that quits before any `run` only elaborates. `--max-time` stays a hard cap |
| `-gNAME=VAL` | Sets the default of parameter `NAME` in every module, interface or program that declares it overridable (a `string` parameter takes an unquoted value as text); a value given at an instantiation or by `defparam` still wins. Parameters in generate blocks, and body `parameter`s of a module that has a parameter port list, are local and not reached. `-g/<top>/NAME=VAL` limits it to module `<top>`; deeper paths are ignored with a warning. A name no module declares is warned about and ignored |
| `-GNAME=VAL` | Like `-g`, and it also replaces values given at instantiations and by `defparam` |
| `-sv_seed <n>`, `-sv_seed random` | Same as `+seed=<n>` / `+seed=random` |
| `-sv_lib <name>`, `-sv_root <dir>` | Load `<dir>/<name>.so` as a DPI library (`--dpi-lib`) |
| `-timescale <u>/<p>` | Default timescale for design elements without one (`--module-timescale`) |
| `-override_timescale <u>/<p>` (also `--override-timescale`, `=` form) | One timescale for every design element, package and compilation unit: every `` `timescale `` directive, `timeunit`/`timeprecision` declaration and `--module-timescale` is ignored. Bare delays count in `<u>`; literals with a unit (`#3ns`) keep their absolute value |
| `-l <file>`, `-logfile <file>` | xezim's `-l`: all output goes to the file, none to the terminal |
| `-c` | xezim's args-file flag when a file (or a path) follows; otherwise the batch-mode switch, accepted |
| `-v <file>`, `-y <dir>`, `+libext+` | Library file / directory |
| `+notimingchecks` | Disable timing checks |
| `-sv12compat`, `-sv17compat` | Same as `--sv2017`. `-sv05compat`/`-sv09compat` do the same with a warning |
| `-work`, `-L`, `-Lf`, `-lib <lib>` | Ignored, with one warning |
| `+cover`, `+cover=<letters>`, `-coverage` | Code coverage (`--code-coverage`): `s`, `b` and `t` select statement, branch and toggle coverage; the bare forms select all three. Other letters (`c`, `e`, `f`, `x`) are ignored with one warning |
| `-sv`, `-mfcu`, `-quiet`, `-64`, `-32`, `-batch`, `-nologo`, `+acc[=…]`, `-<step>args=…` (arguments for a separate optimization step), `-suppress <ids>`, `+fcover`, `-sva`, `-assertdebug` | Accepted, no effect: SystemVerilog is always on, all files share one compilation unit, every object stays visible, assertions and covergroups are always evaluated |
| `-sfcu`, `-t <res>`, `-wlf <file>` | Accepted with a warning: xezim always uses one compilation unit and the finest precision declared in the design, and does not write that waveform file (`--fst` / `--wave` dump waveforms) |

Where a spelling means something else in xezim, xezim's meaning is kept:
`-l` redirects rather than copies the transcript; `-c <file>` reads an args
file. The glued forms `-s<top>`, `-l<file>` and `-f<file>` lose to the
spellings above (`-sv`, `-sv_seed`, `-suppress`, `-lib`, `-logfile`,
`-file`); use `-s <top>` / `-l <file>` / `-f <file>` with a space for such
names.
