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
clang -shared -fPIC -I path/to/xezim/include simple_dpi.c -o simple_dpi.so

# Run
xezim --dpi-lib ./simple_dpi.so simple_dpi_test.sv
```

That's it. The same recipe works for `.cc`/`.cpp` files (use `clang++`
instead of `clang`) and for multi-file builds (list more sources / headers
before `-o`).

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
| (Optional) `svdpi.h` for type/macro helpers | xezim ships it at `<repo>/include/svdpi.h` |
| (Optional) `vpi_user.h` for VPI calls | xezim ships it at `<repo>/include/vpi_user.h` |
| (Optional) `sv_vpi_user.h` for 4-state vectors and SV scope primitives | xezim ships it at `<repo>/include/sv_vpi_user.h` |
| (Optional) `veriuser.h` for legacy PLI v1.0 typedefs | xezim ships it at `<repo>/include/veriuser.h` |

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

The canonical include incantation is `-I <path/to/xezim/include>` so all four
headers are found by their unqualified `#include "svdpi.h"` /
`#include "vpi_user.h"` / `#include "sv_vpi_user.h"` /
`#include "veriuser.h"`. The `include/` subdirectory keeps them separate
from xezim's own source tree so a wide `-I path/to/xezim/include` can't accidentally
shadow anything else.

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
clang -shared -fPIC -I . simple_dpi.c -o simple_dpi.so
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
clang++ -shared -fPIC -std=c++17 -I path/to/xezim/include dpi_module.cc -o dpi_module.so
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

You don't need a library to run UVM. xezim provides UVM's DPI-C helpers
natively: regex matching, command-line processing and `uvm_hdl_*` backdoor
access. So a UVM testbench runs the same with or without `-DUVM_NO_DPI`. Build
Accellera's C code yourself only when you want that implementation itself, or
a base for your own DPI extensions that follow the UVM header conventions.

Accellera's UVM reference (`src/dpi/` in the 1800.2 release kits and in
`uvm-core`) ships as a set of `.c` and `.cc` files plus the SV-side imports.
Its own driver, `uvm_dpi.cc`, `#include`s every source inside one
`extern "C"` block. That C linkage matters: SV binds DPI imports and exports by
their C names. The sources reference an SV export, `m__uvm_report_dpi`, through
a plain `extern` declaration, so compiled as C++ outside such a block that
reference is mangled, and the library cannot be loaded (`undefined symbol:
_Z17m__uvm_report_dpi...`).

`uvm_dpi.cc` can't be used as is: it also `#include`s `uvm_hdl.c`, whose
per-simulator `#ifdef` chain ends in `#error "hdl vendor backend is missing"`
unless a proprietary vendor header is present. Use `include/uvm_dpi_xezim.cc`
instead. It mirrors `uvm_dpi.cc`'s include chain and C linkage, skips
`uvm_hdl.c`, and implements the `uvm_hdl_*` surface itself per IEEE
1800.2-2017 Annex C (return 1 on success, 0 on failure). It uses only
`vpi_handle_by_name`, `vpi_get_value` and `vpi_put_value`: no vendor
extensions, no VHPI. The simulator-specific `uvm_is_vhdl_path` and
`uvm_register_*_vhdl` helpers are not part of IEEE 1800.2 and are not provided.

```bash
clang++ -shared -fPIC -std=c++17 -Wno-format-security \
    -I path/to/xezim/include -I path/to/uvm/src/dpi \
    path/to/xezim/include/uvm_dpi_xezim.cc \
    -o uvm.so
xezim ... --dpi-lib uvm.so
```

- Add `-DXEZIM_UVM_POLLING=1` for a 2020 kit, to include its
  `uvm_hdl_polling.c`; 1800.2-2017 doesn't ship that file.
- The driver also builds as C: `clang -x c -shared -fPIC ...` with the same
  flags otherwise.
- `-Wno-format-security` silences a warning from `uvm_hdl_polling.c`, which
  passes a non-literal format string to `sprintf`. The fix belongs upstream.

`scripts/build_uvm_dpi.sh` (a wrapper around `scripts/Makefile`) downloads the
1800.2-2017-1.0 and 2020.3.1 release kits and builds both libraries, as
`uvm-2017-1.0.so` and `uvm-2020.3.1.so` in the xezim directory. Set `CXX` or
`CXXFLAGS` to override the compiler or add flags.

---

## Linking against an external C/C++ library

Most non-trivial DPI shims depend on something — a CPU model, a cocotb plugin,
a regex engine, a compression codec. Pattern:

```bash
# 1) Compile your shim to an object file
clang++ -shared -fPIC -std=c++17 -fPIC -DXEZIM_SPIKE_REAL=1 \
    -I path/to/xezim/include -I $SPIKE_PREFIX/include \
    -c xezim_spike_dpi.cpp -o xezim_spike_dpi.o

# 2) Link the shim + the external library into one .so
clang++ -shared -fPIC \
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
clang -shared -fPIC -I path/to/xezim/include -I . my_dpi.c -o my_dpi.so
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
clang++ -shared -fPIC -I path/to/xezim/include -I dpi/include uvm_dpi.cc -o dpi/lib/libuvm_dpi.so
xezim --dpi-lib dpi/lib/libuvm_dpi.so -I dpi/include tb.sv
```

