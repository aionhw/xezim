# Clause 21 — I/O system tasks/functions: compliance tests

Self-checking SystemVerilog tests for IEEE 1800-2023 Clause 21 features,
focusing on gaps identified in the xezim implementation assessment.

## Files

| File | LRM | Covers |
|------|-----|--------|
| `51_format_specs.sv`         | 21.2.1 | `%0` zero-suppression (h/b/o), `.N` precision (f/e/g), auto-sized decimal width, field-width rules |
| `52_file_radix_output.sv`    | 21.3.2 | `$fdisplayb/h/o`, `$fwriteh` — unformatted args use task-name radix |
| `53_file_read.sv`            | 21.3.4–8 | `$fgets`, `$feof`, `$ferror`, `$fflush` |
| `54_memory_file.sv`          | 21.4–5 | `$writememh/b` round-trip via `$readmemh/b`; `$readmemd` diagnostic |
| `55_monitor_control.sv`      | 21.2.3 | `$monitoroff`/`$monitoron` pause/resume semantics |
| `56_vcd_misc.sv`             | 21.7.1 | `$dumpall`, `$dumplimit`, `$dumpflush` |

## Running

```sh
cd xezim/tests/sv_compliance/tests_ch21_io

make run_all SIM="<your simulator> <flags>"   # reference
make run_all SIM="/path/to/xezim"             # xezim itself
```

## Format-spec fixes (test 51)

These fixes were needed in the shared `format_string` engine (affect all of
`$display`/`$write`/`$fwrite`/`$swrite`/`$sformatf`):

- **`%0` zero-suppression**: hex/binary/octal now strip leading zeros when `%0`
  is used (`%0h` of `32'hFF` → `"ff"` not `"000000ff"`) per LRM §21.2.1.2.
- **`.N` precision**: real formats now parse `.<digits>` precision
  (`%.2f` of π → `"3.14"`, `%.4e` of 2.5 → `"2.5000e+00"`).
- **`%Nh` zero-padding**: explicit width on hex/binary/octal now zero-pads to
  the specified width (after stripping) rather than always showing full width.
- **Decimal auto-sizing**: `%d` (no width) now space-pads to the maximum
  decimal width for the bit-width (10 chars for 32-bit) per LRM §21.2.1.2.
- **Octal full-width**: `%o` and `$displayo` now default to full signal-width
  octal (ceil(bits/3) digits), matching other radix variants.
- **`%e` C-style exponent**: now produces `e+00` format (was Rust's `e0`).

## New system tasks implemented

| Task | LRM | Notes |
|------|-----|-------|
| `$fdisplayb/h/o`, `$fwriteb/h/o` | 21.3.2 | File-output radix variants |
| `$fflush` | 21.3.6 | Flush one (fd) or all (fd=0) file handles |
| `$fgets` | 21.3.4.2 | Read a line into a string (task + function) |
| `$feof` | 21.3.8 | EOF detection (function) |
| `$ferror` | 21.3.7 | I/O error status (task + function) |
| `$readmemd` | 21.4 | Decimal memory load (2023 feature) |
| `$writememb/h/d` | 21.5 | Memory dump to file (any radix) |
| `$monitoron` | 21.2.3 | Resume paused monitor + immediate print |
| `$dumpall` | 21.7.1.4 | VCD checkpoint of all current values |
| `$dumplimit` | 21.7.1.5 | VCD size cap (best-effort) |
| `$dumpflush` | 21.7.1.6 | Flush VCD writer buffer |

## Tool caveats

- `$readmemd` (21.4) is new in IEEE 1800-2023; older simulator releases do
  not implement it. Test 54 makes this diagnostic.
- `$fgets` includes the trailing newline; tests strip it with `substr`.
