# SystemC (IEEE 1666)

xezim simulates SystemC designs: RTL modules, `sc_main` testbenches,
hierarchical and primitive channels, dynamic processes, the report handler
and TLM-2.0 loosely-timed models, with no SystemC library or C++ compiler.
Give it the `.cpp` files like any other source:

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
| `SC_THREAD`, `wait()` / `wait(t)` / `wait(e)` / `wait(e1 \| e2)` / `wait(e1 & e2)` / `wait(t, e)` | an `initial` process with event controls and delays |
| `SC_CTHREAD` + `reset_signal_is` / `async_reset_signal_is`, `wait(n)` | first runs at its first clock edge; restarts on reset |
| `sc_signal` / port writes | update phase (nonblocking); `sc_main` writes apply at the next `sc_start` |
| `wait(SC_ZERO_TIME)`, `e.notify(SC_ZERO_TIME)` | the next delta cycle, after the update phase (a nonblocking `->>` trigger) |
| primitive channel `request_update()` / `update()` | one `update()` per delta cycle, in the update phase |
| `sc_start(t)` | stops before the events *at* `t` (they run in the next `sc_start`) |
| `sc_start()` | returns at `sc_stop()` or when no events are left (the simulator resumes `sc_main` instead of ending); `sc_main` may then start again |
| `sc_stop()` | stops at the end of the delta cycle, with SystemC's message |
| `next_trigger(...)` in an `SC_METHOD` | its next activation waits for that time, event, list or timeout, then for the static sensitivity again |
| `sc_spawn(sc_bind(&f, args…))` | a `fork … join_none` thread (arguments copied at the spawn, `sc_ref`/`sc_cref` shared); `SC_FORK … SC_JOIN` a `fork … join` |
| `before_end_of_elaboration`, `end_of_elaboration`, `start_of_simulation` | at the first `sc_start`, module before children; `end_of_simulation` after `sc_stop` |
| `sc_clock` | period, duty cycle, start time, `posedge_first` |
| time | picoseconds (SystemC's default resolution); `sc_time` prints like SystemC (`1500 ps`, `1 us`) |

## Supported

- **Modules**: `SC_MODULE` and classes deriving from `sc_module` (with
  `SC_HAS_PROCESS` and out-of-class constructors), template modules,
  constructor parameters, child modules (members, `new`, initializer
  lists), named and positional port binding, `sc_vector` ports and signals,
  arrays of signals.
- **Templates**: class templates and module templates with type and value
  parameters (defaults included) are instantiated per argument list, each
  instantiation lowered on its own; a module template with value parameters
  only is one SystemVerilog module with parameters.
- **Ports and channels**: `sc_in`/`sc_out`/`sc_inout` (an `sc_inout` is an
  output its owner also reads), `sc_signal`, `sc_buffer` (an event on every
  write, also through the ports bound to it), `sc_clock`, `sc_fifo` (with its
  depth) with `sc_fifo_in`/`sc_fifo_out`, `sc_event`. An `sc_fifo` makes a
  written value readable, and the space a read frees writable, in the
  update phase, as the reference kernel does.
- **Interfaces, ports and exports** (§5.12–§5.15): interfaces deriving from
  `sc_interface`; `sc_port<IF>` and `sc_port<IF, N>` (multiports with
  `size()` and `p[i]->`), `sc_export<IF>`; binding a port to a channel, to a
  parent's port, or to an export, and an export to a channel or to a child's
  export; `p->m(...)` calls, also from `sc_main` through an export; static
  sensitivity to a port (`sensitive << p`, the channel's `default_event()`)
  or to one of its event functions. A port or export left unbound stops the
  run with SystemC's error E109. Channels are modules (`sc_channel`,
  `sc_module`) or primitive channels (`sc_prim_channel`, with
  `request_update()` and `update()`) implementing one or more interfaces.
- **Dynamic threads** (§5.5–§5.6): `sc_spawn` of member and free functions
  through `sc_bind`, with a result through `&r`, a name and
  `sc_spawn_options` (`set_stack_size` only); `SC_FORK`/`SC_JOIN`;
  `sc_process_handle` with `terminated()`, `terminated_event()`,
  `valid()`, `name()`, `proc_kind()`, `kill()`, `suspend()` and `resume()`
  (a process due to wake while suspended runs when resumed), for spawned
  and static processes (`sc_get_current_process_handle()` in a process, or
  in a constructor after `SC_THREAD`/`SC_METHOD`). Events local to a
  function, passed by reference.
- **Kernel queries**: `sc_is_running()`, `sc_get_status()`; the object
  hierarchy (§5.16): `sc_get_top_level_objects()`, `get_child_objects()`,
  `get_parent_object()`, `sc_find_object()`, with each object's `name()`,
  `basename()` and `kind()` (modules, ports, channels and processes, in
  creation order).
- **sc_main's arguments**: `argv[0]` is `sc_main`, the others are the
  comma-separated values of the plusarg `+sc_args=a,b,…`; `atoi`/`atof`
  convert them. Module constructor arguments become SystemVerilog
  parameters, so they must be known before simulation (constants, or the
  enclosing module's own constructor parameters); a value computed at run
  time is an error.
- **Checks**: `sc_assert` (SystemC's fatal error F4) and `assert`.
- **Reports** (§8): `SC_REPORT_INFO/WARNING/ERROR/FATAL`,
  `SC_REPORT_INFO_VERB` and `sc_report_handler::report`, with the
  reference kernel's rules: `set_actions` per severity, id and (id,
  severity), `suppress`/`force`, `stop_after`, `get_count`,
  `get_new_action_id`, verbosity levels, and a handler of your own
  (`set_handler(&f)`, which may call `default_handler`); `sc_report`'s
  getters. An error's default action (throw) ends the run with status 1
  after the report, as the uncaught exception does. The kernel's own
  messages (sc_stop's) bypass a user handler.
- **Namespace scope**: variables, arrays (sized by their initializer),
  constants of any type and references, in a package all modules import
  (user classes do not see them); free and member functions overloaded by
  their number of parameters, default arguments.
- **Kernel objects**: `sc_mutex` (`lock`, `trylock`, `unlock`, owner-only),
  `sc_semaphore` (`wait`, `trywait`, `post`, `get_value`), `sc_event_queue`
  (several pending notifications, `cancel_all`, as a sensitivity).
- **Destruction**: module destructors run when `sc_main` returns, in C++
  order (last constructed first; a module's member modules after its own
  destructor body).
- **Resolved signals**: `sc_signal_resolved`, `sc_signal_rv<W>` and the
  `sc_in`/`sc_out`/`sc_inout` `_resolved` and `_rv` ports. Every process
  writing one is a driver, through ports of any module or directly; the bits
  resolve as `sc_logic` does (a `0`/`1` conflict gives `X`, `Z` yields), and
  the signal reads `X` until a driver writes. They become nets of a
  resolving nettype, so they cannot cross into SystemVerilog modules.
- **Writer policy** (§6.4.4): with the default `SC_ONE_WRITER`, a second
  process writing a signal stops the run with SystemC's error E115;
  `sc_signal<T, SC_MANY_WRITERS>` allows several writers.
- **Object names**: `name()` and `basename()` of modules, ports, signals and
  the other channels, with SystemC's generated names (`signal_0`, `port_1`,
  …) for objects constructed without one.
- **Data types**: `bool`, `sc_uint`/`sc_int`/`sc_biguint`/`sc_bigint` (up to
  the width SystemVerilog vectors allow), `sc_bv`, `sc_lv`, `sc_logic` (and
  bit-string literals such as `"01XZ"`), the C++
  integer and floating types, `std::string`, `std::vector`, `sc_time`; part and
  bit selects, `(a, b)` concatenation, reductions, `to_uint()` and friends.
- **Output**: `std::cout` (with `hex`/`dec`/`setw`/`setfill`, printing
  SystemC types the way SystemC does), `printf`, `SC_REPORT_INFO/WARNING/
  ERROR/FATAL` in SystemC's format, `sc_trace` to VCD. The exit status is
  the program's: `sc_main`'s return value, 1 after an error report, 134
  after a fatal one (the kernel aborts).
- **TLM-2.0** (§10–§17): `tlm_generic_payload`; blocking and non-blocking
  transport (`b_transport`, `nb_transport_fw`/`nb_transport_bw` with the
  base protocol phases, which print by name), `transport_dbg`, and DMI
  (`get_direct_mem_ptr` with `tlm_dmi`, access through its pointer,
  `invalidate_direct_mem_ptr`). Sockets: `tlm_initiator_socket` /
  `tlm_target_socket` with a module implementing the interfaces
  (`bind(*this)`), the `simple_*` and `simple_*_tagged` sockets with
  `register_*` callbacks (a tag per socket), `multi_passthrough_*` sockets
  (an index per binding, `size()`, `s[i]->`), arrays of sockets, and
  hierarchical binding of a parent's socket to a child's.
  `peq_with_get`, `peq_with_cb_and_phase`, `tlm_quantumkeeper` with
  `tlm_global_quantum`; payload extensions (`set_extension`,
  `get_extension`, `clear_extension`, `deep_copy_from`), memory management
  (`tlm_mm_interface`, `acquire`/`release`), analysis ports and
  subscribers (`tlm_analysis_port`, `tlm_analysis_if`).
  C data pointers (`set_data_ptr(&x)`, `memcpy`, `memset`, pointer indexing and
  `reinterpret_cast` between byte and word pointers) work through a byte-level
  memory model in the runtime package `sc_rt`.
- **C++ in process bodies**: the usual statements (with `switch`
  fall-through), static locals, value-returning and reference-parameter
  member functions, free functions, `<cmath>` functions.
- **User classes and structs**: as objects in process bodies and as signal,
  port and FIFO payloads. Constructors (overloaded, with initializer lists
  and base-class constructors), member functions (overloaded by their number
  of arguments), inheritance and virtual functions, static members, `new`
  and `delete`, and the operators a class defines (`operator==` decides
  whether a signal write is a change; `operator<<` prints the object). C++
  value semantics hold: an object is copied on assignment, initialization,
  by-value parameters and returns, and signal and FIFO writes; pointers and
  references share it. `std::ostringstream` collects text like `std::cout`
  prints it. Classes become SystemVerilog classes in the package `sc_user`.

## Not supported

Each of these is reported as an error at its C++ source line, never
simulated approximately:

- A simple target socket's automatic conversion between blocking and
  non-blocking transport (register the kind the initiators use),
  `sc_event_and_list` / `sc_event_or_list` objects (the `e1 & e2` /
  `e1 | e2` expressions work), spawned method processes and lambdas,
  `SC_CJOIN`, `disable`/`enable` and `reset` of a process handle, report
  log files (`SC_LOG`) and cached reports, `sc_pending_activity`.
- Function templates, pointers other than byte/word data pointers and
  pointers to objects, destructors with statements in a class that is not a
  module (an empty one is fine).
- C file I/O (`FILE*`), exceptions, `goto`, lambdas.
- The SystemC 3.0 in-class process macros (`SC_METHOD_IMP`, …).

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

    g++ -std=c++17 design.cpp -lsystemc -o ref && ./ref > design.golden

The Accellera examples of the SystemC 2.3.4 source release are the wider
acceptance check: `scripts/systemc_examples.py` builds each one against the
installed SystemC, runs it under xezim and reports which match:

    python3 scripts/systemc_examples.py --examples systemc-2.3.4/examples --work /tmp/scx

Seven of them simulate with identical output (`pipe`, `fir`,
`reset_signal_is`, `sc_export`, `simple_fifo`, `forkjoin`, `sc_report`), and
`scx_barrier` with the same lines in a different same-delta order (the
script reports it as ORDER). `rsa` and `pkt_switch` seed their random
numbers from the wall clock, so even the reference output differs between
runs; `simple_perf` passes a run-time value to a module constructor; the
rest need the features listed as not supported: the TLM examples their
C++ library containers (`std::queue`, `std::list`, `std::map`) and pointers
to pointers, others C file I/O and fixed-point types.