xezim's own `dpi/spike/` follows Convention A.

---

## Running with xezim

```bash
xezim --dpi-lib /abs/path/to/libfoo.so [more --dpi-lib paths …] <sv files>
```

* Repeatable: pass `--dpi-lib` once per shared library. Each is `dlopen`ed and
  its symbols added to the same dlsym table; when two libraries define the
  same symbol, the one given first wins.
* `RTLD_NOW | RTLD_GLOBAL` is used, so transitive deps must resolve at
  load time — set `LD_LIBRARY_PATH` if your `.so` has rpath-less deps, or
  pass `-Wl,-rpath,$PREFIX/lib` at link time. Because the symbols are global,
  one `--dpi-lib` library can call into another (a protocol library on top of
  a bridge library, say), in either command-line order.
* A library that cannot be loaded is an error: xezim prints why and exits 1
  without simulating. Calling an imported function that no loaded library
  defines is a `Fatal` that ends the run (exit 1); imports that are never
  called need no implementation.
* An exported task (`export "DPI-C" task t;`) called from C returns only when
  the task has finished: `#` delays, `wait(...)`, `@(...)`, `fork ... join`
  and `wait fork` inside it all run to completion first, with the rest of the
  simulation advancing meanwhile.
* Each call of an imported task (`import "DPI-C" task t;`) runs on its own
  stack, so several processes can be inside imported tasks at once and each
  resumes when its own wait is over (IEEE 1800 §35.5.2, §35.9). The C
  function returns `int`, and so does the C side of an exported task. The
  stacks are reserved, not committed, 256 MiB each by default;
  `XEZIM_DPI_STACK_MB` changes the size. This works on x86_64 and aarch64,
  on Linux (glibc or musl) and macOS. On other architectures, or with
  `XEZIM_DPI_FIBERS=0`, a wait inside an imported task runs the scheduler
  nested in the C call instead: concurrent calls then return in last-in,
  first-out order, and the §35.9 disable protocol is not available.
* Disabling a process while it waits inside an imported task (`disable` of a
  block around the call, `disable fork`, `process::kill()`) follows the
  §35.9 protocol. The waiting exported task returns 1, and
  `svIsDisabledState()` returns 1. The C code must call
  `svAckDisabledState()`, return 1 and call no more exports. If it does
  otherwise, the run ends with a `Fatal`. Outputs of a disabled call are not
  written back.
* An export declared in a module that is instantiated belongs to each
  instance. All instances share the one C symbol, and a call from C runs the
  copy in the current DPI scope: the instance that a `context` import was
  called from or through, or the scope set with `svSetScope`.
* The SV file must `import "DPI-C" function …` (or include a `.svh` that does)
  for every symbol you call from SV. Symbols that exist in the `.so` but
  aren't imported are simply ignored — there's no eager validation.
* `import "DPI-C"` must appear in module/program/interface scope (or inside a
  package), not at `$unit` scope. xezim today requires it inside a module —
  see `dpi/spike/test_spike_dpi.sv`.

---

## Types of exported subroutines

A C library calls an exported subroutine (`export "DPI-C" function f;`)
through a C function xezim generates. Its arguments must have the C types of
IEEE 1800 §35.5.6 and Annex H, the same prototype the reference simulator
writes into its generated DPI header:

| SV formal | C argument (input) | C argument (`output` / `inout`) |
|---|---|---|
| `byte`, `shortint`, `int`, `longint` | `char`, `short`, `int`, `long long` | pointer to it |
| `real`, `shortreal` | `double`, `float` | pointer to it |
| scalar `bit`, `logic` | `svBit`, `svLogic` | `svBit*`, `svLogic*` |
| `chandle` | `void*` | `void**` |
| `string` | `const char*` | `const char**` |
| packed vector (`bit`/`logic [N-1:0]`, packed struct), N > 64 | `const svBitVecVal*` / `const svLogicVecVal*` | `svBitVecVal*` / `svLogicVecVal*` |
| packed vector, N <= 64, `integer`, `time` | `int` (N <= 32) or `long long`, by value | pointer to it |
| unpacked struct | `const T*`, `T` the C struct of its members | `T*` |
| fixed-size unpacked array | `const E*`, its elements in a row | `E*` |

* In an unpacked struct or array, a packed vector of any width is an inline
  `svLogicVecVal[]` / `svBitVecVal[]` and a `string` is a `const char*`. Array
  elements run from the lowest index up in every dimension (`int a[5:2]`:
  `a[2]` is C element 0), the first dimension outermost.
* A packed vector of up to 64 bits passes by value as `int` or `long long`.
  The standard passes it by pointer at every width; xezim keeps the value
  form there, which earlier releases used.
* A function result (§35.5.5) is one of `void`, `char`, `short`, `int`,
  `long long`, `float`, `double`, `svBit`, `svLogic`, `void*` (`chandle`) or
  `const char*` (`string`). A returned string stays valid until the next call
  of the same export; so do strings stored through `const char**` outputs.
  A function returning a packed vector wider than 64 bits, or an unpacked
  aggregate, is not a legal DPI export: xezim stops with an error before the
  run, as the reference simulator does. Return it through an `output`.
