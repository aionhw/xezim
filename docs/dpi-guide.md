# Compiling and loading DPI libraries for xezim

xezim loads **DPI-C** (Direct Programming Interface, the C-language variant of IEEE 1800 §35)
shared libraries at simulation start via `--dpi-lib`. This guide shows how to compile your
own C or C++ DPI code into a `.so` / `.dylib` / `.dll` that xezim can dlopen, and how to
wire the SystemVerilog side to it.

> For a worked end-to-end example (real Spike integration), see
> [`../dpi/spike/README.md`](../dpi/spike/README.md). For the canonical one-file
> "hello world" pairs used by the test suite, see
> [`../tests/dpi/`](../tests/dpi/).

---

## TL;DR — one-file C library

```bash
# xezim/tests/dpi/simple_dpi.c  ->  simple_dpi.so
cc -shared -fPIC -I path/to/xezim simple_dpi.c -o simple_dpi.so

# Run
xezim --dpi-lib ./simple_dpi.so simple_dpi_test.sv
```

That's it. The same recipe works for `.cc`/`.cpp` files (use `g++` instead of `cc`) and
for multi-file builds (list more sources / headers before `-o`).

---

## What xezim expects

`--dpi-lib <path>` is given a path to a **shared library** that exports the C symbols
declared by `import "DPI-C"` statements in your SystemVerilog source. There is no
ABI versioning, no manifest file, no plugin registry — `dlopen()` + `dlsym()` on the
imported names.

The minimum is:

| Element | Where it comes from |
|---|---|
| The `.so` / `.dylib` / `.dll` itself | You build it (this guide) |
| `import "DPI-C" function …` declarations | Inside your SV source |
| The matching exported C symbols | The `.so` |
| (Optional) `svdpi.h` for type/macro helpers | xezim ships it at `<repo>/svdpi.h` |
| (Optional) `vpi_user.h` for VPI calls | xezim ships it at `<repo>/vpi_user.h` |

The two headers are **minimal** — they're enough for the test suite and for the
non-vendor DPI subset used by Accellera UVM (`uvm_core/src/dpi/`). They're not a full
re-implementation of IEEE 1800 §35/§38; treat missing typedefs as a feature gap to
report upstream rather than papering over with your own.

---

## The two header files in the repo root

xezim ships two headers at the top of the repo so you can compile DPI code without
installing a vendor simulator first:

* **`svdpi.h`** — the SystemVerilog DPI types and macros (`svBitVecVal`,
  `svOpenArrayHandle`, the `SV_PUBLIC` visibility macro, the `DPI_CONTEXT`
  attribute helper).
* **`vpi_user.h`** — a thin subset of the Verilog Procedural Interface
  (`vpi_handle_by_name`, `vpi_get_value`, `vpi_put_value`, `s_vpi_value`, the
  `vpiIntVal` / `vpiHexStrVal` format constants, the standard
  `vpiModule` / `vpiReg` / `vpiMemory` type codes, etc.). Used by the
  HDL-backdoor family of DPI exports (`vpi_backdoor_compliance.c`).

The canonical include incantation is `-I <path/to/xezim>` so both headers are
found by their unqualified `#include "svdpi.h"` / `#include "vpi_user.h"`.

---

## Minimal working example — one C file, one SV file

`simple_dpi.c`:

```c
#include <stdint.h>

int add_c(int a, int b) {
    return a + b;
}
```

`simple_dpi_test.sv`:

```systemverilog
module simple_dpi_test;
  import "DPI-C" function int add_c(input int a, input int b);

  initial begin
    $display("DPI_RESULT=%0d", add_c(20, 22));
    if (add_c(20, 22) != 42) begin
      $display("TEST_FAIL");
      $finish;
    end
    $display("TEST_PASS");
    $finish;
  end
endmodule
```

Build and run:

```bash
cc -shared -fPIC -I . simple_dpi.c -o simple_dpi.so
xezim --dpi-lib ./simple_dpi.so simple_dpi_test.sv
# DPI_RESULT=42
# TEST_PASS
```

This is the exact recipe used by
[`tests/dpi_integration_tests.rs`](../tests/dpi_integration_tests.rs)'s
`compile_dpi_lib` helper.

---

## Compiling C++ sources

Same flags, swap the compiler and add `-std=c++17` (or whatever you need):

```bash
g++ -shared -fPIC -std=c++17 -I path/to/xezim dpi_module.cc -o dpi_module.so
```

The `extern "C"` wrapper that surrounds your `import "DPI-C"` implementations is the
caller's responsibility — the C ABI of the DPI surface is what `dlsym` looks up.
The xezim DPI loader does **not** do C++ name-mangling recovery.

`xezim/dpi/spike/xezim_spike_dpi.cpp` shows the standard layout: anonymous-namespace
state, an `extern "C" { … }` block of `import`ed symbols, optional `#ifdef` blocks
to compile the same source against an optional real backend (Spike's
`libriscv.so`) or in a stub-only mode.

