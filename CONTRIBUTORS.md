# Contributors

Thank you to everyone who has improved xezim and xezim-core through pull requests:

- **Thomas Burg** — class-system and UVM fixes: static-property chains through
  object handles (§8.25), associative-array method dispatch and ref-writeback,
  `ClassName::static_prop` access, parser-gap self-tests, test-harness
  hardening, per-process bookkeeping for methods that park mid-body, the
  condition-waiter drain de-duplication, the NBA-region lane in the
  `--max-time` hang report, class-method performance (fast accessor inlining,
  peephole branch fusion), packed-struct rand-array constraints, queue and
  associative-array returns, handle-keyed `foreach` binding, the resume
  order of NBA-region waiters, bytecode compilation of wait-free class tasks
  and package functions, character writes into strings through every
  receiver shape, block-local scoping in tasks and methods, per-invocation
  task-local arrays and array formals that write back to class properties,
  `@(... iff g)` guards evaluated in the waiting process, fork children
  waking a parked parent, scalar `bit` DPI formals that carry one bit, and
  DPI regression tests for real actuals and returns, scalar `svLogic` cells
  and enum prototypes, qualified enum-cast identity, recursive assertion
  nonvacuity, and sampled array elements and indices in assertion actions.
- **[Akash Levy](https://github.com/akashlevy)** — expression-context and
  exponent signedness fixes, nonblocking assignment widths, assertion operand
  widths, callback-driven assertion clocks, cocotb runner arguments, VPI array
  port writes, a nonzero exit status for runs that end on a run-time error,
  assertion `or` beside a pending nonvacuous operand, patches for
  `always_comb` sensitivity, `super` call argument widths and empty
  `foreach` constraints, and extensive self-checking reports covering
  scheduling, types, interfaces and hierarchical access.
- **[Dave Keeshan](https://github.com/davekeeshan)** — nested member selections
  in unpacked parameter arrays, instance-dependent array queries in local
  declaration widths, packed-struct parameter override layouts and
  forwarding, nested struct assignment patterns, port labels through
  part-select actuals, declared index order of instance arrays, packed
  typedef array dimensions, package-scoped typed assignment patterns and
  package arrays of structs indexed at run time, with simulator and core
  regression tests, and wildcard connections to non-ANSI module ports.
- **[Chuanseng Ng](https://github.com/chuanseng-ng)** — array parameter shapes
  in generate scopes and compiled stores to multidimensional unpacked array
  elements, with regressions for packed widths and branch-dependent indices;
  bare scope randomization, constant net drivers after release and root UPF
  power-domain paths.
- **Vrajesh Prakhya** — real-number modelling: Verilog-AMS `wreal` nets
  resolved by summing, user-defined nettypes across the hierarchy and in
  packages (§6.6.7, §6.6.8), real-ness of members projected from call results,
  negative-test registrations, and the diagnosis that `cover property` sites
  were tallied as failing assertions.
- **Oscar Gustafsson** — expanded VPI functionality (`vpi_get_value`,
  `ObjectValType`), CI setup, and clippy cleanups.
- **Chen Ben Haroosh** — submodule-inline generate-for elaboration: genvar-
  dependent declarations and `parameter type` default resolution, plus the
  accompanying SystemVerilog compliance cases, constant-function evaluation,
  nested `foreach` bounds, parameter-dependent packed selects in child
  instances, casts in constant expressions and constant functions (typedef
  width and signing, `$bits` of a cast), array parameters built from
  replication and keyed patterns, port widths of fill-literal actuals,
  member resolution by static type while another process is suspended,
  header defaults that read an instance's type parameter (`parameter int H =
  $bits(T)`), and the array queries of a type parameter answered from the
  type it is bound to (§6.20.3, §20.7).
- **Jayaraman RP** — cross-platform installation scripts, including the macOS
  installer with UVM setup, and `%` and unknown conversion specifiers printed
  literally by `$display` and `$sformatf`.
- **Ganesh T S** — wide-number parsing for `$sscanf`/`$fscanf`/`$value$plusargs`
  and the string conversion methods (arbitrary-precision decimal in the core),
  loop-variable scoping against same-named identifiers in child instances,
  parameter values converted to their declared type's width and sign (§6.20),
  `%c` in `$fscanf`/`$sscanf` matching exactly one character, class and package
  array constants, deferred static and class-field initialization, hierarchical
  class-member access, virtual-interface alias storage, rollback after failed
  randomization, static subroutine randomization targets, class
  specializations keyed by argument type however it is spelled, static
  methods called through a null specialization handle, call actuals sized by
  the callee's formal, the time-zero port handshake between instances, wide
  `rand` draws and `sum()` targets at full width (§18.4, §7.12.3), packed-struct
  patterns laid out into class properties and array elements (§10.9.2), and
  detailed self-checking bug reports for struct and class member access.
- **eenky** — declared defaults for parameters a class leaves out when it
  extends a parameterized class, and virtual-interface reads through
  class-handle chains (`cfg.vif.data`).
- **Francesco Urbani** — `+incdir+` directories in `-F` args files resolved
  against the args file's own directory, and macro arguments kept after a
  token paste that ends the macro's name (§22.5.1).
- **[Adam Auten](https://github.com/auty1395)** — elaboration fixes in the core
  with a simulator test for each: array parameters whose dimensions name
  imported parameters or other array parameters, element selects of packed
  multi-dimensional package parameters, a body struct localparam in a
  module's first instance, constant concatenations in integer constant
  contexts, `always` as a property operator, continuous assigns of one row of
  a 2-D array or of a whole array with a hierarchical side, relative instance
  references inside nested generate scopes, package delays under a
  command-line timescale, bind actuals the target does not declare, class
  associative-array keys sized per specialization, and specialization
  arguments written as expressions of package parameters keyed by their
  value (§8.25).
- **Taichi Ishitani** — constrained-random fixes (members of rand object array
  elements, `pre_randomize` before the rand and constraint modes are read,
  `this.` member paths in the joint solver, and `XEZIM_RAND_DIAG` reports of
  why `randomize()` failed), named `extends` lists and type-parameter formals
  resolved from the receiver's own bindings, class handles resolved before
  same-named structs, unpacked-struct mailbox messages through task formals,
  and array signs preserved through subroutine frames.
- **AaronKel** — bytecode-compiler rollback that discards stale jump fixups
  (an early `return` inside a loop that cannot be unrolled used to crash the
  compiler or hang the caller's loop), and nested type arguments kept in
  static-call specializations, so `uvm_config_db#(cfg#(AW,DW))::get` reaches
  the configuration that was set, and factory `type_id::create` through chains
  of class typedefs; reclaiming class objects no handle reaches (§8.7), with
  the fixtures for every place a handle can hide; virtual interfaces bound
  through inherited and forwarded type parameters, own-definition-name paths
  and indexed receivers (§8.25, §25.9); class events, clocking blocks and
  `@vif.cb` waits (§14, §15.5); fork-child state shared only within its
  activation and released on every disable path (§9.3.2, §9.6.2); a broad set
  of constrained-random fixes for `inside`, `foreach`, enum domains and
  context widths (§18); extern-method linking through package class aliases
  and linear macro line joining in the core (§8.24, §22.5.1); the
  execution-cache releases and associative-array key caching that keep long
  runs fast; class methods that compile more often and call each other
  directly, with compiled results that match the interpreter's for member
  operand widths and signs, unpacked formals and short-circuit operators
  (§11.4.7, §11.6.1); parked `wait` conditions re-checked only when an
  operand may have changed, and the class heap collected incrementally from
  the previous survivors (§8.29); assigned class-member events that share one
  event object (§15.5.5), `$bits` of dynamic operands (§20.6.2), `return` and
  associative-element stores evaluated in their target's width (§11.6.1),
  stores through a handle that a same-named enum local had hidden, and string
  bytes read straight from the value words in the core.
- **[Hern](https://github.com/hernloh)** — labelled assertion and cover sites
  named in the coverage results file.
- **[opensource-elearning](https://github.com/opensource-elearning)** — reads
  of an inactive tagged-union member reported as errors.
- **[lwflwf1](https://github.com/lwflwf1)** — parser fixes for constraint
  braces, implication operands and `randomize` blocks, packed-dimension
  recovery and closing directives inside statement blocks, plus the report
  that led to parser tests and minimum-version checks in core CI.
- **[ds54e](https://github.com/ds54e)** — elaboration fixes in the core with
  a simulator test for each: casts sized by a value parameter inside constant
  functions (§6.24.1, §13.4.3), and element selects of packed localparams
  declared in a child module's body (§7.4.1, §7.4.5); CLI operation under
  address-space limits, femtosecond runtime caps, finished-fork metadata
  reclamation and shared task continuations, with memory and checkpoint
  regression tests.
- **[git-hevangel](https://github.com/git-hevangel)** — shared statement-list
  numbering when restoring checkpoints of processes that have forked,
  including suspended class tasks and condition or event waiters.
- **[Philip Axer](https://github.com/philipaxer)** — nested collections typed
  by their declared shape at every level: the shape recorded in the core for
  every scope, rows created as their parent's kind allows, and `%p` and
  `foreach` over queue and dynamic outer levels (§7.4.5, §7.8, §7.10); string
  keys that hold brackets, locator methods that search associative arrays by
  key, queue methods and reductions on a locator's result, rows of nested
  collections as locator receivers, string keys ordered as strings, and enum
  labels in `%p` of a locator result (§7.8, §7.12, §21.2.1.6); and time
  literals and `$time` in packages and `$unit` scopes scaled by their own
  time unit (§3.14.2.3); shared package import bindings, type-aware prepared
  body keys and narrower parameter lookups during elaboration; and `$` queue
  selects evaluated with their own queue bounds in compiled code (§7.10);
  and generate blocks as scopes: a scope tree and name binder per definition
  that the generate rename follows, unlabelled-block data, bare upward calls,
  `disable` targets and typedefs bound through it, and undeclared names
  reported by binding (§23.9, §27.6, #952).
- **[Shabbar Vejlani](https://github.com/shabbarvejlani)** — class-member
  element selects sized at their declared width in compiled methods
  (§11.6.1), and `foreach` modulo equalities solved by drawing each element
  from its congruence class (§18.5.8.1).

## Bug reports

Detailed, self-checking bug reports have driven much of xezim's conformance
work. Thank you in particular to
[AaronKel](https://github.com/AaronKel),
[akashlevy](https://github.com/akashlevy),
[amal-khailtash](https://github.com/amal-khailtash),
[auty1395](https://github.com/auty1395),
[bharathnaruto-spec](https://github.com/bharathnaruto-spec),
[chenbhil](https://github.com/chenbhil),
[chuanseng-ng](https://github.com/chuanseng-ng),
[davekeeshan](https://github.com/davekeeshan),
[dhanunay](https://github.com/dhanunay),
[Dragon-Git](https://github.com/Dragon-Git),
[ds54e](https://github.com/ds54e),
[Ganesh-TNL](https://github.com/Ganesh-TNL),
[git-hevangel](https://github.com/git-hevangel),
[hernloh](https://github.com/hernloh),
[jjts](https://github.com/jjts),
[Lramseyer](https://github.com/Lramseyer),
[lwflwf1](https://github.com/lwflwf1),
[overcookedlobster](https://github.com/overcookedlobster),
[philipaxer](https://github.com/philipaxer),
[rharikrishna25](https://github.com/rharikrishna25),
[rpjayaraman](https://github.com/rpjayaraman),
[shabbarvejlani](https://github.com/shabbarvejlani),
[soronpo](https://github.com/soronpo),
[taichi-ishitani](https://github.com/taichi-ishitani),
[urbanij](https://github.com/urbanij) and
[vrajeshprakhya](https://github.com/vrajeshprakhya).

The elaboration-memory investigation in #721 uses synthetic reproducers,
allocation profiles and controlled type-context comparisons supplied by
[amal-khailtash](https://github.com/amal-khailtash).

New contributors are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md).