* A formal of any other type (an `event`, a class handle, a dynamic array,
  a queue, an associative array, an unpacked union) cannot be passed. xezim
  prints at startup that the subroutine is NOT callable from C and why, and
  each call from C reports `[xezim][error] DPI export 'f' cannot be called
  from C: ...`, counted as an error, and returns 0.

---

## Cross-platform notes

| Platform | Shared-object ext. | Compiler | One-file recipe |
|---|---|---|---|
| Linux | `.so` | `clang` / `clang++` | `clang -shared -fPIC -I . foo.c -o foo.so` |
| macOS | `.dylib` | `clang` / `clang++` | `clang -shared -fPIC -I . foo.c -o foo.dylib` |
| Windows | `.dll` | `cl.exe` (MSVC) or `gcc` (MinGW) | `cl /LD /I . foo.c /Fe:foo.dll` |

xezim's `--dpi-lib` accepts the platform-correct extension automatically. On
macOS you may need `DYLD_LIBRARY_PATH` set instead of `LD_LIBRARY_PATH`. On
Windows, MSVC-produced DLLs need the corresponding `.lib` import library
available at link time of any consumer binary (xezim itself doesn't, because
it only `dlopen`s).

On macOS, a library that calls exported SV subroutines must leave those
symbols for the loader to resolve: add `-Wl,-undefined,dynamic_lookup` to the
link (`clang -shared -fPIC -Wl,-undefined,dynamic_lookup ...`). On Linux with
musl, build xezim with `RUSTFLAGS="-C target-feature=-crt-static"`: a static
musl executable cannot load shared libraries at all.

---

## Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `Error: --dpi-lib 'foo.so' could not be loaded: …cannot open shared object…` | Runtime can't find a transitive dep | Add `-Wl,-rpath,<dir>` at link time, or set `LD_LIBRARY_PATH` |
| `Error: --dpi-lib 'foo.so' could not be loaded: …undefined symbol: bar` | `foo.so` calls `bar`, which no loaded library defines | Pass the library that defines `bar` with another `--dpi-lib` (any order), or link `foo.so` against it |
| `** Fatal: DPI import 'my_dpi_fn' has no implementation` | Imported name doesn't match exported name (C++ mangling, missing `extern "C"`, missing `SV_PUBLIC`), or the library was not passed | Wrap in `extern "C"`, mark `SV_PUBLIC`, ensure the `.c`/`.cc` actually compiles the symbol in |
| `ImportError: …failed to run xezim: No such file or directory` from `cargo test` | The test harness uses `env!("CARGO_BIN_EXE_xezim")` — make sure the bin was built first | `cargo build --tests` then run; the env var is set at compile time |
| Symbol resolves but the call returns garbage | ABI mismatch (e.g. `int` vs `int64_t`, `char*` lifetime) | DPI imports must match the C signature exactly; for `string` returns, the buffer must outlive the call site |
| `[DPI] exported subroutine 'f' is NOT callable from C: formal 'x' has type ...` at startup, then `[xezim][error] DPI export 'f' cannot be called from C` | One formal of the export has a type DPI cannot pass (an `event`, a class handle, a dynamic array, ...); the call does not run the subroutine | Pass that data another way (a `chandle`, an index into an SV-side table); see "Types of exported subroutines" |
| An exported subroutine called from C reads wrong argument values | The C prototype does not match the export's (a packed vector wider than 64 bits, an unpacked struct or an array is passed by pointer, `byte` is `char`, `shortreal` is `float`) | Declare the C prototype as "Types of exported subroutines" lists |
| `failed to resolve path` from a VPI call | `vpi_handle_by_name` found no object of that name | Use the full name from the top (`top.u_sub.gen[1].sig`), `pkg::name` for a package member, or pass a scope handle for a relative name; see "VPI object model" |

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

All six are built with the same `clang -shared -fPIC -I <xezim_dir>` invocation —
no per-test build system, just six shell-out-to-`clang` calls inside the test
harness.

---

## Classic VPI modules (`--vpi-lib`)

Besides serving as a DPI callee, xezim can load a *classic* VPI application —
one that registers system tasks/functions and walks the design:

```bash
clang -shared -fPIC -I <xezim_dir> -o my_vpi.so my_vpi.c
xezim --vpi-lib my_vpi.so design.sv        # alias: -m my_vpi.so
```

Each library's `vlog_startup_routines` entries run before simulation.

**Supported surface:**

- `vpi_register_systf` — both system **tasks** and system **functions**
  (`vpiSysFunc` returns what it deposits via `vpi_put_value` on its own call
  handle; `vpiSizedFunc` gets its width from `sizetf`). A registered name never
  shadows an xezim builtin. It returns the registration's `vpiUserSystf`
  object, which `vpi_get_systf_info` reads back; `vpi_handle(vpiUserSystf,
  call)` and `vpi_iterate(vpiUserSystf, NULL)` reach the same objects.
- Per-call user data: `vpi_put_userdata` / `vpi_get_userdata` on a call handle
  attach data to that call **instance** — one call site in one module
  instance — so it persists across every later call from the same place.
- `vpiSysTfCall` / `vpiArgument` — a `$systf` reads its own arguments; a
  signal-backed argument is writable (so `output` args work), a literal is a
  read-only `vpiConstant`.
