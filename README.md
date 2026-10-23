<h1 align="center">xezim</h1>

<p align="center">
  <b>An open-source SystemVerilog simulator, written in Rust.</b><br>
  RTL, gate-level and UVM simulation in one command, from source to results.
</p>

<p align="center">
  <a href="https://github.com/aionhw/xezim/actions/workflows/test.yml"><img alt="CI" src="https://github.com/aionhw/xezim/actions/workflows/test.yml/badge.svg"></a>
  <a href="https://github.com/aionhw/xezim/tags"><img alt="Release" src="https://img.shields.io/github/v/tag/aionhw/xezim?label=release&amp;color=informational"></a>
  <a href="https://github.com/chipsalliance/sv-tests"><img alt="sv-tests: 99.0%" src="https://img.shields.io/badge/sv--tests-99.0%25-brightgreen"></a>
  <a href="docs/building.md#requirements"><img alt="Rust 1.92+" src="https://img.shields.io/badge/rust-1.92%2B-orange"></a>
  <a href="LICENSE"><img alt="License: Apache 2.0" src="https://img.shields.io/badge/license-Apache%202.0-blue"></a>
</p>

<p align="center">
  <a href="docs/user-guide.md">User guide</a> ·
  <a href="docs/command-line.md">Command line</a> ·
  <a href="docs/uvm-guide.md">UVM</a> ·
  <a href="docs/dpi-guide.md">DPI &amp; VPI</a> ·
  <a href="NOTES.md">Release notes</a> ·
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

---

## Welcome to xezim

<table>
<tr>
<td width="50%" valign="top">

**From source to results in one command**
- Verilog and SystemVerilog, IEEE 1800-2023 (1800-2017 on request)
- Preprocess, elaborate and simulate in one step: no library, compile or
  elaboration stages to manage, and no license server
- Accepts the compile and run options of existing flows: args files,
  `+define+`/`+incdir+`, `-do` scripts, parameter overrides

</td>
<td width="50%" valign="top">

**RTL to gate level**
- Event-driven simulation with the full IEEE 1800 scheduling regions
- UDPs, drive strengths, specify-block delays and timing checks
- SDF back-annotation (`--sdf`)
- IEEE 1801 UPF power intent (`--upf`)
- SystemC (IEEE 1666) RTL, testbenches and TLM-2.0 LT from `.cpp` sources

</td>
</tr>
<tr>
<td valign="top">

**Verification**
- UVM 1.2, IEEE 1800.2-2017 and 1800.2-2020, with UVM's DPI-C helpers
  built in
- Classes, constrained randomization, covergroups and SVA
- Functional, assertion and code coverage

</td>
<td valign="top">

**Debug and integration**
- VCD, FST and XTrace waveforms
- DPI-C, the IEEE 1800 VPI (routines, callbacks and the design object
  model), and [cocotb](contrib/cocotb) Python testbenches
- Diagnostics that name the file, line and macro a problem came from

</td>
</tr>
<tr>
<td valign="top">

**Fast to iterate**
- Elaborates large RISC-V SoCs
- Warm design cache for repeated runs
- Profile-guided builds, optional JIT and ahead-of-time native compilation

</td>
<td valign="top">

**Open**
- Apache 2.0 licensed, developed in the open
- More than 5,400 regression tests, many checked against a commercial
  reference simulator
- Built with AI assistance as part of the engineering workflow

</td>
</tr>
</table>

The [feature list](docs/features.md) has the details.

## What xezim does

xezim reads Verilog and SystemVerilog source files and simulates them. It
preprocesses and parses the sources, elaborates the design hierarchy, compiles
processes and continuous assignments into bytecode, and runs them on an
event-driven kernel that follows the IEEE 1800 scheduling semantics. Most
logic runs on fast two-state executors, which fall back to full four-state
evaluation wherever `x` and `z` values appear.

Everything happens in one invocation, so a design goes from source to results
without a separate build step, and a test suite made of many short runs gets
its answers quickly. The same command line works for an RTL unit test, a
gate-level netlist with SDF timing, or a full UVM environment.

## Quick start

```bash
git clone https://github.com/aionhw/xezim.git
cd xezim
cargo build --release          # or ./scripts/build-pgo.sh for a profile-guided build
```

```systemverilog
// counter.sv
module counter_tb;
  logic clk = 0, rst = 1;
  logic [3:0] count;
  always #5 clk = ~clk;
  always_ff @(posedge clk) count <= rst ? '0 : count + 1;
  initial begin
    #12 rst = 0;
    repeat (3) @(posedge clk) $display("t=%0t count=%0d", $time, count);
    $finish;
  end
endmodule
```

```text
$ ./target/release/xezim counter.sv
t=15 count=0
t=25 count=1
t=35 count=2
Simulation finished at time 35 ($finish called)
```

A UVM testbench needs only the UVM sources on the command line:

```bash
xezim -s top -I $UVM/src $UVM/src/uvm_pkg.sv <design and testbench files> \
      +UVM_TESTNAME=my_test
```

See the [user guide](docs/user-guide.md) and the
[UVM guide](docs/uvm-guide.md) for more.

## Performance

Whole-run wall-clock time on one machine (Intel Core i7-9800X, six logical
CPUs, Linux). The xezim 0.11.3 figures were measured with a plain release
build and cold caches. Reference figures are from earlier verified runs on
the same machine in optimized mode, without debug visibility. Both
simulators produce the same results on every row.

| Workload | xezim | Reference simulator |
|---|---|---|
| XuanTie C906 SoC, CoreMark ×1 (295,294 cycles) | 82 s, 0.62 GiB peak | 82 s, 44 s of it simulating |
| XuanTie C910 dual-core SoC, memcpy ×200, cold start | 121 s, about 12 s of it compiling | 136 s, 97 s of it simulating |
| AXI4 AVIP, UVM base test | 7.8 s | 97 s, 51 s of it simulating |

AXI4 is the median of five runs; each SoC figure is one run. Host load affects
elapsed time, so these figures describe the measured workloads rather than a
controlled comparison. Compilation and elaboration account for only a small
part of these xezim runs, making startup cost useful to consider separately
from simulation time.

To get the most out of a build, measure a
[profile-guided build](docs/building.md#profile-guided-build) trained on your
own workload and keep the
[warm design cache](docs/user-guide.md#warm-design-cache) on.
[Native compilation](docs/user-guide.md#native-compilation) pays on designs
with few, very hot blocks, so measure it on yours.

## Conformance

- **sv-tests:** xezim 0.11.0 passes 4,722 of 4,770 tests (99.0%) of the
  [sv-tests](https://github.com/chipsalliance/sv-tests) suite.
- **UVM:** the mbits-mirafra AVIP base tests for AXI4, APB, I3C, SPI and
  AXI4-Lite print the same UVM messages as a commercial reference simulator,
  and 32 of 35 UVM 1800.2-2017 example testbenches pass.
- **Regression suite:** 5,412 default tests and two JIT smoke tests run in CI. Many are differential tests whose expected values were measured on a
  commercial reference simulator; see [CONTRIBUTING.md](CONTRIBUTING.md#test-suites).

## Documentation

| Guide | Contents |
|---|---|
| [Supported features](docs/features.md) | Language, UVM, coverage, debug and integration features in detail |
| [User guide](docs/user-guide.md) | Running simulations, waveforms, design cache, native compilation, timescales |
| [Command-line reference](docs/command-line.md) | Every option and environment variable, and the spellings of other simulators that xezim accepts |
| [UVM guide](docs/uvm-guide.md) | Running UVM testbenches, supported features, UVM's DPI-C library |
| [DPI and VPI guide](docs/dpi-guide.md) | Loading C and C++ libraries, the VPI surface |
| [Coverage guide](docs/coverage-guide.md) | Functional, assertion and code coverage, `xezim_cov.json` |
| [UPF guide](docs/upf-guide.md) | IEEE 1801 power intent |
| [SystemC guide](docs/systemc-guide.md) | IEEE 1666 SystemC and TLM-2.0 designs |
| [Debugging guide](docs/DEBUGGING.md) | Tracing, diagnosing hangs, comparing against another simulator |
| [Building from source](docs/building.md) | Requirements, profile-guided builds, working on xezim-core |
| [Release notes](NOTES.md) | Changes per release, verified workloads, compliance results |

## Support

Questions, bug reports and feature requests go to
[GitHub issues](https://github.com/aionhw/xezim/issues). A small
self-checking testcase gets a bug fixed fastest; see
[CONTRIBUTING.md](CONTRIBUTING.md) for what to include. Pull requests are
welcome.

## Background

xezim explores whether modern tools and AI assistance can sharply reduce the
cost of building core EDA infrastructure such as a simulator: work that has
traditionally taken large teams many years. It was built incrementally, from
simple combinational logic up to SoCs and UVM environments, with every step
verified against the language standard and against a commercial reference
simulator. Longer term, the project looks at cloud-scale and distributed
multi-CPU simulation.

The project was previously developed under the name `sisSIM`.

## Contributors

Thank you to everyone who contributes code, tests and bug reports to xezim
and xezim-core.

See [CONTRIBUTORS.md](CONTRIBUTORS.md) for what each person contributed, and
[CONTRIBUTING.md](CONTRIBUTING.md) to get involved.

## Acknowledgements

- The Icarus Verilog project, for its public test suite
- The [sv-tests](https://github.com/chipsalliance/sv-tests) project
- The Rust community and the open-source EDA projects xezim builds on

## License

xezim is free software, licensed under the [Apache License 2.0](LICENSE).
