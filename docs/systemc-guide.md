# SystemC (IEEE 1666)

xezim simulates SystemC designs: RTL modules, `sc_main` testbenches and
TLM-2.0 loosely-timed models. Give it the `.cpp` files like any other source:

    xezim top.cpp
    xezim -I include design.cpp tb.cpp      # several translation units

Files ending in `.cpp`, `.cc`, `.cxx`, `.h`, `.hpp` or `.hh` are SystemC; `-I`
and `-D` apply to their C++ preprocessor too. `#include <systemc>` and the
other SystemC/TLM library headers are not read (xezim knows the library);
the design's own headers are. Without `-s`, the top is `sc_main`.

## Mixing with SystemVerilog

SystemC and SystemVerilog sources can be given together; the SystemC
modules become SystemVerilog modules, so instantiation works both ways.

- **SystemVerilog instantiating SystemC**: instantiate the SystemC module by
  its class name, with its ports by name (`counter u(.clk(clk), .q(q));`).
  With no `sc_main` in the design, the SystemVerilog top is found as usual
  and the SystemC processes start at time 0.
- **SystemC instantiating SystemVerilog**: declare the SystemVerilog module
  as a class deriving from `sc_foreign_module`, with its ports as members:

      class shifter : public sc_foreign_module {
      public:
          sc_in<bool> clk;
          sc_out<sc_uint<8> > q;
          shifter(sc_module_name nm) : sc_foreign_module(nm, "shifter") {}
      };

  The SystemVerilog module name is the second argument of
  `sc_foreign_module(...)`, or what `const char* hdl_name() const` returns,
  else the class name. Instances bind ports as for any SystemC module.
  Parameters of the SystemVerilog module (generics) cannot be set this way.

Port types must have the same width on both sides (`sc_uint<8>` ↔
`logic [7:0]`, `int` ↔ `int`, `bool` ↔ `bit`/`logic`).

## How it works

xezim-core parses the C++ (`sv_parser::systemc`) and lowers each module to a
SystemVerilog module built directly as an AST, which then elaborates and
simulates like any SystemVerilog design.
`XEZIM_SYSTEMC_DUMP=1` prints the generated modules (as AST).

The SystemC scheduler maps onto the SystemVerilog regions so that outputs
match the SystemC 2.3 reference kernel:

| SystemC | In xezim |
|---|---|
| Processes start at the first `sc_start` | each process waits for `sc_start` |
| `SC_METHOD` (+ `dont_initialize`) | runs at initialization, then on its static sensitivity |
| `SC_THREAD`, `wait()` / `wait(t)` / `wait(e)` / `wait(e1 \| e2)` / `wait(t, e)` | an `initial` process with event controls and delays |
| `SC_CTHREAD` + `reset_signal_is` / `async_reset_signal_is`, `wait(n)` | first runs at its first clock edge; restarts on reset |
| `sc_signal` / port writes | update phase (nonblocking); `sc_main` writes apply at the next `sc_start` |
| `sc_start(t)` | stops before the events *at* `t` (they run in the next `sc_start`) |
| `sc_start()` | returns at `sc_stop()` or when no events are left; the rest of `sc_main` then runs (in a `final` block, so it cannot call `sc_start` or `wait` again) |
| `sc_stop()` | stops at the end of the delta cycle, with SystemC's message |
| `sc_clock` | period, duty cycle, start time, `posedge_first` |
| time | picoseconds (SystemC's default resolution); `sc_time` prints like SystemC (`1500 ps`, `1 us`) |

## Supported

- **Modules**: `SC_MODULE` and classes deriving from `sc_module` (with
  `SC_HAS_PROCESS` and out-of-class constructors), template modules with
  integer parameters, constructor parameters, child modules (members, `new`,
  initializer lists), named and positional port binding, `sc_vector` ports
  and signals.
- **Ports and channels**: `sc_in`/`sc_out`/`sc_inout` (an `sc_inout` is an
  output its owner also reads), `sc_signal`, `sc_buffer` (an event on every
  write), `sc_clock`, `sc_fifo` (with its depth) with
  `sc_fifo_in`/`sc_fifo_out`, `sc_event`.
- **Data types**: `bool`, `sc_uint`/`sc_int`/`sc_biguint`/`sc_bigint` (up to
  the width SystemVerilog vectors allow), `sc_bv`, `sc_lv`, `sc_logic` (and
  bit-string literals such as `"01XZ"`), the C++
  integer and floating types, `std::string`, `std::vector`, `sc_time`; part and
  bit selects, `(a, b)` concatenation, reductions, `to_uint()` and friends.
- **Output**: `std::cout` (with `hex`/`dec`/`setw`/`setfill`, printing
  SystemC types the way SystemC does), `printf`, `SC_REPORT_INFO/WARNING/
  ERROR/FATAL` in SystemC's format, `sc_trace` to VCD.
- **TLM-2.0 (loosely timed)**: `tlm_generic_payload`, `simple_initiator_socket`
  / `simple_target_socket` and `tlm_initiator_socket` / `tlm_target_socket`,
  `b_transport`, `transport_dbg`, `register_b_transport`, `socket.bind(*this)`.
  C data pointers (`set_data_ptr(&x)`, `memcpy`, `memset`, pointer indexing and
  `reinterpret_cast` between byte and word pointers) work through a byte-level
  memory model in the runtime package `sc_rt`.
- **C++ in process bodies**: the usual statements (with `switch`
  fall-through), static locals, value-returning and reference-parameter
  member functions, free functions, `<cmath>` functions.

## Not supported

Each of these is reported as an error at its C++ source line, never
simulated approximately:

- `sc_port`/`sc_export` with user interfaces (custom hierarchical channels),
  an `sc_buffer` bound to a port (a port passes on value changes only),
  `sc_mutex`/`sc_semaphore`, approximately-timed TLM (`nb_transport`,
  payload event queues), `sc_spawn`, `SC_FORK`/`SC_JOIN`, `next_trigger`.
- User classes and structs as data (including signal payloads with
  `operator==`/`operator<<`), type template parameters of modules, pointers
  other than byte/word data pointers and class handles.
- C file I/O (`FILE*`), exceptions, `goto`, lambdas.
- The SystemC 3.0 in-class process macros (`SC_METHOD_IMP`, …).

C++ value semantics of class objects are not replicated: a TLM payload
object assigned to another variable is shared, not copied.

## Verification

Every design under `tests/systemc/designs/` (a `.cpp`, or a directory of
`.cpp` files) is checked against the output of the same program built with
the SystemC 2.3.4 reference kernel (`tests/systemc/designs/*.golden`), and
each unsupported construct in `tests/systemc/errors/` must be rejected at its
source line. The mixed designs in `tests/systemc/mixed/` are checked against
the output of the same design written entirely in SystemC
(`reference/all_sc.cpp`). The group is behind a feature, so plain `cargo test` skips it:

    cargo test --features systemc-tests --test systemc

CI runs it (`.github/workflows/systemc.yml`) only when a change touches the
SystemC tests, `src/main.rs` or `Cargo.toml` (the xezim-core pin, where the
lowering lives). To add a design, write it, build it against SystemC and save
its output (banner removed) as the golden file:

    g++ -std=c++17 design.cpp -lsystemc -o ref && ./ref > design.golden From the Accellera SystemC examples, `pipe` and
`reset_signal_is` simulate with identical output; the others need features
listed above.