- Design walk: the whole elaborated design as the object model of IEEE
  1800-2017 chapter 37 — see "VPI object model" below — through
  `vpi_iterate`/`vpi_scan`, `vpi_handle`, `vpi_handle_by_name`,
  `vpi_handle_by_index`, `vpi_get`, `vpi_get_str`,
  `vpi_get_value`/`vpi_put_value`.
- `vpi_control`: `vpiStop`/`vpiFinish` end the run once the calling routine
  returns; `vpiSetInteractiveScope` takes a module handle; `vpiReset` is
  refused (xezim cannot rewind a run).
- `vpi_chk_error`, `vpi_printf`, `vpi_flush`, `vpi_compare_objects`,
  `vpi_get64`.
- Callbacks: `vpi_register_cb` for every reason of IEEE 1800-2017 §38.36
  (see below), `vpi_get_cb_info`, `vpi_remove_cb`.
- Multichannel descriptors: `vpi_mcd_open`, `vpi_mcd_close`, `vpi_mcd_flush`,
  `vpi_mcd_name`, `vpi_mcd_printf`/`vpi_mcd_vprintf`. They share the channel
  table of `$fopen`, so a descriptor opened in C can be written from
  SystemVerilog (`$fdisplay(mcd, ...)`) and the other way round.
- Time queries: `vpi_get(vpiTimeUnit, h)` / `vpi_get(vpiTimePrecision, h)` give
  a module's own timescale for a module handle and the simulation's for NULL,
  as powers of ten in seconds (`-9` = 1 ns). From DPI code, `svGetTime`,
  `svGetTimeUnit` and `svGetTimePrecision` answer the same for an `svScope`.
- Every value format of `vpi_get_value` / `vpi_put_value`, including
  `vpiStrengthVal`, `vpiTimeVal`, `vpiObjTypeVal`, `vpiShortIntVal`,
  `vpiLongIntVal`, `vpiShortRealVal`, `vpiRawTwoStateVal` and
  `vpiRawFourStateVal` (the header documents which union member each uses).
- Arrays of any dimension: `vpi_handle_by_name` gives a `vpiMemory`
  (one-dimensional) or `vpiRegArray` / `vpiNetArray` handle,
  `vpi_handle_by_multi_index` / `vpi_handle_by_index` select sub-arrays,
  elements, and part- and bit-selects through the packed dimensions, and
  `vpi_get_value_array` /
  `vpi_put_value_array` read and write whole sections in every array format,
  honouring `vpiUserAllocFlag`, `vpiOneValue` and `vpiPropagateOff`.
- Delays: `vpi_get_delays` / `vpi_put_delays` on nets (the delay of the
  gate or continuous assignment driving them), module paths
  (`vpi_iterate(vpiModPath, mod)`), timing checks (`vpi_iterate(vpiTchk,
  mod)`, their limits) and intermodule paths
  (`vpi_handle_multi(vpiInterModPath, outPort, inPort)`). A put changes the
  delays the simulation uses from then on.
- `vpi_get_data` / `vpi_put_data`: defined, and always fail (return 0 and
  report through `vpi_chk_error`) — see below.

**Callbacks** (`vpi_register_cb`, §38.36). The routine receives a fresh
`s_cb_data`: `obj` and `user_data` as registered, `time` the current time in
the registered `time->type` (`vpiSimTime` when `time` was NULL), and `value`
always a valid pointer (`vpiSuppressVal` when there is nothing to report).
The returned handle is a `vpiCallback` object; `vpi_free_object` releases the
handle but leaves the callback registered, `vpi_remove_cb` removes the
callback and frees the handle — from inside any callback, its own included.
`vpi_get_cb_info` returns the registration data. `$display` output written
before a callback runs appears before the routine's `vpi_printf` output.

| Reason | Fires | Notes |
|---|---|---|
| `cbNextSimTime` | first thing in the next time step | one-shot; `time` value ignored |
| `cbAtStartOfSimTime` | before any event of the given **absolute** time | one-shot |
| `cbAfterDelay` | with the first events of now + delay | one-shot |
| `cbNBASynch` | before the NBA region of now + delay | one-shot |
| `cbReadWriteSynch` | after the NBA region of now + delay, once no process of that step is left to run | one-shot |
| `cbAtEndOfSimTime` | after every other region of the given **absolute** time | one-shot |
| `cbReadOnlySynch` | last in the time step of now + delay; `vpi_put_value` is refused | one-shot |
| `cbValueChange` | after each change of a net, variable, part-select, port or memory (`index` = the word) | once per change, whichever path made it |
| `cbForce` / `cbAssign` | after `force` / `assign`, or `vpi_put_value(vpiForceFlag)` | obj = the object, or NULL for all |
| `cbRelease` / `cbDeassign` | once the released object has been re-driven | `value` = the value after release |
| `cbDisable` | after a `disable` terminates the named block, fork or task around a `$systf` call | obj = that call's `vpiSysTfCall` handle |
| `cbStmt` | before each interpreted statement a process of the given module instance runs | obj = a module handle |
| `cbEndOfCompile`, `cbStartOfSimulation` | before time 0, in that order | |
| `cbEndOfSimulation` | after the last time step and all `final` blocks | |
| `cbError` | after each run-time error: `$error`, `$fatal`, a reported timing violation, an illegal bin | `vpi_chk_error` reports it inside the routine |
| `cbPLIError` | after each VPI routine error | `vpi_chk_error` reports it inside the routine, and still to the caller after |
| `cbTchkViolation` | on each timing-check violation | obj NULL; `value` = the violation text (`vpiStringVal`) |
| `cbSignal` | when SIGINT/SIGTERM stops the run | `index` = the signal number |
| `cbUnresolvedSystf` | the first time an unknown `$name` is called | `value->value.str` = the name; registering it with `vpi_register_systf` in the routine makes the call go to it |
| `cbEnterInteractive` | when `$stop` or `vpi_control(vpiStop)` ends the run, before `cbEndOfSimulation` | xezim has no interactive mode: `$stop` ends the run like `$finish` |
| `cbInteractiveScopeChange` | on `vpi_control(vpiSetInteractiveScope, scope)` | obj = the scope |
| `cbExitInteractive`, `cbStartOfSave`, `cbEndOfSave`, `cbStartOfRestart`, `cbEndOfRestart`, `cbStartOfReset`, `cbEndOfReset` | never | accepted; xezim has no interactive mode, save/restart or `$reset` |

