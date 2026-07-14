# Clause 20 — Utility system tasks/functions: compliance tests

Self-checking SystemVerilog tests for the Clause-20 features that xezim now
implements. They are written to the IEEE 1800-2023 LRM so they can be run on
**any** reference simulator and on xezim itself.

## Files

| File | LRM | Covers |
|------|-----|--------|
| `41_severity_tasks.sv`     | 20.10 | `$info`, `$warning`, `$error` — message + **continue** (no terminate) |
| `42_severity_fatal.sv`     | 20.10 | `$fatal` — **terminates** simulation (special pass criteria) |
| `43_stochastic_queue.sv`   | 20.15 | `$q_initialize`, `$q_add`, `$q_remove`, `$q_full`, `$q_exam` (FIFO/LIFO + status codes) |
| `44_system.sv`             | 20.17.1 | `$system` (task+function+no-arg, host-shell round-trip) |
| `45_real_math_trig.sv`     | 20.8.2 | trig + hyperbolic set (`$sin` … `$atanh`, `$hypot`, `$atan2`) |
| `46_stacktrace.sv`         | 20.17.2 | `$stacktrace` (function + task form) — **newer LRM feature, optional** |

They reuse the shared macros in `../common/svtest_defs.svh`:

- `SVTEST_INIT` — declares the `failures` counter.
- `SVTEST_CHECK(expr, msg)` — counts a failure on a false `expr`.
- `SVTEST_PASSFAIL` — prints `TEST_PASS` / `TEST_FAIL count=N` (and `$fatal` on fail).

## Pass criteria

### Normal self-checking tests (`41`, `43`, `44`, `45`)

- stdout contains a line `TEST_PASS`
- simulator exit code is **not** used as a criterion (see caveat below)

### `42_severity_fatal.sv` (special)

`$fatal` **must** terminate, so this test deliberately does **not** print
`TEST_PASS`. It passes iff **all** of:

- `BEFORE_FATAL` **is** printed (ran up to the `$fatal`)
- `SHOULD_NOT_REACH_HERE` **is not** printed (stopped at `$fatal`)
- `TEST_PASS_IF_FATAL_DID_NOT_TERMINATE` **is not** printed (stopped at `$fatal`)

The simulator **exit code is intentionally NOT a criterion**. The LRM (20.10)
says `$fatal` terminates "with an error code", but simulator driver wrappers
vary in whether they propagate that exit status; some return 0 even after a
`$fatal` despite reporting a fatal error in their transcript. The output-based
checks above are authoritative and tool-independent.

### `46_stacktrace.sv` (optional — newer LRM feature)

`$stacktrace` is a relatively newer LRM feature. **Older simulator releases
(released before the feature was added) do NOT support it** — the design fails
to load. This is expected, not a bug. Run it against a NEWER tool that
implements the feature, or use the out-of-the-box Verilator target. It is **not**
part of the default `make run_all`.

## Running

```sh
cd xezim/tests/sv_compliance/tests_ch20_systf

# core suite (41/42/43/44/45) against any reference simulator:
make run_all SIM="<your simulator> <flags>"

# optional $stacktrace test (newer feature):
make run_stacktrace_vl                   # Verilator (out of the box)
make run_stacktrace SIM="<your simulator> <flags>"   # newer tool that supports the feature

# single test:
make run TEST=45_real_math_trig SIM="<your simulator> <flags>"

# against xezim itself:
make run_all SIM="/path/to/xezim"
make run_stacktrace SIM="/path/to/xezim"
```

## Why the exit code is not a pass criterion

The self-checking tests (`41/43/44/45`) deliberately call `$error`/`$warning`,
which inflate the simulator's error/warning counters and make some driver
wrappers exit NONZERO even on a correct run. The single source of truth is the
`TEST_PASS` marker printed by `SVTEST_PASSFAIL` — on failure it calls `$fatal`
and `TEST_PASS` never appears. The Makefile matches `TEST_PASS` as a whole
word, tolerating a `# ` transcript prefix while correctly *excluding*
`TEST_PASS_IF_FATAL_DID_NOT_TERMINATE` (used by test 42's contract).

## Implementation-defined behaviour (asserted weakly in 43)

Per LRM Table 20-10 the stat codes 2/4/5/6 for `$q_exam` are not pinned down
precisely, and simulators differ. Test 43 only **hard-asserts** the
deterministic parts and **reports** (via `INFO:`) the tool's actual value for
the implementation-defined codes:

| Code | Meaning | Asserted |
|------|---------|----------|
| 1 | current queue length | exact value |
| 3 | maximum queue length (high-water) | exact value |
| 2,4,5,6 | mean interarrival / shortest wait / longest wait / average wait | status==0 only; actual value printed as INFO |

Likewise `$q_initialize(max_length<=0)` (LRM status 5) is **reported**, not
hard-asserted: some tools accept `max_length==0` (return 0) while others (and
the LRM) reject it. xezim follows the LRM and rejects any length `<= 0`.

## Tool-default caveats

- **`$info`/`$warning`/`$error`/`$fatal` message text** (file/line/hier/time
  preamble) is tool-specific per 20.10 — only continuation/termination is
  asserted, not the printed text.

  > Per LRM §20.10 **only `$fatal` terminates**. Conformant simulators all
  > continue past `$error`/`$warning`/`$info` by default. **Verilator**
  > non-conformantly aborts on `$error`, so do **not** use Verilator for
  > `41_severity_tasks`.

- **`$stacktrace` content** (20.17.2) is implementation-dependent — only that
  the function form returns a value assignable to a `string` and that execution
  continues is asserted; the test prints `INFO:` lines with the length and a
  short sample for cross-tool capture.

## Host assumptions

- `44_system.sv` uses `$system("printf 'Z' > file")` and `$system("rm -f file")`,
  which require a POSIX `/bin/sh` with `printf` and `rm` (Linux/macOS). On
  Windows use a POSIX shell or adapt the commands.