---

## Multi-file libraries — the UVM DPI case

Accellera's UVM reference (`uvm-core/src/dpi/`) is shipped as a bag of `.c` and
`.cc` files plus a single `.svh` for the SV-side imports. There is no Makefile
in the upstream kit — the consumer compiles them. The recipe is just:

```bash
# All C files compile as C, all .cc files as C++.
# Link them all into one .so.

cc  -shared -fPIC -I path/to/xezim \
    uvm_common.c uvm_hdl.c uvm_svcmd_dpi.c uvm_hdl_polling.c \
    -c -o uvm_c.o

g++ -shared -fPIC -std=c++17 -fno-inline -I path/to/xezim \
    -I path/to/uvm-core/src/dpi \
    uvm_dpi.cc uvm_regex.cc \
    -c -o uvm_cc.o    # only if you don't use uvm_dpi.cc's own #include chain

# Single shared library
g++ -shared -fPIC \
    uvm_c.o uvm_cc.o \
    -o libuvm_dpi.so
```

> **Practical note:** `uvm_dpi.cc` already `#include`s every `.c` and `.cc` source
> from `uvm-core/src/dpi/` inside its own `extern "C" { … }` block. That means a
> **single-file compile** works for the whole kit:
>
> ```bash
> g++ -shared -fPIC -std=c++17 \
>     -I path/to/xezim -I path/to/uvm-core/src/dpi \
>     path/to/uvm-core/src/dpi/uvm_dpi.cc \
>     -o libuvm_dpi.so
> ```
>
> xezim ships with `-DUVM_NO_DPI` so the UVM SV source itself never calls into
> this `.so` (UVM reporting / cmdline is serviced by the Rust core), but having
> the library available is useful for *your own* DPI extensions that piggy-back
> on the UVM header conventions.

---

## Linking against an external C/C++ library

Most non-trivial DPI shims depend on something — a CPU model, a cocotb plugin,
a regex engine, a compression codec. Pattern:

```bash
# 1) Compile your shim to an object file
g++ -shared -fPIC -std=c++17 -fPIC -DXEZIM_SPIKE_REAL=1 \
    -I path/to/xezim -I $SPIKE_PREFIX/include \
    -c xezim_spike_dpi.cpp -o xezim_spike_dpi.o

# 2) Link the shim + the external library into one .so
g++ -shared -fPIC \
    -L $SPIKE_PREFIX/lib -Wl,-rpath,$SPIKE_PREFIX/lib \
    xezim_spike_dpi.o \
    -lriscv -lfesvr -lsoftfloat \
    -o xezim_spike_dpi.so
```

Rules of thumb:

* **Libraries go after sources** in the link line (`xezim_spike_dpi.o -lriscv`,
  not `-lriscv xezim_spike_dpi.o`). The GNU linker resolves left-to-right and
  only pulls objects out of a `-l` library if they're needed to satisfy an
  unresolved symbol seen *before* it on the command line.
* **`-Wl,-rpath,<dir>`** bakes the runtime search path into the `.so`, so the
  loader finds `libriscv.so` even if `LD_LIBRARY_PATH` isn't set when xezim
  starts. Without it, every user has to set `LD_LIBRARY_PATH` by hand or get
  an `cannot open shared object file` error at `dlopen` time.
* **Don't use `-static`**. Static linking prevents `dlopen` from resolving the
  import symbols (or makes the result non-relocatable in weird ways). Always
  `-shared`.
* **Visibility**: `svdpi.h` defines `SV_PUBLIC` as
  `__attribute__((visibility("default")))`. Wrap your DPI exports with it so
  the linker doesn't hide them in `-fvisibility=hidden` builds:
  ```c
  SV_PUBLIC int my_dpi(int x) { … }
  ```

The full Spike shim Makefile (`xezim/dpi/spike/Makefile`) demonstrates all of
these in working form, including a stub-mode build that needs no external
library.

---

## Headers: where to put your `.svh`

Two equally-good conventions. Pick one and be consistent:

**Convention A — alongside the `.c` source.** Drop `my_dpi.svh` next to
`my_dpi.c` and the testbench. Consumers `\``include "my_dpi.svh"` after
adding the dir to `-I`:

```
my_project/
├── my_dpi.c
├── my_dpi.svh        # import "DPI-C" … declarations
├── tb_my_dpi.sv      # `include "my_dpi.svh"
```

```bash
cc -shared -fPIC -I path/to/xezim -I . my_dpi.c -o my_dpi.so
xezim --dpi-lib ./my_dpi.so -I . tb_my_dpi.sv
```

**Convention B — install into a shared `dpi/include/`.** Better when you have
multiple DPI libs sharing one import header (`uvm_dpi.svh`-style):

```
dpi/
├── include/uvm_dpi.svh
└── lib/libuvm_dpi.so
```

```bash
g++ -shared -fPIC -I path/to/xezim -I dpi/include uvm_dpi.cc -o dpi/lib/libuvm_dpi.so
xezim --dpi-lib dpi/lib/libuvm_dpi.so -I dpi/include tb.sv
```

xezim's own `dpi/spike/` follows Convention A.

---

## Running with xezim

```bash
xezim --dpi-lib /abs/path/to/libfoo.so [more --dpi-lib paths …] <sv files>
```

* Repeatable: pass `--dpi-lib` once per shared library. Each is `dlopen`ed and
  its symbols added to the same dlsym table.
* `RTLD_NOW | RTLD_GLOBAL` is used, so transitive deps must resolve at
  load time — set `LD_LIBRARY_PATH` if your `.so` has rpath-less deps, or
  pass `-Wl,-rpath,$PREFIX/lib` at link time.
* The SV file must `import "DPI-C" function …` (or include a `.svh` that does)
  for every symbol you call from SV. Symbols that exist in the `.so` but
  aren't imported are simply ignored — there's no eager validation.
* `import "DPI-C"` must appear in module/program/interface scope (or inside a
  package), not at `$unit` scope. xezim today requires it inside a module —
  see `dpi/spike/test_spike_dpi.sv`.

---

## Cross-platform notes

| Platform | Shared-object ext. | Compiler | One-file recipe |
|---|---|---|---|
| Linux | `.so` | `cc` / `g++` | `cc -shared -fPIC -I . foo.c -o foo.so` |
| macOS | `.dylib` | `cc` / `clang++` | `cc -shared -fPIC -I . foo.c -o foo.dylib` |
| Windows | `.dll` | `cl.exe` (MSVC) or `gcc` (MinGW) | `cl /LD /I . foo.c /Fe:foo.dll` |

xezim's `--dpi-lib` accepts the platform-correct extension automatically. On
macOS you may need `DYLD_LIBRARY_PATH` set instead of `LD_LIBRARY_PATH`. On
Windows, MSVC-produced DLLs need the corresponding `.lib` import library
available at link time of any consumer binary (xezim itself doesn't, because
it only `dlopen`s).

---

## Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `failed to load DPI library 'foo.so': …cannot open shared object…` | Runtime can't find a transitive dep | Add `-Wl,-rpath,<dir>` at link time, or set `LD_LIBRARY_PATH` |
| `undefined symbol: my_dpi_fn` | Imported name doesn't match exported name (C++ mangling, missing `extern "C"`, missing `SV_PUBLIC`) | Wrap in `extern "C"`, mark `SV_PUBLIC`, ensure the `.c`/`.cc` actually compiles the symbol in |
| `ImportError: …failed to run xezim: No such file or directory` from `cargo test` | The test harness uses `env!("CARGO_BIN_EXE_xezim")` — make sure the bin was built first | `cargo build --tests` then run; the env var is set at compile time |
| Symbol resolves but the call returns garbage | ABI mismatch (e.g. `int` vs `int64_t`, `char*` lifetime) | DPI imports must match the C signature exactly; for `string` returns, the buffer must outlive the call site |
| `failed to resolve path` from a VPI call | `vpi_handle_by_name` only knows what's been elaborated into a signal | Make sure the path matches an elaborated signal name; unpacked-struct member access needs the full dotted path |

Run xezim with `--sim_debug` for `[DEBUG]` lines that show symbol resolution,
`vpi_handle_by_name` lookups, and the active DPI library list.

---

## Reference: what the test suite compiles

The integration testsuite
([`tests/dpi_integration_tests.rs`](../tests/dpi_integration_tests.rs))
exercises five patterns end-to-end:

| Test | Source | Demonstrates |
|---|---|---|
| `dpi_simple_test` | `simple_dpi.c` | One function, `int` in/out |
| `dpi_extended_test` | `extended_dpi.c` | 64-bit ints, doubles, pointers, strings |
| `dpi_logic_vec_test` | `logic_vec_dpi.c` | `svBitVecVal` packed-vector in/out |
| `dpi_open_array_test` | `open_array_dpi.c` | `svOpenArrayHandle` unpacked arrays |
| `dpi_shortreal_string_test` | `shortreal_string_dpi.c` | `shortreal` plus C string returns |
| `dpi_vpi_backdoor_compliance_test` | `vpi_backdoor_compliance.c` | VPI force/read of signal hierarchies |

All six are built with the same `cc -shared -fPIC -I <xezim_dir>` invocation —
no per-test build system, just six shell-out-to-`cc` calls inside the test
harness.

---

## See also

* [`../dpi/spike/README.md`](../dpi/spike/README.md) — worked example with a real
  external library (Spike / riscv-isa-sim), including stub-mode and real-mode
  builds.
* [`uvm-guide.md`](uvm-guide.md) — running UVM testbenches on xezim (the
  `-DUVM_NO_DPI` flag there means the UVM library itself doesn't call into
  a DPI `.so`; your own DPI extensions still can).
* [`../tests/dpi/`](../tests/dpi/) — the canonical one-`.c`-per-test pairs.