Within one time step the simulation-time reasons run in the order of the
table. A value written from any of them except `cbReadOnlySynch` opens a
fresh delta in the same step, so edge-sensitive processes see it. Concurrent
assertions are evaluated in the step of their clock edge, also when a routine
writes the clock. A `cbReadWriteSynch` due at an assertion's clock edge runs
before them and before that delta's clocking-event waiters, instead of after
both, once the step's Active, Inactive and NBA work and its `cbNBASynch`
routines are done (the Post-NBA region comes before the Observed region,
§4.4.2): a reset it releases at the edge, as a cocotb test does after
`RisingEdge`, does not disable that edge's attempt (`disable iff` reads
current values), and one a clocking-event waiter raises at the edge comes
after the attempt. They sample the values the step began with (§16.5.1),
from before even `cbNextSimTime`, `cbAtStartOfSimTime` and `cbAfterDelay`
run, so a clock written from `cbReadWriteSynch`, as cocotb's Python `Clock`
does, samples like one driven from HDL. A pending callback holds the
scheduler at its time even when no HDL event is due there.
The time of `cbAfterDelay`, `cbNBASynch`, `cbReadWriteSynch` and
`cbReadOnlySynch` is a delay (a NULL `time` is a zero delay); that of
`cbAtStartOfSimTime`/`cbAtEndOfSimTime` is absolute, and a time already begun
is refused. `vpiScaledRealTime` is in the time unit of `obj` when that is a
module, else in simulation ticks.

A value written from `cbStartOfSimulation` is evaluated in the time-0 step,
even when nothing else happens at time 0. Waveform dumps and toggle coverage
record each write in the step it was made in, the end-of-step callbacks'
included.

xezim has no statement objects, so `cbStmt` takes a scope and reports that
scope as `obj`, and `cbForce`/`cbRelease`/`cbAssign`/`cbDeassign` registered
with a NULL `obj` report the affected net or variable (a handle valid only
during the routine) where the standard names the statement. `cbStmt` covers
statements the interpreter runs; statements of processes compiled to
bytecode or native code (most `always` blocks) are not reported. A run with
no callback registered pays nothing for any of this beyond one flag test per
hook.

**Semantics notes:**

- `vpi_iterate(vpiInternalScope, mod)` yields the module's child **scopes**
  (instances, generate scopes, named blocks, tasks and functions), per the
  standard — *not* its declared nets/variables (a common misuse in the
  wild). Declared objects come from `vpiNet`/`vpiReg`/`vpiVariables`/
  `vpiParameter`/`vpiMemory` and the other relations below.
- `vpi_handle(vpiScope, NULL)` returning the top module is an xezim extension
  (the standard route is `vpi_scan(vpi_iterate(vpiModule, NULL))`).
- `compiletf` runs once per call instance, just before that instance's first
  `calltf` (xezim has no separate compile phase to run it in), so the usual
  "allocate per-instance state in compiletf, keep it with
  `vpi_put_userdata`" pattern works.
- `vpiStrengthVal`: a variable always reads strong (§38.15); a net reads the
  drive strength of the continuous assignment or gate driving it — the same
  record `%v` displays — and strong when none was declared; `z` reads
  `vpiHiZ`. xezim keeps no strength per value, so `vpi_put_value` with
  `vpiStrengthVal` (legal only on a scalar) writes the logic value and checks,
  but does not store, the strengths.
- `vpiObjTypeVal` picks `vpiIntVal` for an integer-typed object of up to 32
  bits, `vpiRealVal` for a real, `vpiTimeVal` for a time variable,
  `vpiStringVal` for a string, `vpiScalarVal` for any other 1-bit object and
  `vpiVectorVal` for any other vector (§38.15). A real object read in an
  integer or string-of-digits format is rounded to an integer first; its
  `vpiStringVal` is its decimal text. Octal and hex digits read `x`/`z` when
  every bit of the digit is x/z, `X`/`Z` when only some are. `vpiIntVal`
  sign-extends a signed object narrower than 32 bits.
- `vpiShortIntVal`, `vpiLongIntVal`, `vpiShortRealVal` and the two raw formats
  have no member of their own in `s_vpi_value`; xezim carries them in
  `integer`, `misc` (-> `PLI_INT64`), `real` and `misc` (-> the raw bytes of
  one array element) respectively, using the implementation-specific `misc`
  field the standard provides for this.
- Delays: xezim lowers every gate and continuous assignment onto the net it
  drives and keeps no primitive or continuous-assignment objects, so their
  delays are read — and, as an extension, written — through the driven net's
  handle (1–3 delays: rise, fall, turn-off). A driver elaborated without a
  delay is compiled into a form that cannot take one unless it is a plain
  copy, a one-bit gate or an interpreted assignment; putting delays on such a
  net is refused (reported through `vpi_chk_error`, nothing written), as is
  a zero rise delay with a non-zero fall or turn-off delay. xezim keeps one
  value per delay and has no separate pulse limits: `mtm_flag` reads the
  value in all three slots (a put takes the active min:typ:max selection's),
  and `pulsere_flag` reads the delay as the reject and error limits (a put
  whose limits differ from its delay is refused). `append_flag` adds to the
  current delays. A module path's `vpiFullName` is its output net's; a timing
  check's is its scope plus the check's name.
- An input or inout port connected to a whole net of its own shape is one
  object with it (§23.3.3.7), an unpacked-array port connected to a whole
  array included: a value written through VPI to `top.u.a[3]` is what
  instance `u` reads, and the actual's name `top.a[3]` reads it too, through
  any number of port levels; one written to `top.a[3]` reaches `top.u.a[3]`.
  A port on a bit- or part-select, an array slice, a concatenation or an
  expression keeps a net of its own. An unpacked-array connection with a
  variable formal or actual also keeps its propagation step (§23.3.3.3).
  Scalar identity ports can still share storage with a variable actual;
  a VPI write through such a scalar port then changes the variable too.
- `vpiInterModPath` delays live where SDF `INTERCONNECT` delays do, on the
  input port's net. When elaboration collapsed the two ports into one net
  there is no interconnect between them, and `vpi_handle_multi` returns NULL.
  Only the first two reference handles of `vpi_handle_multi` are read: the
  standard defines no other relation for it, nor a terminator for a longer
  list.
- `vpi_get_data` / `vpi_put_data` may only be called while a restart / save
  is in progress (from `cbStartOfRestart`/`cbEndOfRestart` /
  `cbStartOfSave`/`cbEndOfSave`). xezim has no `$save`/`$restart`, so neither
  is ever in progress: both return 0 and report the error through
  `vpi_chk_error`.

**Not implemented:** every routine of IEEE 1800-2017 clause 38 is declared in
`include/vpi_user.h` and implemented as described above. The one gap is the
SystemVerilog thread, frame and class-object callback reasons of
`sv_vpi_user.h` (`cbStartOfThread` … `cbEndOfObject`): xezim has no thread,
frame or class-object VPI objects, and `vpi_register_cb` rejects them.

Worked examples: `tests/dpi/vpi_object_model.{c,sv}`,
`tests/dpi/vpi_systf.{c,sv}`, `tests/strings/vpi_routines.rs` (user data,
systf info, arrays, delays and every value format), and the design walks in
`tests/strings/vpi_object_model_walk.rs`.

### VPI object model

xezim flattens the design at elaboration, so the object model is rebuilt the
first time a VPI routine needs it: the preprocessed text of every source file
(and of every `-v`/`-y` library file a definition was taken from) is parsed
again, and each instance of the elaborated instance tree walks its
definition — evaluating generate constructs with that instance's parameter
values, as the elaborator did — recording its scopes and objects and the
signal each declared object was elaborated to. Nothing is built or kept when
no VPI or DPI code calls in. A design loaded from a compiled artifact (which
carries no sources) has no object model; VPI then sees the instance tree and
the signal table only, as before.

**Scopes.** `vpi_iterate(vpiModule | vpiInstance | vpiInterface |
vpiProgram | vpiPackage, NULL)` gives the design's roots: the top modules,
programs and interfaces (several when there is more than one top), and for
`vpiInstance` the packages after them. Below a scope, `vpiModule`,
`vpiInterface`, `vpiProgram` and `vpiInstance` give its instances,
`vpiGenScopeArray` its generate loops (whose `vpiGenScope` elements are
`name[i]`), `vpiGenScope` its generate scopes, `vpiTaskFunc` its tasks and
functions, and `vpiInternalScope` all of these plus named `begin`/`fork`
blocks, in declaration order. An unnamed generate block is `genblk<n>`
(§27.6) with `vpiImplicitDecl` 1. Objects inside a generate scope belong to
that scope, not to the module around it.

| Scope | `vpiType` | Properties |
|-------|-----------|------------|
| module / interface / program instance | `vpiModule`, `vpiInterface`, `vpiProgram` | `vpiDefName`, `vpiDefFile`, `vpiDefLineNo`, `vpiTopModule` (modules), `vpiTop`, `vpiCellInstance` (inside `` `celldefine``, or taken from a `-v`/`-y` library), `vpiTimeUnit`, `vpiTimePrecision` |
| package | `vpiPackage` | full name `pkg::`; `vpiDefName`, `vpiDefFile`, `vpiDefLineNo`, `vpiTop` 1, `vpiUnit` 0, `vpiAutomatic` |
| generate scope | `vpiGenScope` | `vpiArrayMember`, `vpiImplicitDecl`; `vpiIndex` (a constant) and `vpiParent` (the array) for a loop element |
| generate scope array | `vpiGenScopeArray` | `vpiSize` (elements); `vpi_handle_by_index` selects one |
| named block | `vpiNamedBegin`, `vpiNamedFork` | `vpiJoinType` (fork), `vpiAutomatic` |
| modport (in an interface; `vpi_iterate(vpiModport, ifc)`) | `vpiModport` | `vpiIODecl` iterates its ports: `vpiDirection`, and `vpiExpr`, the interface object (or modport expression) the port names |
| task / function | `vpiTask`, `vpiFunction` | `vpiAutomatic`, `vpiVisibility` (`vpiPublicVis`), `vpiFuncType`, `vpiSize`, `vpiSigned` and `vpiTypespec` of the result; a DPI import has `vpiAccessType` `vpiDPIImportAcc`, `vpiDPIContext`, `vpiDPIPure`; `vpiIODecl` iterates the arguments (`vpiDirection`, `vpiSize`, ranges, `vpiTypespec`) |

**Declared objects**, iterated from any scope: `vpiNet`, `vpiNetArray`,
`vpiReg`, `vpiRegArray` (= `vpiArrayVar`), `vpiMemory` (one-dimensional
arrays of `logic`/`reg`, the 1364 memories), `vpiVariables` (every
variable), `vpiParameter`, `vpiNamedEvent`, `vpiNamedEventArray`, or any
single variable type (`vpiIntVar`, `vpiStructVar`, ...). The type is the
declared one, through typedefs:

| Declaration | `vpiType` |
|-------------|-----------|
| `logic`/`reg` variable | `vpiReg` (= `vpiLogicVar`) |
| `bit`, `integer`, `int`, `byte`, `shortint`, `longint`, `time`, `real`/`realtime`, `shortreal`, `string`, `chandle` | `vpiBitVar`, `vpiIntegerVar`, `vpiIntVar`, `vpiByteVar`, `vpiShortIntVar`, `vpiLongIntVar`, `vpiTimeVar`, `vpiRealVar`, `vpiShortRealVar`, `vpiStringVar`, `vpiChandleVar` |
| enum / struct / union variable | `vpiEnumVar`, `vpiStructVar`, `vpiUnionVar` (a packed array of them: `vpiPackedArrayVar`); `vpiMember` iterates the members |
| unpacked array (fixed, dynamic, queue, associative) | `vpiRegArray` (`vpiArrayVar`); `vpiArrayType`, `vpiIsMemory`; `vpi_handle_by_index` gives an element (a sub-array of a multi-dimensional array), and `vpiReg`/`vpiNet` iteration the elements of the outermost dimension |
| net | `vpiNet` with `vpiNetType` (`vpiWire`, `vpiWand`, `vpiTri1`, ...); `vpiEnumNet`, `vpiStructNet`, `vpiIntegerNet`, `vpiTimeNet`, `vpiPackedArrayNet` for nets of those types; `vpiNetArray`; an implicit net has `vpiImplicitDecl` 1 |
| parameter | `vpiParameter` with `vpiLocalParam` and `vpiConstType` |
| `event` | `vpiNamedEvent` (named; it has no value) |

Every net and variable answers `vpiSize` (bits; elements for an array;
characters for a string), `vpiSigned`, `vpiScalar`/`vpiVector`, `vpiArray`,
`vpiLineNo`, `vpiAutomatic`, `vpiVisibility`, `vpiConstantVariable`,
`vpiDirection` (its port's, for a port's net or variable), and — like a
parameter — the relations
`vpiLeftRange`/`vpiRightRange` (constants: the outermost packed range, or
the outermost unpacked range of an array — for a dynamic array or queue its
current bounds `[0:size-1]`, for an associative array none), `vpiRange`
(every packed range, or every unpacked range of an array), and
`vpiTypespec`. A typespec
(`vpiLogicTypespec`, `vpiIntTypespec`, `vpiEnumTypespec`, `vpiStructTypespec`,
`vpiArrayTypespec`, ...) has `vpiName` (the typedef, if any), `vpiSize`,
`vpiSigned`, `vpiPacked`, `vpiRange`; `vpiElemTypespec` of an array
typespec, `vpiBaseTypespec` and `vpiEnumConst` (name and value) of an enum,
and `vpiTypespecMember` of a struct or union. A vector's bits come from
`vpi_iterate(vpiBit)` (also `vpiNetBit`/`vpiRegBit`) or by name
(`top.w[3]`); each is a `vpiNetBit`/`vpiRegBit` with `vpiParent` and
`vpiIndex`, readable and writable.

**Processes and assignments.** `vpiProcess` iterates `vpiInitial`,
`vpiAlways` (with `vpiAlwaysType`: `vpiAlways`, `vpiAlwaysComb`,
`vpiAlwaysFF`, `vpiAlwaysLatch`) and `vpiFinal`. `vpiContAssign` iterates
continuous assignments (`vpiNetDeclAssign` for a net declaration
assignment) with `vpiLhs`, `vpiRhs` and `vpiDelay`. An expression handle is
the object it names; a literal, and a genvar inside its loop (the element's
index), is a `vpiConstant` with `vpiConstType`; a select is a `vpiBitSelect`/`vpiPartSelect` (with `vpiParent`, `vpiIndex`,
`vpiLeftRange`/`vpiRightRange`); anything else is a `vpiOperation` with
`vpiOpType` and `vpiOperand`. Expressions are readable with
`vpi_get_value`.

**Primitives.** `vpiPrimitive` iterates `vpiGate`, `vpiSwitch` and `vpiUdp`
instances (also iterable by those types): `vpiDefName` (`and`, or the UDP
name), `vpiPrimType` (`vpiAndPrim`, ..., `vpiSeqPrim`/`vpiCombPrim` for a
UDP), `vpiSize` (inputs), `vpiDelay`, and `vpiPrimTerm` terminals with
`vpiTermIndex`, `vpiDirection`, `vpiExpr` and a value.

**Specify.** `vpiModPath` iterates path declarations (`vpiModPathIn` /
`vpiModPathOut` path terms with `vpiExpr` and `vpiDirection`, `vpiCondition`,
`vpiModPathHasIfNone`); `vpiTchk` iterates timing checks (`vpiName` is the
task name, `vpiTchkType`, `vpiTchkRefTerm` and `vpiTchkDataTerm` terms with
`vpiEdge`, `vpiExpr` and `vpiCondition`, and `vpiTchkNotifier`).

**Ports.** `vpiPort` iterates in port-list order with `vpiPortIndex`,
`vpiDirection`, `vpiSize`, `vpiConnByName`, `vpiPortType` (`vpiPort`, or
`vpiInterfacePort`/`vpiModportPort`), `vpiLowConn` (the net or variable
inside) and `vpiHighConn` (the connected expression in the parent; NULL for
a top-level or unconnected port). `vpiBit` iterates a vector port's
`vpiPortBit`s. From a net or variable, `vpiPortInst` gives the child-instance
ports it is connected to, and `vpiPorts` the ports it is the low connection
of. §23.2.2.3 decides a port's kind: an `input`/`inout` declared without a
net type or `var` is a net (so `input logic clk` is a `vpiNet`), an
`output` with an explicit data type is a variable.

**Relations upward.** `vpiScope` is the scope an object is declared in (for
an instance, the scope that instantiates it: a generate scope, say);
`vpiModule` the enclosing module instance (NULL inside an interface,
program or package); `vpiInstance` the enclosing instance of any kind,
package included; `vpiParent` the object an object belongs to — the array of
a generate scope element, the struct of a member, the variable of a bit or
array element, the primitive of a terminal — and otherwise the scope.

**Names.** `vpiName`, `vpiFullName`, `vpiType` (the type's name), `vpiFile`
and `vpiLineNo` work on every object, and `vpiDefName`/`vpiDefFile` on
instances, packages and primitives; a string that does not exist (an unnamed
process's name) is NULL. Full names are hierarchical from the top
(`top.gl[1].u_l.d`); package members are `pkg::name` and the package itself
`pkg::`. `vpi_handle_by_name` finds every named object by its full name, a
bit or element (`top.w[3]`, `top.mem[1]`), and, with a scope handle, a name
relative to that scope. An instance's `vpiFile`/`vpiLineNo` are where it is
instantiated (the `bind` directive for an instance a `bind` adds; the
definition for a top-level instance); every other object's are where it is
declared. A location xezim cannot place — an object
elaboration made up (an implicit net), or a library line changed by an
`` `include`` — is `vpiLineNo` 0 and `vpiFile` NULL.

**Values.** Nets, variables, parameters, members, bits, array elements,
ports, primitive terminals and expressions have values. A variable declared
in a named block, task or function is an object with a type, but its storage
is private to the running process: `vpi_get_value` on it sets `vpiSuppressVal`
and reports a `vpi_chk_error` error, as it does for a named event, a scope, a
process and the other objects without a value. `vpi_put_value` rejects all of
them the same way.

**Not modelled** in the object model: statements (a process's `vpiStmt` is
only its named block, when it is one), classes and their objects, clocking
blocks, concurrent assertions, `let` and `checker` declarations,
typedef and import objects (`vpiTypedef`, `vpiImport`), `vpiDriver` and
`vpiLoad`, attributes, `vpiParamAssign`/`vpiDefParam`, `vpiGenVar`,
`vpiSpecParam`, array objects for instance and primitive arrays
(`vpiModuleArray`, `vpiGateArray`, ...; their elements are there), the
`$unit` package, `vpiDecompile`, and the arguments of a task or function
declared in its body (`function f; input int x; ...`) rather than in its
header.


---

## See also

* [`../dpi/spike/README.md`](../dpi/spike/README.md) — worked example with a real
  external library (Spike / riscv-isa-sim), including stub-mode and real-mode
  builds.
* [`uvm-guide.md`](uvm-guide.md) — running UVM testbenches on xezim. With
  `-DUVM_NO_DPI` the UVM library makes no DPI calls; without it, xezim serves
  UVM's DPI-C helpers natively. Your own DPI extensions work either way.
* [`../tests/dpi/`](../tests/dpi/) — the canonical one-`.c`-per-test pairs.
