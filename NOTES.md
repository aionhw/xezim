# xezim technical notes

Release notes, verified workloads and compliance results. The user guide is
in [docs/user-guide.md](docs/user-guide.md); building and contributing are in
[docs/building.md](docs/building.md) and [CONTRIBUTING.md](CONTRIBUTING.md).

# What's new in 0.11

### Unreleased

**Correctness**

* A run that ends on a run-time error exits 1, as one that ends on `$fatal`
  does (§20.10): a read or a task call through a null handle in a process
  body, or a wait on a mailbox that was never constructed (§8.4); a `forever`
  with no timing control, or a zero-delay loop, that can never let time
  advance; a nonblocking-assignment cascade that does not converge; a
  `foreach` that makes no progress; an SDF file `$sdf_annotate` cannot read or
  parse; and a failed internal consistency check. Each printed its error and
  stopped the run, but the exit status was 0, so a script or harness that
  checks it took the run for a pass.

### 0.11.3 — hierarchy, interfaces, assertions and collection correctness (October 2026)

**Correctness**

* A hierarchical name that does not resolve is an elaboration error (§23.8),
  wherever it is written: a read or a write, a continuous assignment, an
  event control, an assertion, a system task argument, or a task or function
  that is never called. The message gives the file and line, the name, the
  scope that does not declare it and the scope the search started from. Such
  a name was accepted silently: a testbench that wrote to a guessed internal
  name (`dut.mem_data[3] = ...`) ran as if the write had happened, and a read
  returned x. A name inside a generate block a constant rules out is not
  elaborated, so it is not checked. `--no-strict` turns the check off, for
  designs that relied on the old behaviour.
* An assertion attempt at time 0 reads default sampled values (§16.5.1): a
  variable's declaration initializer, else its type's default. It read the
  values the time-0 continuous assignments and `always_comb` blocks had
  already driven, so under a clock that rises at time 0 (an HDL clock, or
  cocotb's Python `Clock`) such an attempt passed. `$past` at the next tick
  reads the same samples (§16.9.3).
* The optional SoA signal store preserves X/Z when a four-state signal starts
  with a known value, and converts both X and Z to zero for explicitly declared
  two-state signals, including vectors wider than 64 bits (§6.11.2).
* Native case equality sign-extends signed operands of different widths,
  including comparisons against constants, matching four-state execution
  (§11.4.5).
* Toggle coverage no longer lists the names of the top module's instance
  arrays, instances in generate blocks and generate blocks as 1-bit signals.
  They are scopes, not signals, so their bins could never be covered and the
  top instance's toggle score read low (§23.6). (#441)
* A value a VPI client writes at time 0 is the toggle-coverage baseline.
  Values written from the end-of-slot callbacks (`cbReadWriteSynch`,
  `cbAtEndOfSimTime`) reach toggle coverage and the waveform dumps in their
  own time slot, not the next one. A value written from
  `cbStartOfSimulation` is evaluated in the time-0 slot even when nothing
  else runs there, so the logic it drives no longer reads x until the next
  slot. cocotb writes its first values from both, so in a design with
  nothing of its own at time 0 every signal those values reached lost its
  first toggle. A toggle is counted once per time slot from the slot's net
  change (§38.36.2, §38.36.3). (#442)

* Class task handles and local variables remain alive across same-slot
  clocking waits, including guarded waits and cancellation (§14.13).
  (#335, reported by urbanij)
* Parameterized class constructor statics initialize once per referenced
  specialization before initial blocks, with the declared target type and
  constructor arguments preserved (§6.8, §8.25). (#338, reported by Dragon-Git)
* FST value records retain their declared widths when runtime storage uses a
  different width. Unsupported variable-length strings are omitted with a
  warning naming them, preserving the rest of the waveform (§6.16, §10.8).
  (#339, reported by Lramseyer)

* Integer `inside` constraints evaluate size-cast bounds in the joint
  comparison context and sample uniformly over distinct legal values,
  including overlapping ranges and repeated items. Explicit `dist` weights
  retain their meaning (§11.4.13, §18.5). (#336, reported by urbanij)
* Indexed writes create associative entries below dynamic outer dimensions
  in class properties, method locals and struct members, preserving declared
  key types and independent rows through allocation, copying and resizing
  (§7.5, §7.8). Recompile saved designs to refresh their class metadata.
  (#337, reported by Dragon-Git)

* Named blocking `case` branches retain their `%m` scope and `disable`
  targets across suspension (§9.3, §9.6.2).

* Variables declared in a `for` header retain their signed type across
  suspension and restore the enclosing variable when the loop exits (§12.7.1).
  (#341, reported by urbanij)
* Bit and part writes through locals, formals and instance properties use
  their own storage even when a module variable has the same name (§23.9).
  (#344, reported by akashlevy)
* Static `always` block locals retain their values when another task has
  suspended, and concurrent or nested declarations retain independent
  storage, widths, signing and string kinds (§6.21, §13.3).
  (#345, #348 and #350, reported by akashlevy)
* Compiled typedef locals, formals and results retain their declared width,
  signing and two-state defaults, including scoped aliases and alias chains (§6.18).
  (#362, reported by akashlevy)
* `always` waits on clocking events resume after sampling and NBA updates.
  Callback-driven clocks deliver those events in the same slot and preserve
  the slot-entry samples of `#1step` inputs (§14.4, §14.13, §38.36).
  (#356 and #357, reported by akashlevy)

* Streaming concatenations read every element of class-member queues and
  arrays, including slices and reversed byte streams (§11.4.14).
  (#340, reported by urbanij)
* Arrays of module instances split ANSI-port and expression connections,
  preserve declared direction in every dimension, and drive output
  concatenations through their writable parts. Unpacked connections match
  the instances left to left (§23.3.3.5).
  (#342 and #343, reported by akashlevy)
* Nested packed selections retain declared dimension ranges and struct
  member offsets in locals, formals, results and unpacked-array elements.
  Selected writes to static class properties update their shared storage
  (§7.4.1, §8.9, §11.5.1).
  (#346 and #352–#354, reported by akashlevy)
* Explicit static subroutine initializers run once before initial processes.
  Static subroutine arrays, queues and associative arrays retain one store
  across calls and recursion (§6.21, §13.3, §13.4).
  (#349 and #355, reported by akashlevy)

* Unpacked port connections match elements in declaration order, including
  descending and negative ranges, slices and multidimensional arrays (§7.6).
  (#358, reported by akashlevy)
* Unary minus preserves operand signing and width, packed string literals use
  integral ordering in numeric comparisons, and casts through typedef chains
  retain signing and convert X/Z to zero for two-state targets (§5.9, §6.18,
  §6.24.1, §11.8.1). (#359–#361 and #377, reported by akashlevy)
* Sampled-value functions retain explicit clocks, independent history depths,
  gated samples and slot-entry values of aggregate fields. Assertion action
  blocks evaluate their gates after NBA updates (§16.9.3).
  (#363, #364, #366 and #370, reported by akashlevy)
* Fixed-array declaration patterns initialize every element, and whole-array
  nonblocking assignments snapshot their right-hand sides before scheduling
  updates, including negative ranges and multiple dimensions (§10.4.2, §10.9.1).
  (#365 and #381, reported by akashlevy)
* Long assertion delays and repetitions retain their complete bounds.
  Followed-by properties parse with right association and evaluate alternative
  sequence matches without treating an unmatched antecedent as vacuous success
  (§16.9.2, §16.12.9). (#367 and #375, reported by akashlevy)
* `$finish` stops procedural, compiled and callback-driven execution immediately.
  Final procedures run before end-of-simulation callbacks (§9.2.3, §20.2).
  (#371–#373, reported by akashlevy)
* Generated assertions substitute their genvar values, and unnamed generate
  loops expose canonical scope names in VPI, waveforms and coverage (§27.4, §27.6).
  (#374 and #380, reported by akashlevy)
* Compiled class methods retain each specialization's local types, widths,
  signing and value sets, including inherited bindings. Qualified built-in
  types remain distinct from typedef names (§8.25).
  (#378, reported by akashlevy)
* Added regressions for signed inlined input formals and timed loop scope
  cleanup (§12.7.1, §13.5.1). (#376 and #379, reported by akashlevy)

* Instance arrays resolve parameter-named sizes before expansion, including
  imported, scoped and overridden parameters and multiple dimensions. Interface
  elements retain their clock and reset connections (§7.4.2, §23.3.3.5).
  (#342 and #343, reported by akashlevy)

* Nonblocking selections of static subroutine locals preserve NBA-region
  ordering, capture indices when scheduled, and retain their declaration-owned
  storage after return or suspension. Partial and delayed writes preserve
  unrelated bits and source order (§6.21, §10.4.2, §23.9).
  (#368, contributed by akashlevy)
* Nested packed local selections retain enum base ranges, ascending labels,
  partial out-of-range behavior and live static storage. Two-state struct
  members convert X/Z independently of four-state neighbors
  (§6.19, §6.21, §7.4.1, §11.5.1).
  (#369, contributed by akashlevy)

* Initializers accepted with `--relax-implicit-static` run once before initial
  processes and retain their values across calls and activations. Design caches
  distinguish this lifetime mode from the default policy (§6.21).
  (#351, reported by akashlevy)
* Static class calls retain lexical scope and specialization through module
  type parameters, typedef chains, nested classes and instance handles.
  Suspended static tasks retain their bindings, and class event OR waits wake
  once on the selected event (§8.23, §8.25, §9.4.2, §13.4.1, §15.5).
  Recompile saved designs to refresh scoped class metadata.
  (#382, reported by Ganesh-TNL)
* Randomization callbacks reject tasks, non-void returns and argument lists
  before execution, preserving legal inherited and overridden callbacks
  (§18.11). (#383, reported by rpjayaraman)
* Unsigned expression context reaches a shift's left operand before extension
  and evaluation in compiled and interpreted execution. Self-determined
  boundaries and function side effects are preserved (§11.8.2, §11.4.10).
  (#384, reported by akashlevy)

* Concurrent assertions and covers observe clock edges at time zero and
  four-state clock transitions, using Preponed samples (§16.5, §9.4.2).
  (#385, reported by akashlevy)
* Assertion action blocks preserve generated scope and keep their local
  declarations separate from module variables (§16.14.1, §23.9, §27.4).
  (#386, reported by akashlevy)
* Returning an unpacked function-name variable no longer recurses. Dynamic
  arrays, queues and fixed arrays retain their elements across return,
  including descending fixed ranges (§13.4.1, §7.6).
  (#387, reported by Dragon-Git)


* Queue elements created from struct-returning expressions retain their
  string and other members when passed to functions or copied. Arguments
  run once even when a struct's leaf layout is unavailable
  (§7.10.2.2, §13.4.1, §13.5.1).
  (#390, #392, contributed by Ganesh-TNL)
* Variables declared in `for` headers retain their declared signedness
  across synchronous and suspended loops, including typedefs and unsigned
  initializers (§6.11.1, §11.8.1, §12.7.1).
  (#394, contributed by akashlevy)

* Full-word concatenations preserve value and X/Z bits without overflowing
  a native shift in debug execution, including nested and wide-block
  forms (§11.4.12).

* Negated weak sequences fail when the clock stops with an attempt still
  pending. Nested negation preserves sequence strength, and `disable iff`
  cancels attempts even without another clock edge (§16.12.2–3).
  (#393, reported by akashlevy)
* Followed-by properties settle at the first successful consequent tick,
  including vacuous successes, so later matches cannot add pass actions,
  cover hits or end-of-simulation failures (§16.12.9, §16.14.8).
  (#395, reported by akashlevy)

* Tiered string casts convert bytes without padding concatenations, including
  casts through typedefs. NUL removal and byte values agree with interpreted
  calls, and persistent methods recompile for the new conversion
  (§6.16, §6.18, §6.24.3, §11.4.12).
  (#391, reported by urbanij; #396, contributed by Ganesh-TNL)

* Interpreted nonblocking and event-controlled assignments evaluate their
  right-hand sides at the target width, preserving carries and unary results
  (§11.6.1). (#399, reported by akashlevy; #407, contributed by akashlevy)
* Concurrent assertion operands and local match assignments retain their
  expression context width (§11.6.1, §16.10).
  (#402, reported by akashlevy; #408, contributed by akashlevy)
* VPI-driven clocks evaluate assertions in the edge's time slot, with fixed
  slot-entry samples and Post-NBA callbacks before assertion evaluation
  (§16.5.1, §4.4.2). (#404, reported by akashlevy; #409, contributed by akashlevy)
* The cocotb runner honors build arguments, timescales and parameter overrides,
  including later tests and positional arguments.
  (#406, reported by akashlevy; #410, contributed by akashlevy)
* VPI writes through unpacked-array net ports reach every alias. Variable
  connections retain their propagation timing (§23.3.3.3, §23.3.3.7).
  (#405, reported by akashlevy; #411, contributed by akashlevy)
* Sampled-value functions use declaration defaults from before time zero,
  including deferred initializers and procedural assertions (§16.5.1, §16.9.3).
  (#403, reported by akashlevy; #412, contributed by akashlevy)
* Select writes to static subroutine locals update their shared storage and
  preserve packed member writes (§6.21, §11.5.1).
  (#413, reported by akashlevy; #414, contributed by akashlevy)
* Concurrent assertions sample unpacked-array elements, including large
  memories, variable indices and forced values. Out-of-range slice sampling
  clips safely at extreme indices (§16.5.1, §11.5.1).
  (#415, reported by akashlevy; #416, contributed by akashlevy)
* Named and inline assertions sample generate-block, instance-array and
  top-rooted references and recognize hierarchical clocks (§16.5.1).
  (#417, reported by akashlevy; #418, contributed by akashlevy)

* Package-scoped virtual interfaces accept direct declarations and typedefs,
  preserve qualified copies and equality, and retain independent bindings
  when names are imported or shadowed (§25.9, §26.2, §26.3).
* Virtual interfaces in string-keyed associative arrays preserve their keys
  through member access and handle copies, including inherited properties
  and method-local keys (§7.8, §25.10).
* Returned virtual interfaces support member reads, writes, task calls and
  selected nonblocking targets. Receiver calls run once, and nonblocking
  targets retain their assignment syntax (§10.4.2, §25.9).
* Parameterized static collections retain virtual-interface element identity
  through typedefs, named type arguments, reordering and static methods
  (§8.25, §25.10).
* Copying or returning a null virtual interface clears its previous binding.
  Explicit virtual-interface locals default to null (§25.9, Table 6-7).
* Instance-targeted binds resolve indexed and nested generate scopes,
  including constant index expressions and explicit target lists, while
  leaving sibling instances unbound (§23.11, §27). VPI retains the host's
  source definition and bound port metadata (§37).
* Generated module instances use signed genvars, preserving negative indices
  and terminating descending loops correctly (§27.4).

* Virtual-interface collection copies preserve element identity. Queue edits,
  associative deletion and dynamic-array allocation use the current values,
  and absent elements read as null (§7.5–§7.10, §25.10).
  (#427, reported by Ganesh-TNL)
* Task `ref`, `output` and `inout` virtual-interface arguments propagate
  rebinding and null values, preserve input isolation and retain their scope
  across suspension (§13.5, §25.9).
* Bind targets resolve against the effective hierarchy and declaration scope,
  including relative and `$root` paths, instance arrays, interface hosts,
  active generate branches, type parameters and `defparam` overrides. Repeated
  binds retain upward module references and VPI port metadata (§23.11, §37).
  Bare target names prefer the nearest visible instance over a same-named type.
  Recompile saved designs to refresh their hierarchy metadata.

* Interface static initializers evaluate runtime system functions in each
  elaborated instance's scope before user processes, including nested,
  generated, arrayed and bound interfaces. Already deferred initializers
  retain their once-only side effects (§6.21, §25.3).
  (#435, reported by AaronKel; #436, contributed by AaronKel)

* Compilation-unit nets retain pull values, drive strengths, resolution and
  continuous declaration assignments, including when a module shadows a unit
  name. Explicit unit writes remain distinct from later module declarations.
  Parser source locations, interface-property parameters, directive recovery
  and constraint braces and implications are also corrected.
  (core #55–#63, contributed by AaronKel, Thomasb81 and lwflwf1)
* User constant functions select generate branches and initialize packed
  struct fields with their full argument and result widths.
  (#460 and core #64, contributed by chenbhil)
* Static virtual-interface comparisons and method receivers share storage
  through class typedef aliases. Collection identity coverage is retained.
  (#450 and #456, contributed by Ganesh-TNL)
* Assertion action scopes, pending property disjunctions and sampled operands
  follow their declaring instance, end-of-simulation outcome and Preponed
  values. (#457; #419, #420 and #421, contributed by Thomasb81)
* Hierarchy diagnostics and time-zero assertion and toggle coverage fixes
  include scope and foreign-write regression coverage.
  (#439, #440, #443 and #444, contributed by akashlevy)
* Continuous assignment sensitivity includes unnamed generate-block reads.
  (#434, contributed by Thomasb81)

* Source locations preserve original columns after empty and multiple macro
  expansions, and identify each invocation independently (§22.5). Added
  virtual-interface recovery and postfix-span regressions.
  (#397 and #398, reported by lwflwf1)

* Undefined statement macro calls recover as one placeholder, preserving the
  undefined-macro diagnostic and the following statement (§22.5.1).
  (#451, reported by lwflwf1)
* Assertion, property and sequence clocking events accept `or` and comma
  lists, including nested groups and per-term edge and `iff` conditions.
  Each qualifying event advances the assertion and its sampled history
  (§16.5, §9.4.2). Recompile saved designs.
  (#453, reported by lwflwf1)

* The cocotb runner reloads parameters when another runner rebuilds the same
  directory. Test overrides persist while the recorded build is unchanged,
  including empty overrides. (#426, reported by akashlevy)

* Interface-array slices retain their selected instances' parameters, signal
  widths and connections when passed to array ports or forwarded through
  another module. (#463)
* Fixed arrays of associative arrays apply each row's declared key width and
  signedness to lookups, iteration and traversal, including class properties
  and automatic task storage. (#464, reported by chenbhil)
* Continuous assignments and `always_comb` writes preserve the complete width
  of nested packed elements and sub-arrays, including packed struct elements
  wider than 32 bits (§7.4.1). (#461, contributed by chenbhil)

* Nested implications and property `if` preserve vacuous successes through
  their consequents, so `$assertvacuousoff` suppresses their pass actions.
  Nonvacuous successes remain recorded across delayed and multiple matches
  (§16.14.8, §20.12). (#445, reported by akashlevy)

* Static virtual interfaces retain their specialization through qualified,
  typedef and object-member reads, inheritance, copies, null assignments and
  output writeback. Nested local bindings restore the enclosing interface
  after function, task and loop scopes end (§6.21, §8.26, §25.9).
  (#431, #433; #465 and #466 contributed by Ganesh-TNL)

* Virtual-interface formals preserve the bindings of call and conditional
  arguments without repeating getter calls or leaking their bindings into
  scalar results. Output formals start null and copy that default back when
  left unassigned (§13.5.2, §25.9).

* Hierarchical references through interface ports resolve to the connected
  interface instance, including modports, nested connections and generated
  instances. Reads, writes, waits, events and subroutine calls share that
  instance's storage (§23.8, §25.3). Recompile saved designs to refresh their
  interface-port connections.

* Unpacked parameter arrays preserve declared bounds, element types and
  nested selections across package imports and hierarchical reads. Instance
  body overrides and array-dependent constants use the selected values
  (§6.20.2, §26.3). (#425, reported by Ganesh-TNL;
  #467 and companion core changes contributed by Ganesh-TNL)

* Assertions and sampled-value functions continue to tick when their clock
  is reached through an interface port from a nested scope (§16.9.3, §23.8).

* `foreach` walks fixed outer dimensions and each inner collection in
  declaration order, including descending bounds, inherited properties and
  hierarchical module arrays (§12.7.3). (#492, contributed by chenbhil)
* Assertion actions read Preponed samples even when the property does not
  read the operand. Omitted virtual-interface arguments bind their defaults
  once in the declaration scope and preserve input/output identity (§16.9.3,
  §13.5.3). (#468 and #469, reported by akashlevy;
  #493 contributed by Thomasb81)
* Assertion `until`, `s_until` and `iff` evaluate property operands and pending
  obligations, with strong end-of-simulation failure and Boolean X/Z truth
  handling (§16.12.8, §16.12.12). Pure constraints in concrete classes and
  parameter initializers that read specify parameters receive scoped
  diagnostics (§18.5.2, §6.20.5). Recompile saved designs to refresh their
  declaration metadata. (#493 and core #81, contributed by Thomasb81)
* Compilation-unit constant functions are available to program and interface
  constants, including parameter headers (§13.4.3).
  (#488, reported by Dragon-Git)
* Body type parameters retain widths, aliases, signing and class
  specializations. Module instances bind their own type overrides (§6.20.3,
  §8.3). (#498, reported by Dragon-Git)

* Class instance and static property assignments take the declared signing
  even when the right-hand side already has the destination width. Hidden
  base properties use their own declarations and type bindings (§10.7, §8.14).
  (#499, reported by akashlevy)
* Procedural bitwise `&`, `|`, `^` and `~^` results are signed when both
  operands are signed, preserving arithmetic and comparison behavior
  (§11.8.1). (#500, reported by akashlevy)
* Arithmetic right shifts replicate X and Z sign bits in narrow values and
  retain the left operand's signing for unknown amounts and overshifts
  (§11.4.10).

* Class parameter and localparam arrays preserve specialization bindings,
  element types, declared index order and assignment-pattern keys across
  class, object and method access. Recompile saved designs to refresh their
  array declaration metadata
  (§6.20.2, §8.25, §10.9.1).
  (#423, #503 and xezim-core#82, reported and contributed by Ganesh-TNL)
* Deferred static initializers run once in each elaborated instance,
  including bound instances, generated scopes and instance arrays (§6.21).
  (#494 and #504, reported and contributed by Ganesh-TNL)
* Class fields initialize once in declaration order after the base constructor,
  using their declared assignment types and lexical scope. Dynamic-array and
  class-typed field constructors retain their sizes and type bindings (§8.7).
  (#496 and #505, reported and contributed by Ganesh-TNL)
* Hierarchical class-member access follows nested object handles and updates
  selected bits in instance or shared static storage. Same-named observer
  variables cannot redirect hierarchical struct accesses (§8.4, §11.5.1, §23.7).
  (#497 and #506, reported and contributed by Ganesh-TNL)

* Unsigned expression types reach context-determined operands before nested
  division, modulus and widening are evaluated (§11.8.1/§11.8.2). Compiled
  expressions also recognize unbased fill literals as unsigned. Signed array
  elements and increment/decrement results retain their declared types
  (#501, reported by akashlevy).

* Packed selects in parameterized child instances use each instance's
  elaborated dimensions, including typedef element widths and hierarchical
  reads (§7.4.1/§23.10; #507, contributed by chenbhil).

* An exponent keeps its own signedness and width when a surrounding
  expression is unsigned (§11.6.1). Continuous and procedural results agree
  (#502, contributed by akashlevy).
* Enum casts retain their declaring type through package and class names,
  including same-named classes in different packages (§6.19.3). Assigning a
  cast of a different enum is diagnosed (#511, contributed by Thomasb81).
* Assertion `or`, `and`, `iff` and `until` preserve recursive nonvacuity
  (§16.14.8). Completed `until` ticks are folded so long attempts retain only
  pending evaluations (#512, contributed by Thomasb81).
* `$sampled` in assertion actions reads unpacked-array cells and variable
  indices from the same sampled edge (§16.5.1), including indices changed by
  an NBA (#514, contributed by Thomasb81).
* Whole references to class and package array constants retain specialized
  bounds, query direction and declared element order when copied (§7.6,
  §20.7), including automatic destinations (#516, contributed by Ganesh-TNL).
* Static initializers that depend on deferred values run in declaration
  order, with function calls executed once (§6.21). Package ownership and
  dependencies in casts, slices and replications are preserved (#517,
  contributed by Ganesh-TNL).
* Failed `std::randomize` calls restore scalar values, array elements and
  dynamic collection sizes, including early unsatisfiable size constraints
  (§18.12). Static subroutine locals participate in joint constraints
  (#518 and #520, contributed by Ganesh-TNL).
* Array queries in procedural packed ranges use the current instance or
  shadowing local array (§20.7), including generated scopes and static tasks.
  Width lookup visits only queried arrays (#524, contributed by davekeeshan).
* Forwarded packed-struct parameters retain their field layout before later
  header localparams are evaluated (§6.20, §23.10), including type bindings
  and `defparam` overrides (#525 and xezim-core #85, contributed by davekeeshan).
* Constant nested member selects of packed structs in unpacked parameter
  arrays retain their values and `$bits` widths (§7.2.2), including qualified
  package arrays and compilation-unit typedefs (xezim-core #83, contributed
  by davekeeshan). Saved compiled designs affected by this constant-folding
  bug, the struct-parameter override bug or the static-initializer bug above
  require recompilation.
* Class methods declared inside modules call tasks in the object's declaring
  instance (§23.8), including blocking calls through sibling and descendant
  instances. Package events keep their own identity through scoped and imported
  names, aliases and same-named declarations (§15.5, §26.3), including triggers
  from package tasks (#526, contributed by aionhw). Saved compiled designs
  affected by package event name collisions require recompilation.
* Package typedef aliases and imports retain collection dimensions and
  packed element widths (§6.18, §26.3), including queues inside associative
  arrays of structs and same-named scalar aliases. This completes the
  xezim-core #83 namespace follow-up and preserves the #323 case reported
  by Ganesh-TNL.

**Performance**

* Dense `case` statements with four to seven labels use jump tables, and
  comparison and constant-operand fusion recognizes temporaries reused by
  later case arms while preserving values still read on another control-flow
  path. Jump-table targets are relocated when NBA-only edge blocks merge,
  preserving that optimization for dense cases (§12.5).
* Constant equality comparisons feeding a branch execute as one bytecode
  instruction when their result is otherwise unused, preserving X/Z semantics
  and native execution support (§11.4.5, §12.4).
* Repeated named-event triggers reuse resolved signal targets and dense
  trigger timestamps, reducing name allocation and lookup work while retaining
  event aliasing and scheduling behavior (§15.5).
* Constant equality branches also execute as one instruction in the two-state
  paths, preserving comparison results, branch targets and X/Z fallback.
* Event-reference checks borrow ordinary names and alias targets, reducing
  temporary string allocation during procedural assignments.

* Class-heavy simulations share suspended process frames, reuse object
  initialization templates, and avoid unnecessary interpreter lookups.
  An explicit `XEZIM_METHOD_TIER` also attempts compilation on a later call
  after enough interpreted loop iterations, including `foreach`; default
  admission continues to use call counts. (#326, contributed by Thomasb81)

* The integrated correctness fixes use 1.18% more host instructions on AXI4
  and 0.004% more on C906 CoreMark than the previous main revision
  (`5cd31ee5`). Outputs match, and C906 guest cycles are unchanged. These
  measurements cover two interleaved AXI4 pairs and one C906 pair with one
  CoreMark iteration; they do not establish a general performance trend.

**Command line**

* `--tee <file>` copies stdout/stderr and DPI/VPI output to a transcript while
  retaining console output. `-vcd` and `-vcd dumpon` enable source-driven VCD
  dumping like `--wave`. (#347, reported by akashlevy)

**Maintenance**

* Core CI runs the parser crate's unit, integration and documentation tests,
  and verifies its minimum supported Rust version explicitly.
  (#455, reported by lwflwf1)

* Expanded regression coverage for default-valued array arguments and nested
  associative collections, including repeated calls and queue element isolation.
  (#332 and #333, contributed by Thomasb81)
* Rust warning cleanup preserves optional native execution and successful
  two-state dynamic operations. (#334, contributed by Thomasb81)

* Shortened simulation windows and optimized dependency test builds reduce
  regression runtime. Coverage analysis stops on failed or unprofiled tests.
  (#458, contributed by Thomasb81)

**Validation**

* The release passes 5,412 default debug tests in CI, with one existing
  ignored test, plus two JIT smoke tests. Core and parser CI pass 98 and
  142 tests respectively.
  The 32 DPI tests pass on Linux's nested-scheduler fallback and on macOS
  with both fibers and the fallback. Both repositories pass their minimum
  supported Rust version checks.
* Local release checks pass 4,830 related simulator tests, 98 core tests and
  142 parser tests. Production-build checks pass 211 saved-design and cache
  checks across 24 fixtures, six AVIP workloads, and C906 hello with output
  identical to the saved reference. The C906 CoreMark comparison also
  preserves the full program output and 295,294 guest cycles.
* Fresh 0.11.3 measurements cover C906 CoreMark, C910 memcpy with 200
  iterations and five AXI4 runs. C910 completes the 1,024-byte copy in 216
  guest CPU cycles. The README reports the measured elapsed times and
  distinguishes the historical reference timings from the new xezim runs.

### 0.11.2 — class, collection and DPI parity fixes, `-uvm`, crash-safe FST dumps (October 2026)

**Correctness**

* A type name declared in more than one package (`pkg_one::blk_t` and
  `pkg_two::blk_t`, or a package and `$unit` or a module) resolves by the
  §26.3 rules instead of to the package elaborated last: `$bits`, struct
  layouts, enum members, `rand` members and same-named classes each come
  from their own package, so `randomize()` succeeds. (#300, reported by
  Ganesh-TNL)
* Unpacked-array, struct and dynamic-array properties of a class start at x
  for 4-state element types when the object is constructed (§6.8 Table
  6-7); they used to start at 0. A test pins the time-0 value of every
  kind of declaration.
* In a method of a class with an `int s` property, `s = v` writes the
  property even when another class has a struct property `s` and the module
  has a struct variable `s`; the write used to land in the module struct.
* A method called more than 1000 times reads `h.val` through a handle of
  class `node` from `node`'s `val` when the object is a subclass that
  redeclares `val`; it used to read the subclass's copy once compiled.
* A character write `s[i] = c` changes one character of a string reached
  as a task formal (`ref`, `inout`, `input`, `output`), a class property
  (`this.s[i]`, `c.s[i]`, `c.b.s[i]`, `C::s[i]`), a struct member, an element
  of a string array, queue or associative array (`sa[k][i]`, `s2[a][b][i]`),
  or a module string written from an `always` block. A write to a whole
  element of a string collection (`c.b.q[1] = "x"`, `s2[1][2] = "x"`) still
  replaces that element, and a task's string formal no longer makes a
  same-named packed variable elsewhere read as a string. (from PR #292 by
  Thomas Burg)
* A variable declared in a `begin…end` block of a task, function or method
  goes out of scope when the block ends: an enclosing variable, class
  property or module variable of the same name is visible again with its own
  value. This holds for blocks that wait (`#1`, `fork…join`), for blocking
  `repeat`/`for`/`while`/`foreach` bodies, for `static` block variables, and
  for array and queue block variables. A method- or block-local unpacked
  struct named like an `int` property is the local, and `s = f()` writes an
  `int` property named like a module struct. A `static` variable of a package
  or module task is one variable for every calling class. A fork child's
  write to its parent's automatic wakes the parent's `wait` before the
  time step's nonblocking assignments update, and a UVM register's second backdoor `write` no longer
  dereferences a null `map_info`. (from PR #293 by Thomas Burg)
* Unpacked-array locals of an automatic task, including multi-dimensional
  ones, are per invocation, so concurrent calls no longer share them. Tasks
  and functions in a `module automatic`, `interface automatic`, `program
  automatic` or `package automatic` are automatic unless declared `static`.
  An `output`, `inout` or `ref` array formal writes back when its actual is
  a class property (`fill(prop)`, `this.prop`, `c.prop`, `o.i.prop`, a
  `[2][2]` property), and when the callee declares a local with the actual's
  name; an integer-keyed array stays integer-keyed. Hierarchical and
  package-qualified actuals (`u.A`, `i.IA`, `pk::PA`) keep writing back. An
  `output` array formal starts at its type's default instead of a copy of
  the actual, and `foreach` over a local `int a[int]` binds negative keys.
  (from PR #288 by Thomas Burg)
* An `@(posedge clk iff g)` wait in a class task evaluates its guard in the
  waiting process: the guard reads that object's fields and the task's
  locals (`@(posedge vif.clk iff vif.gnt[master_id] === 1)`), so several
  objects parked on one clock no longer see each other's state. Inside
  `req.randomize() with {...}`, a receiver-qualified element constraint
  (`req.data[0] == d`) and a receiver-qualified size (`req.data.size() == 3`)
  apply to the randomized object; the size used to be ignored. A size that
  conflicts with the class constraints makes `randomize()` return 0. In
  `pre_randomize`, `post_randomize` and when the class has a property named
  like the receiver, `req` keeps its ordinary meaning. (from PR #287 by
  Thomas Burg)
* `randomize()` with a soft constraint and an array sized by the constraints
  no longer fails when a constraint reads a member path through `this`
  (`this.lo`, `this.lim.lo`) that names state rather than a random variable.
  (#256, from PR #257 by Taichi Ishitani)
* A member read through a class handle (`item.item.len`) no longer picks up
  an unrelated struct variable of the same name in another scope or another
  process, and a class handle sent through a mailbox from a parameterized
  class reaches the receiver while another specialization's struct receiver
  is waiting. A `ref` or `const ref` class-handle formal keeps following the
  caller's variable after the caller reassigns it. (#258, #260, from PR #259
  by Taichi Ishitani)
* A base class specialized with named parameter assignments inside an
  `extends` argument (`extends wrap #(.BASE(base #(.REQ(REQ))))`) binds its
  parameters by name; it used to bind them by position, which could end in a
  null dereference.
* A parameter declared with a type or range takes its value as an assignment
  to that type, for its default, a `#(...)` override and a `defparam`: the
  value is evaluated in the declared width, wrapped to it and given the
  declared signedness. `parameter [3:0] P = -1` is 15, `signed [3:0]` turns 15
  into -1, and `#(.P(4'hF + 4'h1))` on `[7:0]` is 16. Real values round
  (`parameter int P = 2.6` is 3), 2-state types drop x and z, and
  unpacked-array elements and pattern overrides wrap per element. A value
  parameter typed by a type parameter keeps that type's full width, and a
  range that uses another parameter of the same instance uses the instance's
  own value. (#237, from xezim-core PR #52 and PR #264 by Ganesh T S)
* A function or task with an early `return` inside a loop that cannot be
  unrolled no longer crashes the bytecode compiler ("index out of bounds")
  or leaves a caller's `break`/`continue` unpatched, which hung the
  simulation; a `for` loop whose header falls back to the interpreter no
  longer does either. (#268, from PR #267 by AaronKel)
* A static call on a class specialization that nests another one or uses
  named parameter assignments (`uvm_config_db#(cfg#(AW,DW))::get`,
  `db#(cfg#(.AW(AW),.DW(DW)))`, `db#(.T(...))`) resolves the enclosing
  class's parameters at every level, so the getter reaches the specialization
  the setter wrote. (#269, from PR #270 by AaronKel)
* `foreach` over an associative array keyed by class handles binds the loop
  variable to the key's class, so property writes through it land; an
  associative array returned through a class method's `output` formal is
  copied back; and processes parked on NBA-region completion (as in
  `uvm_wait_for_nba_region`) resume only after the time slot's queued
  processes, their `#0` continuations and their nonblocking assignments have
  run (IEEE 1800 §4.5). (#277, from PR #275 by Thomas Burg)
* `Alias::type_id::create(...)` through a chain of class typedefs, including
  package-qualified aliases and a parameterized specialization in the middle
  of the chain, reaches the class's factory registry, so factory overrides
  apply and the object is not null. (#274, from PR #276 by AaronKel)
* An unrolled `for` loop over a signed variable runs the right number of
  times when its bounds are negative (`for (int i = 1; i > -1; i--)`); it
  used to run zero times. (#278)
* A class member read or compared through a handle, through `this` or by its
  bare name in a method (`t.irq != x`) is no longer mistaken for an interface
  instance of the same name. (#271, reported by AaronKel)
* A covergroup in a parameterized class builds its bins from that
  specialization's parameters (`bins b[] = {[0:N-1]}`), so
  `get_inst_coverage()` counts samples correctly. (#273, reported by AaronKel)
* `pop_front()`/`pop_back()` on a queue of unpacked structs returns every
  member, whether it goes to a local, a class property, a return value, an
  argument or a declaration initializer, and the initializer form pops only
  once. A scalar `bit` argument of a class method has the formal's one-bit
  width. (#272, reported by AaronKel)
* `randomize()` with a soft constraint and an array sized by the
  constraints no longer returns 0 when the joint solver gives up: once its
  runs are used up, an assignment that satisfies every constraint is
  accepted. (#256, reported by Taichi Ishitani)
* The constraint solver handles more shapes: packed-struct fields set from
  other random variables, bit and part selects of random array elements,
  elements or bits at positions chosen by random variables, equalities whose
  sides wrap at their width, `$countones`, and `foreach` over a state vector.
  (#255, reported by rharikrishna25)
* Assertion control (§20.11, §20.12) is implemented: `$asserton`,
  `$assertoff`, `$assertkill`, `$assertpasson`/`$assertpassoff`,
  `$assertfailon`/`$assertfailoff`, `$assertnonvacuouson`/`$assertvacuousoff`
  and `$assertcontrol` (Lock/Unlock, assertion-type and directive-type masks,
  `levels` and a list of scopes or assertion names). They apply to
  concurrent, immediate and deferred assertions and to covers; they used to
  print "assertion control is not modeled" and be ignored, so assertions kept
  firing after `$assertoff`.
* `size()` on queues and dynamic arrays, `num()`/`size()` on associative
  arrays, `len()` on strings, `num()` on enums and mailboxes, and the array
  query functions (`$size`, `$bits`, `$unpacked_dimensions`, ...) return a
  signed `int`, so `q.size() - 2` on an empty queue is -2 and a loop such as
  `for (i = 0; i < q.size() - 1; i++)` no longer runs about 4 billion times.
  `$size` of an associative array is its number of entries, and `$bits` of a
  queue is its current size in bits. (#280)
* A `wait (...)` inside a DPI-exported task called from C blocks until its
  condition holds even when a timed event (a clock) is pending; it used to
  hang until the run ended. This covers event `.triggered`, nets and the
  task's own automatic variables, other processes' `wait`s keep resuming
  meanwhile, and a wait that can never be satisfied ends the run with the
  task still waiting. (#279, reported by Dragon-Git)
* `randomize()` solves classes with rand variables wider than 64 bits: a wide
  member that no constraint reads no longer blocks the rest of the class, and
  shifts by a constant, masks, part and bit selects, struct fields,
  `==`/`!=`, `inside` and comparisons on wide scalars, wide array elements
  and wide packed structs are solved over 64-bit segments. Arithmetic on a
  wide variable is still left to the trial loop. (#261, reported by Taichi
  Ishitani)
* `randomize()` solves `$countones` constraints whose other side is not
  linear, such as `$countones(strb[i]) == 2**size` in AXI write transactions,
  with counts following the uniform distribution over solutions (§18.5.10).
* `randomize()` no longer reports success with a violated `foreach`
  constraint over a state vector, queue, dynamic array or packed array, or
  one nested in another `foreach` (§18.6.1).
* Copying an unpacked struct with a wide multi-dimensional packed member is
  done 64 bits at a time instead of bit by bit.
* `$bits` and the array query functions (`$size`, `$dimensions`,
  `$unpacked_dimensions`, `$left`, `$right`, `$low`, `$high`, `$increment`)
  of a fixed-size unpacked-array variable fold to its declared shape wherever
  a constant is needed: localparams, parameter values, packed widths and
  generate conditions. `localparam P = $bits(fx)` on `int fx[4]` was 0 and
  is now 128.
* A `generate if`/`case` condition or a generate `for` bound may use these
  queries; `if ($size(fx) - 5 < 0)` used to be rejected as not constant. A
  generate `for` genvar that starts from a parameter, a negative number or an
  expression no longer starts at 0.
* An integral operand of a real operator is converted from its own value and
  width: with `int unsigned u = 1`, `(u - 2) * 1.0` and `r = u - 2` give
  4294967295.0, not 18446744073709551616.0 (§11.8.2).
* A `let` may be declared in a function, task, class method, `begin`/`fork`
  block, generate block, checker, program or clocking block (§11.13).
  Instances follow the standard's substitution rules, including defaults,
  typed formals and the width of the surrounding expression, and
  `import pkg::name` and `pkg::name` reach package lets. (#283, reported by
  Dragon-Git)
* Subroutine prototypes (`extern`, `pure virtual`, interface class, DPI)
  accept a typedef return type with packed dimensions, such as
  `extern function M [1:0] f();`. DPI imports may return a typedef of an
  integer, real or other small C type. (#284, reported by Dragon-Git)
* `--dpi-lib` works on non-x86 hosts: the DPI export trampoline is built with
  `$CC` (default `cc`) and no x86-only flags, and a trampoline build failure
  is named in the library-load error. (#282, reported by Dragon-Git)
* A variable whose type is a typedef of a virtual interface
  (`typedef virtual bus_if vif_t; vif_t v;`) starts as `null` at module,
  `$unit`, subroutine and block scope and in arrays, and `$typename` names
  virtual interfaces as declared (`virtual bus_if.mp`,
  `virtual pbus_if #(16)`) instead of `logic`. (#285, reported by Dragon-Git)
* `$readmemh`, `$readmemb`, `$writememh` and `$writememb` reach another
  instance's memory through a hierarchical name from tasks, functions and
  class methods, and through instance-array elements (`ra[1].mem`). A target
  that names no memory is reported as an error, and a file that cannot be
  opened as a warning, instead of being skipped silently. (#281, reported by
  jjts)
* A class that extends a parameterized class resolves the base's type
  parameters, including defaulted ones such as `RSP = REQ`, in its own
  methods: `RSP::type_id::create()` in a `uvm_driver #(item)` subclass no
  longer returns null (§8.25).
* A class property typed by a type parameter, such as a sequence's `req`, is
  no longer mistaken for a same-named struct variable elsewhere, so each
  `req = ...::type_id::create()` takes effect.
* `$size`, `$left`, `$dimensions` and the other array queries report
  per-dimension bounds for multi-dimensional packed class properties and for
  members of struct formals (§20.7).
* A dynamic-array class property initialized with `new[n]` is sized when the
  object is constructed (§8.7).
* `find*`, `min`, `max` and `unique` locator methods work on associative
  arrays with integral keys, in key order (§7.12).
* With these, the axi4 AVIP's write, read and write-read tests run to
  completion.
* Writing one character of a string that lives in a function or task frame
  (`s[i] = "x"` on a local, a static local, a `ref`/`inout`/`output`
  function formal or an automatic local in a forked child) replaces that
  byte instead of setting a bit. (from PR #289 by Thomas Burg)
* A class function that runs as bytecode (after 1000 calls) writes back every
  `output` argument, not only the last, and at the declared width of a packed
  typedef (`output M [1:0] o`). Its `static` variables keep counting across
  that switch, and a non-virtual method it calls through a base-class handle
  runs the base class's method (§8.20). A module or package function with a
  `static` variable is no longer inlined into module code, which restarted
  the variable on every call. A user class method named `put` called inside a
  function now runs. A task that reaches a wait only through a call cycle
  (`a` calls `b`, `b` calls `a`, and `a` waits) suspends its caller again.
  (from PR #265 by Thomas Burg, with follow-ups)
* `randomize() with { ... }` and `std::randomize() with { ... }` inside a
  module instance constrain the instance's own variables. A fixed array was
  left unconstrained, a dynamic array's `size()` read as 0, and a variable of
  the same name in the top module was randomized instead (§18.7). Array-method
  `with (...)` clauses and `matches` inside instances work too. (#303, #304,
  #305, reported by Ganesh-TNL)
* `std::randomize()` no longer runs out of its budget when an array `sum()`
  must land in a window around a random centre built with a cast, written as
  `inside {[lo:hi]}` or as a `>=`/`<=` pair. (#302, reported by Ganesh-TNL)
* Package functions, tasks, parameters, variables and `let`s declared in more
  than one package resolve by the §26.3 and §26.6 rules, through imports,
  re-exports and package bodies; same-named package covergroups no longer
  collide, a package `let` is visible through a wildcard import, and static
  members of same-named classes in different packages start at their own
  initializers.
* `randomize()` solves `dist` and `inside` over members wider than 64 bits.
* Unpacked arrays of class handles and class-handle struct members start
  null, 4-state static class properties without an initializer start x, the
  default of a missing associative-array key covers packed structs, and
  `$cast` to an enum fails for a source with x or z bits.
* Nets driven through overlapping bit and part selects resolve every driver
  bit by bit, so edges on those bits are counted from the resolved value
  (§6.6.1). `bufif` gates drive x for z data and L/H for an unknown enable,
  and drive strengths resolve by level: supply over strong over pull over
  weak (§28.7, §28.12).
* 2-state array elements and 2-state casts store x and z as 0, and arrays
  declared inside procedural blocks hold their declared element type.
* Conflicting variable drivers are compile errors, checked per written bit
  (§6.5, §9.2.2.2, §9.2.2.4): two continuous assigns, a continuous assign plus
  a procedural write, and an `always_comb`, `always_latch` or `always_ff`
  variable written by another process. `--relax-driver-conflicts` (or
  `XEZIM_RELAX_DRIVER_CONFLICTS=1`) reports them as warnings instead.
  Designs that write an `always_ff` variable from an `initial` block now need
  that switch.
* A vector with a `force` on one bit keeps an edge alias per bit, so the
  forced bit's edges are not shared with the other bits of a replicate.
* DPI-C: a C function writing or returning `sv_x` or `sv_z` through a
  `logic` output, inout or result reaches SV as x or z, and an import whose
  prototype uses an enum type binds and passes the enum's base type
  (§35.5.5, §35.5.6.1). (#306, reported by rharikrishna25)
* DPI-C: `integer` and `time` formals are passed as `svLogicVecVal*` (C code
  reading them crashed), unpacked struct formals are passed as C structs,
  arrays of scalar `bit` and `logic` hold one `svBit`/`svLogic` per element,
  array formals of enum, packed-struct, `integer` and `time` types bind, an
  assignment-pattern actual passes every element, named and default
  arguments on import calls bind, and `svGetScope` in a context import
  declared in a package, interface or generate block names the declaring
  scope (§35.5.3, §35.5.6, §13.5.4, Annex H).
* A typedef, enum or nested class declared in one class no longer replaces a
  same-named declaration in another class, a package or `$unit` (§8.3,
  §6.18). (#307, reported by Ganesh-TNL)
* `$bits` of a class typedef sized by a class parameter follows the object's
  parameter values. `pkg::C#(..)::static` reads that specialization's value,
  and `pkg::P` in a module instance's continuous assignment reads the
  package's parameter (§8.25, §26.3).
* `randomize()` solves constraints through sub-objects of sub-objects, on wide
  fields of `rand` unpacked struct members, on `x == f(y, z)` over wide
  values, and on `std::randomize` locals wider than 64 bits. A wide `dist`
  honours its weights (§18.4, §18.5.4, §18.12).
* `==` and `!=` on values wider than 64 bits compare every bit (§11.4.5).
* Package-scope initializers of unpacked structs and associative arrays,
  including associative arrays of structs, run once before any initial
  block, as module-scope ones do (§6.8, §7.9.11). (#309, reported by
  Ganesh-TNL)
* A struct member that is a queue, dynamic array or associative array behaves
  as a collection wherever the struct lives: a variable, a nested struct, an
  element of any array (`aa[k].q.push_back(v)`, `foreach (aa[k].q[i])`), or a
  class property. Assignment patterns replace such members, and copies, `==`,
  `%p`, subroutine formals and struct return values carry them by value
  (§7.2.1, §7.10). (#310, #311, reported by Ganesh-TNL)
* An assignment pattern written to an unpacked-struct class property assigns
  its members (§10.9.2).
* A class typedef of a type parameter (`typedef T payload_t;`) follows each
  specialization in members, locals, return types, `$bits`, casts,
  `randomize()`, derived classes and `C#(args)::name`, including packed and
  unpacked array typedefs (§8.25, §6.18). `T'(v)` casts to the bound type,
  `T [1:0]` sizes by it, and `$bits(obj.arr)` sizes the whole array member.
  `obj.m = new` for a `self_t` or `G#(T)` member, packed structs with
  type-parameter members and nested classes using the enclosing class's
  parameters follow the object's specialization too (§8.23).
* Values stored into queues and dynamic arrays (`push_back`, `push_front`,
  `insert`, indexed writes, `q = {q, v}`) are converted to the element type in
  every scope, and concatenation onto class queue members works (§7.10.2,
  §10.10).
* Compiled class methods: element selects and stores on multi-dimensional
  packed members give the right bits, collection returns and typedef-alias
  reads of static collections resolve per specialization, `Class::m()` inside
  a class or subclass calls `m` on the implicit `this` (§8.23) while the same
  call from outside a class is an error (§8.10), and typedef'd local queues
  behave as queues (from PR #308 by Thomas Burg).
* Missing collection elements read their element type's default: class and
  module queues read past the end, missing associative-array keys (including
  multidimensional class members) and members of missing struct elements
  (§7.8.6, §7.10.1). Class associative-array properties accept assignment
  patterns with keys and `default:` (§7.9.11).
* Static unpacked-struct class properties are one shared store (§8.9);
  functions returning unpacked structs start each member at its default
  (§13.4.1); packed structs mixing 2-state and 4-state members start all x
  and keep their 2-state members at 0 or 1 (§7.2.1). `%p` prints struct
  members holding fixed arrays as nested lists and null handles as `null`
  (§21.2.1.7).
* Initial blocks of instantiated programs run in the reactive region (§24.3).
* `force` of one bit of a vector driven by a part-select assign holds
  (§10.6.2). Pulls and gates on one bit of a strength-driven vector resolve by
  strength, `%v` prints the resolved strength, including ranges and L/H, and an
  undriven trireg reads z (§28.12, §21.2.1.5). An output-port connection beside
  another continuous driver of the same variable is a driver conflict (§6.5).
* DPI-C: output array formals (fixed, open, multi-dimensional, dynamic,
  and task formals) reach C as a zeroed buffer, and elements C leaves
  unwritten come back as 0 (§35.5.6.1). (test from PR #319 by Thomas Burg)
* Compiled class methods read `s[i]` of a string local as a byte, so a
  comparison such as `s[0] != "["` is right in hot methods; this also fixes
  the UVM 1.2 `print_topology()` table, which came out empty when its
  printer methods were compiled (§6.16).
* Dimensions, packed widths, port widths, parameter defaults, generate
  bounds and size casts written with a class-scoped constant (`cls::N`,
  `pkg::cls::N`, `cls#(4)::N`, or one inherited from a base class) get
  their declared size; such arrays used to elaborate with no elements, so
  `foreach` and `randomize()` did nothing. Method-local arrays sized by a
  parameter size correctly too (§8.23, §8.25.1). (#316, reported by urbanij)
* `@(cb iff cond)` judges `cond` after the clocking block samples, so it
  sees this edge's clockvars rather than the previous edge's, and a
  clocking wait reached in the same slot as its edge honours its `iff`
  (§14.13). (#317, reported by urbanij)
* A module subroutine called by its full hierarchical path from a class
  method (`tb.sub.u_h.get_path()`, also through `$root.`, generate blocks and
  instance arrays) runs in that instance, and `%m` in it names the callee; it
  used to return 0 or an empty string (§23.6, §23.8). (#320, reported by
  Ganesh-TNL)
* A parameterized class specialized with a virtual interface type
  (`cfg_db#(virtual my_if#(T))`) shares its statics with every other spelling
  of that type: through the enclosing class's parameters, a typedef, the
  `interface` keyword, named or defaulted interface parameters, or an
  `extends` clause (§25.9, §8.25). (#320, reported by Ganesh-TNL)
* Methods of a class declared inside a module resolve bare function, task and
  variable names in the instance they run for, not a same-named top-level
  declaration or the first instance's copy (§23.8). (found while fixing #320)
* `randomize()` and `std::randomize()` solve `inside`, `dist` and relations
  whose operand can wrap at its width (`c + 4000 inside {[4003:4004]}`,
  `2 * c inside {…}`, 8 to 64 bits, signed and unsigned); they used to fail
  with "no solution found". Range bounds wrap at the operand width, so
  `[-2147483648:x]` starts at INT_MIN instead of being empty (§11.4.13,
  §11.8.2).
* Static queues and other collections of structs in parameterized classes read
  back through `C#(args)::q[i].member` from outside the class (§8.25). (#321,
  reported by Ganesh-TNL)
* An element of a fixed array of queues, dynamic or associative arrays,
  declared directly or as a struct member, is a collection for assignment,
  locator results, methods, `foreach (a[i, j])` and `inside` (§7.4.5). (#322,
  reported by Ganesh-TNL)
* Struct members and package variables declared with a queue, dynamic-array or
  associative-array typedef behave like the directly declared forms, and
  `q_t x = {a, b}` initializers hold every element of `a` and `b` (§6.18,
  §7.2.1, §10.10). (#323, reported by Ganesh-TNL)
* `find with (item.field …)` on queues of packed structs, `delete(key)` on
  associative arrays of structs, and string keys on associative members of
  struct class properties work (§7.12, §7.9.2, §7.8.1).
* A class declared inside a module reads and writes the variables,
  parameters, collections, tasks, events and interface instances of the
  instance that created the object, not the top module's or the first
  instance's (§8, §23.9).
* Block-local arrays, queues, dynamic and associative arrays no longer
  overwrite a module item of the same name, locals of a module's processes are
  no longer shared between its instances, and a `for` loop over a block local
  that shadows a module variable terminates (§6.21, §9.3.4).
* Comparing an unpacked array, queue or struct with an assignment pattern
  (`a == '{9, 5, 5, 5}`, `!=`, `===`, `!==`) compares element by element,
  with `default:`, `index:` keys and nested patterns (§11.4.5, §10.9).
* A local string-queue element compared with a string in a compiled class
  method (`q[0] == name`) no longer always reads as unequal (§6.16, §7.10).
* A string character `s[i]` and `getc()` are signed bytes (`"\xff"[0]` is
  -1), and `compare()` returns signed character differences (§6.16.3,
  §6.16.6).
* Strings keep bytes above 0x7F intact in `s[i]`, `substr`, `putc`,
  character writes, `foreach`, `$fgets`, `toupper`/`tolower`, and in
  non-ASCII source text such as `"é"` (§6.16, §5.9).

* A function's value takes its declared return type (§13.4.1): a real
  returned from an integral function rounds (`int f(); return 7.6;` is 8), an
  integral value returned from a `real` function is real, and a 2-state
  return maps X/Z to 0, in every context and on every execution path,
  including class methods, type-parameter returns and functions compiled into
  `always` blocks. (#324, reported by Dragon-git)
* DPI: a real passed to an integral C type converts by value, rounding half
  away from zero, so an exported `longint` function returning `t / 1ns` hands
  C `100`, not the bits of 100.0 (§35.5.6, §6.12.2). (#324, reported by
  Dragon-git)
* `$typename` of a real-valued expression, such as the time literal `1ns`, is
  `real`, and a real in [2^63, 2^64) converts to `longint unsigned` without
  saturating (§20.6.1, §6.12.2).
* A parenthesis-free reference to a no-argument static function, static
  property or parameter through a class type parameter or a typedef bound to a
  parameterized-class specialization (`Tregistry::type_name`) runs under that
  specialization, as do `pkg::C#(..)::f` and bare static calls inside the class
  chain (§13.5.1, §8.25). UVM 2017-0.9 factory registration works again.
  (#325, reported by Dragon-git)
* `C#(args)::P`, `C#(args)::LP`, string parameters given in an `extends`
  clause, and parameters read by static methods of a class extending a
  specialization take the specialization's values instead of the defaults
  (§8.25). (#325, reported by Dragon-git)
* Class-scope typedefs reached through a type parameter or a typedef of a
  specialization (`R::elem_t`, `h_t::vec_t`) have that specialization's width
  and signedness (§8.25.1). (#325, reported by Dragon-git)
* A specialization written with an expression or a differently spelled value
  (`P#(2+2)`, `P#('h4)`, `P#(4'd4)` for an `int` parameter, `P#(LP)`) is the
  same specialization as `P#(4)` and shares its statics (§8.25).
* Two typedefs of the same unpacked array type, including a class-local
  `typedef T tq_t[$]` with `T = int`, are one type argument of a parameterized
  class (§6.22.1).
* A virtual interface held in a static class property, or in `static T` of
  `G#(virtual bus_if)`, keeps its binding: reads and `return` of it reach the
  interface (§25.9).
* `cls#(4)::N`, `cls#(.W(5))::N` and constants of a typedef of a
  specialization read that specialization's value at run time, and
  `outer::inner::N` reads a nested class's constant (§8.25, §8.23).
* `$bits` of a multi-dimensional class property, and of an unpacked struct with
  a member sized by a module parameter, counts every element (§20.6.2).
* A class declared in a module that is instantiated more than once has its
  statics per instance, and its methods write the instance's struct members and
  variables (§23.9).
* Struct class properties with queue, dynamic or associative members copy those
  members when pushed or assigned into collections, are not confused with a
  same-named module variable inside methods, and print their dynamic and string
  members with `%p` (§7.10.2, §8.4, §21.2.1.7).
* Static structs of a parameterized class can be written and read from module
  code through `C#(args)::` or a typedef of the specialization, and print with
  `%p` from a static method (§8.25).
* `%m` keeps the names of nested named blocks and named forks after the process
  waits inside them, and in fork children (§21.2.1.7).
* `@(cb.sig)` on a clocking block declared in an interface or module instance
  fires on the block's samples, including the first (§14.13).
* Program-block processes resume in the Reactive region, after the time slot's
  nonblocking updates and clocking events (§24.3).
* `assign arr = '{...}` drives each element of an unpacked array, including
  descending, `default:`, replicated, keyed and nested 2-D patterns, and a
  whole-array continuous copy between opposite directions pairs elements left
  to right (§10.9).
* Class-property arrays with a descending range (`int ds[3:1]`) keep their
  direction in assignment patterns, `%p`, `foreach` and
  `$left`/`$right`/`$increment`, and `$left(m[0])` describes the selected row
  (§7.4.2, §20.7).
* A scalar `bit` DPI formal passes only its one bit to C, and an `output` or
  `inout` one keeps only bit 0 of what C writes (§35.5.6.1). (#330, contributed
  by Thomasb81)
* A real converted to an integral type wraps to the target width instead of
  saturating, and fills targets wider than 64 bits: `longint l = 2.0**64` is 0
  (§6.12.2). (#324, #330)
* A static task that calls another static task of its class by bare name
  blocks like any task call: a `mailbox.get()`, `#`, `@`, `wait`, semaphore
  `get` or `fork…join` in the callee used to return at once. `@ev` and
  `@(C::ev)` on a static class `event` wait for the next `-> ev`, and
  `ev.triggered` reads it (§8.10, §13.3, §15.4, §15.5). (#313, reported by
  Ganesh-TNL)
* `foreach` and the collection methods work on arrays declared through
  typedefs in every scope the #312 survey covers: chained and cross-package
  typedef aliases, typedef plus declarator dimensions, typedef'd arrays in
  generate blocks and as struct members, `foreach (m[i, j])` over an
  associative array of associative arrays, typedef'd associative formals
  (`input`, `output`, `ref`) with struct elements, functions returning a
  queue, dynamic or associative array (also `f().size()`), package arrays
  used inside module tasks (`P::d = new[n]`, `P::q.max()`), and
  multi-dimensional package arrays. Two packages declaring a variable of the
  same name keep separate storage, a class property is no longer shadowed by
  an unrelated package variable of the same name, and struct members that
  are queues or associative arrays work in submodules and in a second top
  module (§6.18, §7.4.5, §7.8, §12.7.3, §13.4, §13.5, §26.3). (#312,
  reported by Ganesh-TNL)
* `$typename` names the type of integral expressions (`i+1` is `int`, `1+i` is
  `bit signed[31:0]`, `i/2` is `integer`), casts and function results, prints
  unpacked dimensions (`int$[0:1]`, `int$[]`, `int$[string]`) and struct and
  enum typedefs with their scope, and spells declared 4-state vectors as
  `reg[7:0]` (§20.6.1).
* Locals declared through a typedef of a string collection
  (`typedef string q_t[$]`) compare their elements as strings in compiled
  methods (§6.16).
* Assigning an element of a virtual-interface queue, dynamic array, fixed
  array or static array (`p = o.vq[0]`, `p = S::pa[1]`) binds the variable to
  that element's interface, and `G#(virtual pif)::one.data` reads through the
  specialization's static virtual interface (§25.10, §8.9).
* `%p` prints an unwritten member of a static struct array at its type's
  default (0 for 2-state types and for `T` bound to `int`), including
  `C#(int)::fa` and `N::fa[1]` (§6.8, §21.2.1.7).
* Variables declared in `initial` and `always` blocks and in static tasks and
  functions keep their values between activations and calls, per instance and
  per generate copy. They are initialized once before time 0, and `blk.v`
  can be read inside an instance (§6.21).
* `inside` and `case` operands are sized to the widest operand, in procedural
  code and in constraints (§11.4.13, §12.5).
* Constraints on variables wider than 64 bits solve sums and differences with
  constants, and signed wide variables take part in `std::randomize` (§18.5).
* A failed `std::randomize` leaves its variables unchanged (§18.6.1).
* An omitted argument binds its formal's default as an aggregate: a fixed
  array, dynamic array, queue or associative array default no longer turns
  into one packed value. `output` and `inout` formals may have a default
  (1800-2023 §13.5.3), and the formal is copied back to it. An earlier call
  with an array argument no longer changes how a later call behaves.
  (#327, reported by Dragon-Git)
* An associative array of associative arrays under a fixed, dynamic or queue
  dimension (`int v[2][string][string]`) works in modules, classes, automatic
  locals and struct members, including `exists`, `size`, `num` and `foreach`,
  and automatic locals that are fixed arrays of queues or associative arrays
  work (§7.4.5, §7.8.1). (#328, reported by Dragon-Git)
* A whole associative array assigned to or from a class property, or pushed,
  inserted, concatenated or popped as a queue element, keeps its entries
  (§7.9.9, §7.10). (#331, reported by Dragon-Git)
* `$rtoi` of a real outside the 64-bit range returns 0, and a real assigned to
  a wide integral variable or an integral parameter wraps to the target width
  (§20.5, §6.12.2).
* `vpi_user.h` declares the `s_vpi_vecval` words `PLI_UINT32`, as the
  standard's header does (§38.15).
* Imported DPI tasks run on their own stack on x86_64 and aarch64, for Linux
  (glibc and musl) and macOS, so a `wait` in an exported task behaves the
  same everywhere. `XEZIM_DPI_FIBERS=0` selects the nested-scheduler
  fallback, which no longer loses concurrent callers or ignores a `$finish`.
  (#290, reported by Dragon-Git)
* Exported subroutines accept every formal type DPI can carry (§35.5.6,
  Annex H): `chandle` as `void*`, a `string` result or output, packed vectors
  wider than 64 bits through `svLogicVecVal*`/`svBitVecVal*`, the exact
  scalar C types, and unpacked structs and arrays by pointer. A formal DPI
  cannot carry is reported as an error at each call instead of silently
  returning 0, and a wide or aggregate result is rejected (§35.5.5). (#291,
  reported by Dragon-Git)
* `randomize()` solves rand sub-objects held in fixed, dynamic and queue
  arrays of handles, rand sub-object members wider than 64 bits (packed
  structs, nested fields, packed-array elements) and `solve ... before` on
  sub-object members together with the enclosing object (§18.5.9). Reading a
  nested packed-struct field or packed-array element through a class handle
  no longer returns 0. (#255, reported by rharikrishna25)
* `randomize()` solves `sum()` constraints over elements of 16 bits or more,
  where the sum wraps at the element width as §7.12.3 requires. A reduction
  with a `with` clause has the clause expression's width and signedness, and
  `size() inside {…}` picks a size in the set. (#295, reported by Ganesh-TNL)
* Packed-struct arrays local to a task or function keep their field layout,
  so `a[i].f` reads and writes work and `foreach` field constraints solve.
  (#296, reported by Ganesh-TNL)
* `std::randomize()` solves dynamic arrays and queues local to automatic
  tasks jointly with the other variables, and `bit` array elements are
  solved unsigned. (#297, reported by Ganesh-TNL)
* `randomize()` and `std::randomize()` solve the size of a dynamic array
  before its elements when a constraint ties the size to other variables
  (`beats.size() == cmd.count + 1`, `a.size() + b.size() == t`) (§18.5.8.1);
  they used to draw a size and retry until the time budget ran out.
  Packed-struct elements whose type is a class type parameter are solved,
  `(a >> k) == (b >> k)` and alignment masks on struct fields no longer stall
  the solver, and a variable that caps another through a guarded `dist` is
  drawn by the solutions it leaves. (#298, reported by Ganesh-TNL)
* `std::randomize()` over several dynamic arrays with `foreach` constraints
  succeeds, also when a size comes from another target (`p.size() == n`),
  which used to allocate an array as large as a random `n` and hang. (#299,
  reported by Ganesh-TNL)
* `randomize()` solves `foreach` constraints over multi-dimensional packed
  members wider than 64 bits of several rand sub-objects
  (`foreach (sel[i]) b.W == a.W;` with `bit [8:0][7:0] W`), and element and
  part selects of such members read the right bits (§7.4.1, §11.5.1). A
  `randomize()` that fails now prints a warning naming the class and its
  constraint blocks (first 3 failures per class; `XEZIM_RAND_WARN=0`
  silences it). (#301, reported by Ganesh-TNL)
* `a.size() == n + m` compares at 32 bits, since `size()` and `num()` return
  an `int`; narrower operands used to wrap. A field write to an element of a
  class array of packed structs (`beats[i].last = 1`) is no longer dropped.

**Performance**

* Measured against 0.11.1 on the README workloads (cold cache, plain release
  build): C906 CoreMark ×1 runs 8.5% fewer host instructions (376.8 G against
  412.0 G, 79 s); the C910 memcpy ×200 runs about the same (472.7 G against
  474.0 G instructions, 107 s whole run, about 11 s of it compiling); the
  AXI4 AVIP runs about 7% more cycles (median of five runs, 6.7 s), the cost
  of this release's correctness fixes.
* More class methods and package functions run as bytecode: those that read
  a member off an element of a class-handle collection (`m[k].val`,
  `q[i].name`), package functions that call a static method of a class
  (`Srv::get()`), and bodies that store a returned queue or dynamic array in
  a local (`q = f(..)`). Element member reads run 62% fewer instructions
  and a package function calling a singleton getter 15% fewer. (from PR
  #294 by Thomas Burg, with follow-ups)
* Wait-free class tasks and package and module functions run as bytecode once
  they cross the same call threshold as class functions (`XEZIM_METHOD_TIER`,
  default 1000 calls), and compiled code calls compiled package functions
  directly. A class-heavy benchmark (400,000 calls of a wait-free task that
  makes a virtual call and two package-function calls) runs 16% fewer
  instructions; the axi4 AVIP runs 0.4% more. (from PR #265 by Thomas Burg)
* Edges on bits of a vector driven by one replicate assign
  (`assign clk_v[7:0] = {8{gclk}}`) share one internal edge alias instead of
  one per bit, so the flops they clock wake once per edge. The c906
  memcpy benchmark runs 7.8% fewer instructions (4.1% fewer cycles); edge
  timing is unchanged.
* With the `jit` feature and `XEZIM_TS_JIT=1`, a native two-state stream
  that reads an x re-runs on the two-state executor's x path instead of the
  four-state interpreter (20x fewer such re-runs on c906).

**Usability**

* `[PHASE]` times from a precompiled model file use the wall clock, like a
  run from source, so the two report comparable phase totals.
* `XEZIM_INIT_REG=random` now varies with the run's seed (`+seed=<n>`,
  `-sv_seed <n>`), so a reset bug that depends on the power-up value can be
  found by changing the seed; the same seed reproduces the same values, and
  no seed (or seed 1) gives the pattern earlier releases used.

* `-fst_scope_file <file>` reads FST dump scopes from a file (one or more per
  line, `#`/`//` comments), and `-xezim_env <file>` sets `XEZIM_*` variables
  from a file (`NAME=value`, `export`, `setenv`, `unsetenv` lines) before xezim
  reads any of them, so a run's settings can live next to its file list
  whatever shell starts it.

* `-uvm` (or `--uvm`) adds the UVM library that `XEZIM_UVM_DIR` names, for
  compile and simulation alike: its `src` directory joins the include path and
  its `uvm_pkg.sv` the file list, so a UVM testbench needs only its own files.
  `XEZIM_UVM_DIR` may name the `src` directory, a release root, or a checkout
  of several releases, where `XEZIM_UVM_VERSION` picks one (default: the
  newest). Without `-uvm` nothing is added.

* `-override_timescale <unit>/<precision>` (also `--override-timescale`)
  gives every design element, package and compilation unit one timescale,
  replacing every `` `timescale `` directive, `timeunit`/`timeprecision`
  declaration and `--module-timescale`: with `1ns/1ns` a bare `#10` is 10 ns
  everywhere, with `1ps/1ps` 10 ps. Literals with a unit (`#3ns`) keep their
  absolute value. It works on the command line and in `-f`/`-F` files.

**Waveforms**

* An FST scope can carry a depth, `LEVEL:SCOPE`, both in `--fst-scope` and in
  a `-fst_scope_file`: `1:top.u_cpu` dumps only that scope's own signals,
  `2:top.u_cpu` one level of children too, and no prefix (or `0:`) every
  level, as `$dumpvars` does (§21.7.1.4).

* Ctrl-C and SIGTERM also stop a long loop inside one time slot, and close
  the waveform dumps normally. When the run cannot stop (a DPI call that does
  not return), the FST dump is closed at the current time before the process
  ends. FST dumps reach the disk at least every 2 seconds
  (`XEZIM_FST_FLUSH_SECS`), so a run killed with `kill -9` or by running out
  of memory leaves a readable file up to its last write.

* An FST dump killed while a value-change block was being written (`kill -9`,
  an out-of-memory kill, a second Ctrl-C) is readable again. The block went
  to the file in 8 KB pieces with its length patched last, so a kill in that
  window left a block that made every reader reject the whole file (about 1
  kill in 15 under load). Each block now reaches the file whole or not at
  all, and the first block is written as soon as the run starts, so the file
  always opens and holds everything up to the last block written.

### 0.11.1 — complete VPI, faster memory models, IEEE 1800 conformance fixes (October 2026)

**Correctness**

* A loop variable declared in `for (int i = ...)` or `foreach (arr[k])` no
  longer binds to a same-named variable in a child instance: the loop runs on
  its own storage. It used to read the child's x and run zero iterations, or
  step the child's counter. A bare name also no longer follows an earlier
  hierarchical reference such as `u.x = 1` down into that child. (#195, from
  PR #196 by Ganesh T S)
* A member access through a struct no longer lands on an unrelated class
  object whose handle number happens to equal the struct's value: the path is
  chosen from the declared type. This covers struct variables, collections of
  structs, hierarchical references, `ref` formals and part-selects, a
  module-level struct holding a class handle, and class handles declared
  through a typedef (including a parameterized one). (#193, from PR #194 by
  Ganesh T S)
* Members of a struct-typed class property resolve at any depth: indexed and
  part-selected members, nested structs, and queue, dynamic and associative
  array members, in methods and at module scope. (#197, from PR #198 by
  Ganesh T S)
* A class that extends a parameterized class without giving all its
  parameters (`class d extends base;`) gets their declared defaults, in
  functions and tasks alike. A type parameter used to read as a 1-bit
  `logic`, so a UVM base test built this way created null objects and drove
  nothing. (PR #199 by eenky)
* `+incdir+` directories in a `-F` args file resolve against the file's own
  directory first, so `+incdir+.` means that directory. (PR #200 by Francesco
  Urbani)
* `--dpi-lib` libraries are loaded into the global symbol scope, as the DPI
  guide says, so one library can call another, in either command-line order.
  A call into a second library used to end the run with `symbol lookup
  error`. (#202)
* A `--dpi-lib` library that cannot be loaded stops the run before it starts
  (exit 1), and calling a DPI import that no loaded library defines is a
  `Fatal`. Both used to continue, with the import returning 0, and exit 0.
  (#201)
* `$display` output from a testbench that prints rarely appears as it is
  printed, not only at exit. (xezim-core #49)
* A task enabled without parentheses through a hierarchical path or a
  package scope (`u.t;`, `a.b.t;`, `pkg::t;`) is called. It used to be
  dropped inside a task, so an `always` calling that task spun at time 0
  and the run hung, and directly in an `always` it was rejected as having no
  timing control. A three-level enable inside a task (`a.b.t();`) also kept
  advancing time after `$finish`.
* An exported task called from C returns only when it has finished:
  `fork ... join`, `join_any`, `wait fork`, `wait(...)` and `@(...)` inside it
  now wait, with the rest of the simulation running meanwhile. `fork ... join`
  used to drop the rest of the task and return, and the others returned at
  once. (#204)
* `wait(ev)` on a named event waits for a new trigger every time. After the
  event's first trigger it used to fall straight through.
* VPI: `vpi_mcd_open`, `vpi_mcd_close`, `vpi_mcd_flush`, `vpi_mcd_name`,
  `vpi_mcd_vprintf`, `vpi_flush`, `vpi_compare_objects` and `vpi_get64` are
  available, and `vpi_mcd_printf` writes to files opened with `vpi_mcd_open`
  or `$fopen` instead of only stdout. The DPI guide lists the VPI calls that
  are still missing. (#205)
* `vpi_get(vpiTimeUnit, ...)` and `vpi_get(vpiTimePrecision, ...)` return a
  module's own timescale for a module handle (the two names are now defined in
  `vpi_user.h`), and `svGetTime`, `svGetTimeUnit` and `svGetTimePrecision`
  give the same answers from DPI code. (#206)
* Gate primitives, `#(rise, fall)` gates, UDP instances with parameter
  delays and `wire #d w = expr;` inside sub-module instances keep their
  delays, resolved per instance in the child's timeunit; a sub-module `buf`
  turns z into x as at top level.
* `always @(m[i])` fires in designs that also declare an array of more than
  100,000 elements; every array allocated after the large one used to lose
  element sensitivity.
* Ports with no data type, and ANSI `input logic` / `inout logic` ports, are
  nets (§23.2.2.3); a hierarchical `assign dut.u.clk = ...` onto a net-typed
  port chain drives it. A continuous assign that only reads an unpacked-array
  element no longer creates a phantom 1-bit net named after the array.
* The one-time notes for ignored system tasks (such as `$dumpfile` without
  `--wave`) are written to stderr as one piece, so `$display` output going to
  the same file can no longer land inside one and hide a UVM message from log
  parsers.
* Continuous assigns to a bit or part of an unpacked-array element
  (`assign r[i][j] = ...`, `.out(r[i][j])`) drive the array inside
  instantiated modules. They used to be dropped silently there, which left
  the c906 interrupt controller's gated clocks dead. Writes to a bit of a
  2-D or deeper array element (`m[i][j][k] = ...`) are no longer dropped
  either, at the top level or in procedural code.
* A net delay (`wire #2 w;`, `#(rise, fall, off)`, `#(P)`) delays every
  driver of the net: separate assigns, ports, gates and UDPs. It acts after
  the drivers' own delays and after resolution, bit by bit on a vector. A
  declaration assignment's delay stays that assignment's own. A port bound to
  a net takes the external net's delay. Each of several drivers on one net
  keeps its own delay, and driven `tri0`/`tri1` nets start at x.
* An undriven output port with no data type (`output o`, `output [3:0] o`,
  non-ANSI `output o;`) floats at z. It used to start at x.
* `@(mem[5])`, `@(mem[i])` and `@(posedge mem[i][0])` on a large memory wake,
  in `always` blocks and in procedural waits alike, and continuous assigns and
  `always_comb` blocks reading an element follow writes to it. The event was
  dropped. An event on a select (`v[3]`, `v[5:4]`, `m[i]`) is an event on the
  selected value: edges are judged on the selected bit, and a changed index
  re-selects the element. `posedge v[P-1]` with a parameter index watches the
  right bit.
* A class parameter default is evaluated in the class's own scope, so
  `int D = W*2` and `type U = T` follow the parameters before them in every
  specialization, including typedefs, extends clauses, parameterized
  subclasses and static calls. Specializations that differ only in defaulted
  parameters share statics. Named class parameter assignment (`C #(.W(5))`)
  binds by name, `$typename` lists every parameter, and a class variable
  declared in an instantiated module keeps its specialization.
* A virtual interface reached through any receiver works: class-handle chains
  (`cfg.vif`, `a.cfg.vif`, `this.cfg.vif`), local and formal handles,
  module-scope chains, inherited, parameterized, static and typedef'd
  properties, and vif arrays and queues. This holds for reads, writes,
  nonblocking writes, `@(...)`, `wait(...)`, clocking blocks, interface task
  and function calls, and binding. They used to read 0 or x, drop writes and
  never wake. `@(vif.cb)` in a class method resumes after the clocking block
  samples. (From PR #207 by eenky)
* A call in receiver position (`get_obj().x`, `q.pop_front().addr`) runs
  once. It used to run up to 12 times, and `q.pop_front().addr` drained the
  queue.
* Unpacked-struct members with two or more unpacked dimensions
  (`bit [7:0] m [2][2]`) survive whole-struct copies (module, block-local,
  nested, array, queue and associative-array elements, class properties),
  struct formals of every direction, function returns, assignment patterns and
  `%p`. Only the first dimension used to be seen, so the member read as x
  or 0. A nonblocking whole-struct assignment `b <= a` updates every member;
  it used to get even scalar members wrong. Struct input ports and struct
  continuous assigns carry array members, patterns and `%p` follow a
  descending dimension's declared order, 2-state array members default to 0,
  `num()` on an associative array of structs counts keys, and
  `$size`/`$bits` work on a member sub-array such as `s.mm[1]`.
* A UVM DPI library built from `include/uvm_dpi_xezim.cc` loads. The UVM
  sources it compiles call the SV export `m__uvm_report_dpi`, and built as
  C++ that call was mangled, so `--dpi-lib` stopped with `undefined symbol`.
  The driver now gives everything C linkage, as the reference `uvm_dpi.cc`
  does, and also builds as C. (#208)
* An input port whose connection computes something (`.p(a & b)`) is a net
  of its own (§23.3.3), so `@(p)` no longer wakes when an operand changes
  while `a & b` stays the same. A child reached through a select
  (`.wl(wl[1])`) whose own port has the same name latches again.
* Inlined task bodies no longer see the caller's local variables, an output
  argument is copied back into the caller's own variable, `disable` of a task
  inside `foreach` ends the task, and integer values written to `real` locals
  and formals convert.
* `/` and `%` size both operands to the wider one (§11.6.1); a narrow dividend
  used to narrow a ternary divisor and read x.
* A two-state variable no longer stores x or z when a value with x or z bits
  is written to it from compiled code.
* A member write through a class property whose type is a type parameter
  (`class p #(type CFG = cfg_c); CFG a; ... a.x = 1;`) reaches the object.
  It used to be dropped without a diagnostic, along with compound,
  nonblocking and nested writes through such a property. (PR #209 by eenky)
* A module path or SDF delay that rounds to zero ticks is no delay: the
  change it carries happens in the same Active region as its cause. A clock
  passed through a zero-delay library cell (`(posedge A => (Y:1'b1)) = (0.01,
  100.0)` at 1ps precision) clocked its flops after that time step's
  nonblocking assignments had committed, so they sampled the new values.
* A class whose base class is a type parameter
  (`class wrap_c #(type BASE = base_c) extends BASE`) gets that base in every
  specialization, including one that passes its parameter on
  (`extends wrap_c #(B)`). It used to get no base at all: `super.new()` did not
  run the base constructor, inherited properties read x, inherited methods and
  `super.` calls returned nothing, and `$cast` upcasts failed. UVM's
  `uvm_port_base` and `uvm_reg_sequence` extend a type parameter, so a `$cast`
  from a TLM port to its interface base failed and register sequences never ran
  their `body`. (#210)
* A static property read or written through an object handle (`h.count`)
  reaches the static of the handle's declared class and specialization, also
  through a null handle. It used to read x unless the static had been used
  through the class name first, and writes through a specialized handle were
  lost.
* `$cast` to a variable declared with a class specialization
  (`pbase #(8, byte) x;`) checks the parameters wherever the variable is
  declared: module scope, class property, local, formal or typedef. Outside
  procedural locals it used to accept an object of any specialization.
* An interface class that extends a type parameter is an error in every mode,
  as IEEE 1800 requires, including as a second base (`extends ic_a, B`).
* Class properties take the width of the object's specialization:
  `bit [W-1:0] pv = '1` in `pbase #(8)` is `8'hff`, not the default width's
  fill, and the same holds for member arrays, queues, statics and class-local
  typedefs. A type parameter bound by an ancestor's `extends comp_base #(byte)`
  gives inherited properties, locals and function returns that type's width
  and signedness. A ranged type argument (`#(bit [5:0])`) in a typedef keeps
  its range.
* Variables declared inside functions, tasks, class methods, `begin`/`fork`
  blocks, loops and `always` blocks keep their full declared type (IEEE 1800
  §6.8). A 2-state local no longer holds x or z, an initializer is converted to
  the declared type (`byte unsigned v = 200` used to read -56, `int unsigned
  v = -1` compared as negative), and struct literals, strings, virtual
  interfaces and tagged unions initialize correctly. Packed multi-dimensional
  array locals keep their dimensions.
* 2-state members of an unpacked struct (`int`, `byte`, ...) start at 0
  wherever the struct lives; they used to read x until first written. Member
  defaults (`int a = 3;`) apply to block-local and class-property structs too.
* Bit-stream casts to and from unpacked arrays, dynamic arrays and queues
  (`barr_t'(32'h01020304)`, `int'(barr)`) unpack and pack the elements in
  declared order (§6.24.3); they used to give zeros.
* A base-class handle to a derived object reads and writes the base's copy
  of a property the derived class redeclares (§8.14). Writing through the
  base handle used to change the derived copy.
* A class fixed array sized by a value parameter (`int items[N]`) has each
  specialization's own size, and a static method called through a handle to a
  specialized class sees that specialization's parameters (§8.25).
* Elements of class `byte`, `shortint` and signed-vector queues and dynamic
  arrays keep their sign (`-1` used to read as `255`).
* Writes through a chain of handles of any depth (`n.next.next = new(3)`,
  `a.b.c.x = 5`) are no longer lost (§8.15).
* A class property no longer resolves to a same-named member of an unrelated
  struct variable elsewhere in the design, which used to leave handles null
  and their members x. `%s` of a string read through two handles prints the
  text without padding.
* `q.min() == q.max()` on a class queue property no longer crashes.
* Same-named variables in different packages, or in a package and a module
  that imports it, are separate variables (§26.3). They used to share storage.
* Queue, dynamic-array and associative-array arguments follow §13.5: an
  `input` is a copy (it used to alias the caller's variable), an `output`
  starts empty and is copied back on return, and a `ref` aliases the actual,
  so concurrently forked tasks see each other's writes.
* `@(posedge x)` and other event controls on a `ref` argument wait for the
  actual to change; they used to return at once.
* A queue slice passed directly as an argument (`f(q[1:$])`) passes exactly
  the slice; it used to pass one element, and a recursive function over
  slices overflowed the stack. `q[a:b]` with `a > b` is empty.
* Clocking blocks (§14): `#1step`, `#0` and `#N` input skews sample the
  Preponed, Observed and time-minus-N values (inputs used to return the value
  after the clock edge), output skews including `#1step` are honored, `inout`
  clockvars can be driven, and an `@(cb)` reached right after the raw clock
  edge sees that edge's samples. `#1step` used to be read as `#1`, which also
  dropped the rest of a `default` clocking item.
* `disable` of a named block or task from a process forked inside it ends the
  whole block, kills the processes forked in it and resumes the block's owner
  after it (§9.6.2). The sibling branches used to keep running.
* `force` and `release` on bit- and part-selects of nets hold against the
  net's drivers and hand back only the released bits (§10.6.2). Forces also
  hold on gate-driven nets.
* A level-sensitive `always` block in an instance runs at time 0 when an
  input port's actual has a declaration-initialized value; its outputs used to
  stay x.

**Performance**

* Combinational logic re-evaluates only when a bit it actually reads
  changes (`assign lo = bus[3:0]` stays idle while `bus[7:4]` moves;
  `XEZIM_BIT_SENS=0` restores whole-signal sensitivity), and the two-state
  executors and the settle loop do less bookkeeping.
* Class method calls cache a per-method call plan and save the formals'
  type metadata only when it is rewritten; instantiation reuses a per-class
  template.
* Startup is much cheaper: the design is preprocessed once per run instead
  of two or three times, elaboration and the bytecode compiler allocate far
  less, and a warm `--cache` run loads the stored design about twice as
  fast. `XEZIM_EXIT_AFTER_COMPILE=1` stops right after compile, for measuring
  startup.
* Together, host instructions fall 7% on a C906 CoreMark run and on the
  AXI4 AVIP and 11% on a C910 memcpy run, with identical output; a UVM
  testbench reaches time 0 with about a third of the previous work.
* Designs built from many small instances, such as a cell-level memory:
  input ports that nothing can observe by name are left out
  (`XEZIM_KEEP_PORTS=1` keeps them; waveform dumps, VPI, DPI and hierarchical
  references keep them automatically), AND/OR gates whose other input holds
  the controlling value skip clock-driven evaluation (`XEZIM_CTL_MASK=0` turns
  it off), and `acc = acc | x[k]` reduction loops compile straight-line. A
  64k-cell DRAM model simulates with 1.0 G instead of 11.7 G host
  instructions; a C906 CoreMark run uses 4% fewer.
* Behavioural memory models: a task call from compiled code no longer copies
  the task, and a statement that still needs the interpreter runs inside its
  compiled block instead of sending the whole block to the interpreter. A
  DDR4-style behavioural model runs 4 to 6 times faster.
* Large flat designs start faster: elaboration skips passes that cannot apply,
  and dependency ordering joins nets with very many writers and readers
  (`XEZIM_TOPO_JOIN=0` turns that off). The 64k-cell DRAM model elaborates and
  compiles in about 3.5 s instead of 8.2 s.

**Memory**

* Large memories live in a packed arena by default (integral elements up to
  64 bits in arrays of more than 100,000 cells; `XEZIM_PACKED_MEM=0` turns it
  off), and arrays of 257 or more elements no longer store a name per
  element (`XEZIM_VIRTUAL_NAME_MIN_CELLS` sets the cut-off). `force`/`release`,
  `$readmemh`/`$writememh`, DPI array arguments and `uvm_hdl_force` work on
  arena memories. A C906 CoreMark run peaks at 0.6 GB instead of 2.6 GB.
  `XEZIM_RSS_TRACE=1` prints resident and peak memory at every phase.
* Instantiated `always` blocks stay in their shared form until they compile
  (`XEZIM_LAZY_ALWAYS=0` turns it off), compiled blocks drop their statement
  trees when nothing needs them, and freed memory is returned after
  compilation. The 64k-cell DRAM model peaks at 0.58 GB instead of 1.6 GB, and
  a 256k-cell one at 2.1 GB instead of 7 GB.

**VPI**

* Every routine of IEEE 1800-2017 clause 38 is implemented: user data per
  call instance, `vpi_get_systf_info` with `vpiUserSystf` handles,
  `vpi_handle_by_multi_index`, `vpi_handle_multi(vpiInterModPath)`,
  `vpi_get_value_array`/`vpi_put_value_array`, `vpi_get_delays`/
  `vpi_put_delays` (nets, module paths, timing checks, intermodule paths) and
  `vpi_get_data`/`vpi_put_data` (always 0: xezim has no `$save`/`$restart`).
  `vpiStrengthVal`, the short/long integer, shortreal and raw value formats
  and `vpiTimeVal` puts work; `vpiObjTypeVal` follows §38.15.
* `vpi_register_cb` accepts every §38.36 reason. Simulation-time callbacks
  run in their region order within each time step; `cbValueChange` reports
  every change, including continuous assignments (cocotb edge triggers on
  such nets no longer hang); force, release, assign, deassign, disable,
  statement, error, timing-violation, signal and unresolved-systf callbacks
  are new; `vpi_get_cb_info` and `vpi_remove_cb` work for all of them.
  `cbReadOnlySynch` fires once at the end of a time step.
* A VPI application can walk the whole design (IEEE 1800-2017 chapter 37):
  instances of every kind, packages, generate scopes, named blocks, tasks and
  functions, processes, continuous assignments, gate/switch/UDP primitives,
  specify paths and timing checks, modports, `bind` instances and ports, each
  with `vpiFile`/`vpiLineNo`; `vpiType` is the declared type through
  typedefs, with ranges and typespecs. Ports iterate in port-list order and
  SystemVerilog unpacked arrays report `vpiRegArray`.

**Assertions**

* A failing assertion with no `else` clause (immediate, deferred or
  concurrent, `assume` included) reports `** Error: Assertion error.` with its
  time and scope, as §16.3 requires. It used to print nothing.
* Property and sequence local variables (§16.10) are supported: declarations
  with initializers, match-item assignments `(seq, v = e)` and `local input`
  formals, with a separate copy per attempt. Subroutine calls in match items
  (§16.11) run at each match.
* A concurrent assertion inside an `always` procedure takes its clock from the
  procedure's event control and starts only when the procedure reaches it, so
  an enclosing `if`, `case` or loop gates it (§16.14.6).
* `default disable iff` (§16.15) applies to the assertions of its scope; an
  explicit `disable iff` still wins.
* An `always_comb` or `always @*` whose only reads are inside an assertion
  re-runs when those values change; it used to run once at time 0.
* `a ##1 b == c` delays the comparison, as the operator precedence requires.
  Each copy of an assertion in a generate loop runs (only the first used to),
  and `%m` in its action names the generate block. A named property without a
  clock uses the default clocking.

**DPI**

* Imported DPI tasks that wait through exported tasks suspend and resume
  independently (§35.5.2, Linux with glibc), so concurrent callers no longer
  return in last-in first-out order. Each active call reserves its own stack,
  256 MiB by default; `XEZIM_DPI_STACK_MB` changes the size.
* The §35.9 disable protocol is implemented: `svIsDisabledState()` reports a
  call whose process was disabled, a disabled exported task returns 1, and
  protocol violations are fatal. The C side of imported and exported tasks
  returns `int`.
* `export "DPI-C"` declared in an instantiated module works. Each instance's
  copy is called according to the current DPI scope (`svSetScope`, or the
  calling context import).
* Packed struct and union arguments pass as `svBitVecVal` or
  `svLogicVecVal` in every direction; they used to arrive and return as 0. Multi-dimensional
  open arrays are supported, and output and inout arguments of exported
  functions and tasks are written back to C.

**Tests**

* A new `lrm` test group checks 409 subclauses of IEEE 1800-2023, chapters 3
  to 38 and Annexes D and E, against values taken from the reference
  simulator (169 tests). The test suite has no ignored tests.

**Removed**

* The experimental multikernel / PDES code: the `--multikernel-scope`,
  `--pdes-c910-stub` and `--pdes-c910-ticks` options, the
  `XEZIM_DISPATCHER=pdes`/`perlp` modes and the `XEZIM_PDES_*` /
  `XEZIM_PERLP_*` variables. The library function `simulate_multi` no longer
  takes a `multikernel_scope` argument. Partition-based parallel dispatch
  (`--load-partition`, `XEZIM_PARTITION_*`) is unchanged.

### 0.11.0 — code coverage, reference-parity fixes, faster UVM (September 2026)

**Correctness**

* UVM: a blocking task called on a function result
  (`pool.get("x").wait_trigger()`) suspends the caller;
  `iface_array[i].handle = new` constructs the object; `$typename` of class
  handles, `this` and class typedefs prints `class [pkg::]name #(params)`;
  `std::randomize(obj.member) with {...}` works; queue and dynamic-array
  locals of class methods are per call; 2-state class properties and
  struct members drop X/Z on assignment (UVM register reads of a status
  field returned X); inline constraints tying rand sub-objects together
  are solved jointly; parked `wait(cond)` statements re-check only when a
  name they read changes; SIGTERM and Ctrl-C end a run stuck in a
  constraint solve within 5 s.
* The `--features jit` build compiles again.
* Functional coverage: covergroups work in instantiated modules,
  interfaces, packages, file scope, multi-top designs and classes declared
  in modules (each module instance gets its own covergroup, named `u0.cg`
  in `xezim_cov.json`). Cross bodies support `binsof`, `intersect`, `!`,
  `&&`, `||`, `with` and cross `ignore_bins`/`illegal_bins`; `bins x[N]`,
  transition sets, lists and repetitions work; `option.at_least` applies to
  automatic and cross bins; `@(clk iff en)` and named-event sampling work;
  `cg_type::get_coverage()`, `(covered, total)`, `start()`/`stop()` and
  `cg.option.<name>` reads are supported; `illegal_bins` hits are errors
  (counted by `--error-exit`); unlabeled coverpoints are named after their
  variable; `cover sequence` counts every match; and assertion entries in
  `xezim_cov.json` carry `file` and `line`. Coverage numbers match the
  reference simulator. See docs/coverage-guide.md.
* UVM examples that hung now finish: a clock generator on an interface
  member (`BUS.clk`) toggles its own signal; a non-root module drives
  members of its own interface array (`assign INT[0].irq = ...`); class
  handles in child instances start as null; `semaphore s = new(N)` as a
  class member gets N keys; `new` through a typedef of a parameterized
  class builds that specialization (UVM transaction event pools);
  a redeclared base-class property keeps its own initializer; fixed-size
  arrays declared in class methods are separate per call; a class
  property takes precedence over a same-named module signal; and
  `randomize() with {}` constraints on members of nested rand objects
  (register-model fields) are solved.
* sv-tests: 98.9% pass (from 97.6%). Strict checks reject malformed UDP
  table rows, duplicate non-ANSI subroutine port declarations, constructor
  body port declarations, end labels on unnamed blocks, `#([7:0] A)` without
  `parameter`, `for (var [7:0] i ...)`, `void'` of a non-call, mixed
  positional and named connections, inout port defaults, generate loops
  without a genvar, self and generate-local `defparam` targets, and more;
  non-ANSI port expressions (`.b(a[2:1])`, `{a, b}`) are supported, and a
  ranged non-ANSI subroutine port redeclared as a `reg` is one port.
* `obj.randomize() with {...}` binds names in the object first and then in
  the calling scope (§18.7): caller locals, inherited members, array and
  queue elements, handle members and `local::` names now work, and a
  caller local no longer hides an object member. A receiver-less
  `randomize() with {...}` in a class method applies its inline block.
* Randomization semantics follow the reference simulator: `dist` weights
  hold in coupled constraint sets (`:=` weighs every declared value, `:/`
  the whole item), `solve a before b` picks `a` uniformly, `randomize(a,
  b)` makes exactly the named members random (even non-`rand` ones), and
  an `inside {[hi:lo]}` range with the bounds reversed is empty.
* Processes waiting on a net driven by a continuous assignment (`assign
  CLK2 = PCLK;`) resume after those waiting on its source, and waiters on
  an interface's clock port after both, as in the reference simulator.
  Testbenches that race on this order (the uart UVM example's modem tests)
  now agree with it.
* `%m` and the "Scope:" line of `$info`/`$warning`/`$error` name the
  declaring scope (`pk.f`, `pk.C.show`, `tb.u.E.show`); generate blocks,
  named `always` blocks and labelled assertions are scopes. Auto-detected
  tops run in source order. A class and an interface may share a name,
  a blocking `randcase` branch suspends its process, and package-scope
  initializers may call `$sformatf`.
* Absolute hierarchical writes (`tb.u.sig = v`) inside tasks, functions
  and class methods take effect; a callee's local declaration no longer
  changes a same-named variable's type in the caller (a signed `int` read
  back through `inout` printed unsigned); arithmetic wider than 128 bits
  keeps every bit; a forked package task runs as its own process;
  `s.h = new` on a struct's class-handle member constructs it; `defparam`
  paths resolve upward; a bare name in a class method must be visible from
  that class (UVM 1.2's removed global `factory` is rejected, as the
  reference simulator does).
* sv-tests: 97.7% pass (from 94.8%). New legality checks reject what the
  LRM forbids and the reference simulator rejects: enum, unpacked-array
  and class-handle assignment compatibility, subroutine argument binding,
  constant expressions in ranges/parameters/part-selects, automatic
  variables in NBAs and procedural assigns, enum base types, packed struct
  members, net output ports, replication counts and casting sizes, gate
  terminal counts, package exports and scoped names, and parameter
  overrides. Parser: min:typ:max delays, `and #6 (q, a, b)`, net
  declaration delays after the data type, nameless UDP instances,
  `this.super.x`, and `begin_keywords "1364-2001-noconfig"`.
* UVM 1800.2 (2020.3) DPI: `uvm_re_comp`/`uvm_re_exec`/`uvm_re_compexec`/
  `uvm_re_deglobbed` follow UVM's C code, and `uvm_hdl_signal_size` and
  the `uvm_polling_*` value-change API are built in. UVM testbenches
  compiled without `UVM_NO_DPI` used to abort at time 0 on unresolved
  regex symbols.
* An `inout` or `output` formal passes a virtual-interface handle back to
  its caller even when the actual was null on entry (`uvm_resource_db::
  read_by_name` returned nothing), and a formal no longer leaks into a
  same-named class property.
* An unconnected 2-state `output` port of an interface or module starts at
  0 instead of x, so a clock driven through one toggles.
* Deferred immediate assertions (`assert #0`, `assert final`) run their
  action blocks at the end of the time slot; a report whose process
  resumes first is dropped, and one still pending at `$finish` prints a
  note instead.
* A path-delayed output also delays its first change at time 0, as the
  reference simulator does.
* `randomize() with { ... }` over struct members is about 13x faster.
* UVM tables print their rows: copying a whole element out of a
  class-property collection of unpacked structs (`row = m_rows[i]`, queue,
  dynamic, fixed or associative) gave zeros, so the topology print and
  `sprint()` tables were blank. Such elements now copy, return, compare and
  print with `%p` correctly, and `this.q[i].m` reads and writes them. `%s`
  of a string member reached through a select no longer pads, `%p` of a
  string-valued call prints the string, and `%0p` prints it unquoted.
* Unpacked-struct locals in `initial` blocks and subroutines: member writes
  through selects, packed-member fields and declaration initializers take
  effect, 2-state members start at 0, and members that are arrays of
  structs survive whole-struct assignment.
* Time-0 activation order follows the reference simulator: depth-first
  source order through the hierarchy, bound instances after their host's
  own items (several binds into one host in reverse order), and `initial`
  blocks that cannot suspend run as one group.
* Assertions: named properties and sequences declared in sub-modules and
  interfaces work in `assert property`, every instance of an assertion
  interface evaluates its own assertions (only the first one did), `bind`
  accepts interface targets by name or path, and an action block reports
  its assertion's own scope.
* A virtual interface can be bound to a nested interface instance by
  hierarchical path (`a.d`, `g[0].a.d`), and tasks can be called through a
  generate-block path.
* UVM's DPI-C functions are built in, so `+define+UVM_NO_DPI` is optional:
  regular expressions and globs for config_db, resource_db, factory
  overrides and `+uvm_set_*` plusargs, the command-line processor, and the
  `uvm_hdl_*` backdoor (read, deposit, force, release by hierarchical
  path). A `--dpi-lib` that defines a symbol still takes precedence.
* A second hierarchical write such as `u_sub.sig = v` went into a class
  object instead of the signal once any class object existed (every UVM
  testbench). 2-state variables in instantiated modules no longer hold x/z,
  and a released net takes its drivers' value immediately.
* Specify path delays count the declaring module's timeunit (they ran 1000x
  short in a 1ns/1ps module). All path forms are modelled (`*>`, polarity,
  edge-sensitive, `if`/`ifnone`, 2/3/6/12-value delay lists), the delay is
  chosen per transition and enabled path, and outputs reject pulses
  narrower than the delay. SDF IOPATH still overrides them.
* Identifiers declared nowhere in the design are an error inside task,
  function, class-method, package, interface and program bodies, as they
  already were elsewhere.
* Untyped parameters take the width and signedness of their initializer
  (`localparam C = {8'h1, 8'h2}` is 16 bits, `{4{B}}` and string literals
  keep all their bits, `parameter unsigned P = 5` is unsigned). A `string`
  parameter prints with `%s` like a string variable.
* Several tops each act as their own root: the internal wrapper name no
  longer appears in `%m`, messages, VPI or VCD/FST/XTrace scopes, a
  `defparam` path may start at a top, and `$bits(<type>)` in a child
  instance uses that instance's parameters.
* From the sv-tests suite (now 94.8%): `` `include `` of a function-like
  macro expands fully; paren-less `aa.size` counts entries; `item.index`
  works in locator `with` clauses; `repeat` rounds a real count; `expect`
  blocks until its attempt finishes; `= @ev rhs` without parentheses waits;
  input port defaults use the instance's own parameters; subroutines in
  labelled generate branches resolve; `$bits` of a class type parameter
  works in class constants. Strict mode rejects a second or mis-sized
  non-ANSI port redeclaration, `specparam`/module declarations inside
  generate blocks, class/string/chandle/event nets, and an unqualified
  interface-class typedef used by an implementing class.
* Casts evaluate their operand at the cast type's width (IEEE 1800-2017
  §6.24.1): `int'(a + b)` over 8-bit operands is 300, not 44. This holds
  in procedural code, continuous assigns, constants, packed dimensions and
  constraints; `signed'`/`unsigned'` keep the operand's own width.
  `pkg::T'(x)` casts are no longer dropped, and interpreted `/` and `%`
  size by both operands (`(a + b) / 3` gave 14 instead of 100).
* Typed parameters use their declared width as the context of their value
  (`localparam int P = A + B`), and module header parameters keep their
  declared signedness (`parameter bit [7:0] P = 200` read -56).
* Reductions and sorts with a `with` clause work on class properties
  (`o.arr.sum() with (item * 2)`, `this.arr`, `objs[i].arr`), and
  associative arrays reduce over their keys (`aa.sum()` returned 0).
* Associative-array elements follow their declared type: a missing key
  reads 0 for `int` and x for `integer`/`logic` (function and task locals
  read a 1-bit x), stores take the element's signedness, and `++`/`--`
  keep the operand's signedness (`seen[k]++` on a new key gave x).
* Specify-block timing checks are modelled: `$setup`, `$hold`,
  `$setuphold`, `$recovery`, `$removal`, `$recrem`, `$skew`, `$timeskew`,
  `$fullskew`, `$period`, `$width` and `$nochange`, with edge qualifiers,
  `&&&` conditions, notifiers (toggled in the NBA region, once per time
  step) and negative `$setuphold`/`$recrem` limits. A violation prints one
  `** Error: ... violation in <instance> at time <t>` line, counts toward
  `--error-exit`, and does not stop the run. `+notimingcheck(s)` and
  `+nospecify` disable the checks (the flag used to be a no-op), and
  `+no_notifier` / `+no_tchk_msg` are honoured. A `specparam` declared
  inside a specify block is now a module-scoped constant instead of being
  dropped.
* SDF TIMINGCHECK entries (with edges, `COND` and `INSTANCE *`)
  back-annotate check limits through `--sdf` and `$sdf_annotate`. SDF
  delays now scale to the simulation tick: designs with ps precision got
  delays 1000x too short.
* A package parameter takes its signedness from its declared type:
  `parameter bit B = 1` is 1, not -1, and a ranged untyped package
  parameter takes the declared range's width. The apb AVIP never built its
  master driver because of this.
* A task of an interface or module instance, called in a `forever` loop
  through a virtual interface or a hierarchical path, keeps running. The
  loop used to stop silently or with a false "null receiver" error.
* Clock generators keep first-in-first-out order with the other events of
  their time slot: a `#delay` scheduled after the generator's previous
  toggle resumes after this one. A reset asserted and released around a
  clock edge now spans that edge as it does in the reference simulator.
* A one-element array of instances (`bus_if b[1]`) has its element `b[0]`.
* `@(cb)` for a clocking block declared in a module or interface instance
  waits for the block's clock inside that instance's tasks and processes
  instead of returning at once.
* `randomize()` solves constraint sets that tie many variables together:
  an array `sum()` (including `with (... item.index ...)`), `unique`,
  orderings between elements or arrays, and `->`/`if` around them. When
  the per-variable solver fails, a bounded joint search takes over.
  Infeasible sets of this kind return 0 promptly and leave the random
  variables, array elements included, unchanged. Sums such as
  `a + b + c == K` compare at the full context width instead of wrapping
  at the operand width. A random size variable tied to `arr.size()` is no
  longer pinned to the previous size, and `item.index` reads the element
  index inside reduction `with` clauses.
* String methods on a subroutine formal or local (`name.len()`,
  `name.getc(i)`, `name.substr(i, j)`) read that variable, not a same-named
  signal of some design instance (an interface's `string name`). UVM's
  resource-name check read the wrong string, so its `UVM/RSRC/NOREGEX`
  warning never fired on testbenches with such interfaces.
* A module or interface task reached through a virtual interface or a
  hierarchical path resolves names in its own scope. Called from a class
  method, an unqualified call in its body used to bind to the calling
  class's method of the same name — which is how `uvm_info` inside a BFM
  task was attributed to the calling component instead of the global
  reporter.
* Covergroup coverage numbers match the LRM: `ignore_bins` and
  `illegal_bins` values are excluded from array and automatic bins; an
  automatic cross has the product of its coverpoints' bins (not of their
  value ranges); a crossed variable without a coverpoint is an implicit
  coverpoint that is sampled and counts in the group; `cg.cp.get_coverage()`
  and `cg.cp.get_inst_coverage()` report that coverpoint or cross (they
  returned the group's number); type coverage averages the instances unless
  `type_option.merge_instances` is set; `$get_coverage` weights each type by
  its `type_option.weight`; a `ref` constructor formal reads its actual at
  every sample; bin bounds may be expressions of constructor formals.
* A concurrent assertion clocked by an event without an edge (`@(clk)`, or
  `@clk` on a checker's event formal) samples on every change of the clock,
  as §9.4.2 defines; it used to sample on the rising edge only.
* A checker's event formal accepts an edge actual (`chk u(v, posedge clk)`),
  which used to be a parse error, and the checker's assertions clock on
  that edge.
* A declaration after a statement in a `begin`-`end` or `fork`-`join` block
  or a subroutine body is rejected (§9.3.1); `--no-strict` still accepts it.
* A replication whose count is a signed constant expression is read as
  signed: `{$bits(T) - 1{1'b1}}` in a type-parameterized class whose type
  parameter is unresolved replicates nothing instead of wrapping to about
  four billion copies, which never finished elaborating. Specializations
  keep their widths (`int` gives 31 ones, `byte` gives 7).
* `mailbox` and `semaphore` objects are constructed by every form of
  `new`: a parenthesis-less `mailbox mb = new;` or `mb = new;`, and a
  declaration inside a block (`automatic mailbox #(int) mb = new(2);`,
  `automatic semaphore s = new(1);`). These left the mailbox null (the run
  stopped at "null mailbox handle") or the semaphore without its keys.
* Reading an unpacked-array element with an out-of-range index yields x at
  the element's width (§7.4.6). Three paths produced a 1-bit x that the
  store then zero-extended to `0000000X`: the fused memory-read flop
  (`q <= mem[i]`), a blocking read in a process (`a = mem[i]`) and the same
  read through a task output. Combinational reads were already right.
* Selecting from a class property inside an `always` block or an `assign`
  reads the property instead of zero. `bk.arr[i]`, `bk.arr[3][0]` and
  `bk.arr[i][3:0]` asked the interpreter for `bk.arr` alone, which is an
  array and does not reduce to one number, and then selected from that
  number; an accumulator fed by `bk.arr[i]` stayed at 0 while `$display` of
  the same expression printed the right element.

**Usability**

* Code coverage: `--code-coverage[=stmt,branch,toggle]` (all three by
  default), `--code-coverage-scope=<path>[,...]` and `XEZIM_CODE_COVERAGE`
  turn on statement, branch (if/else and case arms including the implicit
  ones, `?:`) and toggle (per-bit rise/fall) coverage; `+cover[=sbt]` and
  `-coverage` now enable it too. Results go in `xezim_cov.json` under
  `code_coverage`, per instance and per design unit, with a per-scope
  summary under `--verbose`. It is off by default and costs nothing when
  off. See docs/coverage-guide.md.
* `$fwrite`/`$fdisplay` to regular files are buffered (64 KB per handle)
  and flushed on `$fflush`, `$fclose`, `$finish`, `$system`, `$fopen`,
  `$readmem*`/`$writemem*`, DPI/VPI calls and before any read of the same
  file; a 400k-line trace testbench went from 1.78 s to 1.15 s. A
  `tail -f` on a trace file now lags until the next flush.
* The default simulation time limit is 100 ms instead of 100 µs, so UVM
  tests that run for milliseconds (for example 2–25 ms of simulated time)
  finish instead of stopping early. A design that never calls `$finish`
  still stops at the limit; `--max-time` sets a different one, and
  `-do "run -all"` runs until `$finish`.
* Command lines written for other simulators run as-is: bare or `work.`
  top names, `-F`, `-g`/`-G` parameter overrides, `-sv_seed`,
  `-sv_lib`/`-sv_root`, `-svNNcompat`, and a `-do` subset (`run -all`,
  `run <time>`, `quit`, `exit`, `do <file>`; `log`, `add wave` and
  `coverage save/report` are accepted with a warning; anything else is an
  error). Library options (`-work`, `-L`, `-lib`) are ignored with one
  warning; `-c` and `-l` keep their xezim meanings. `run -all` runs past
  the default time cap, and after `run <time>` the closing line and
  `final` blocks see the stop time.
* Runs are quiet by default. The transcript holds the design's output,
  warnings and errors, and one closing line, `Simulation finished at time N
  ($finish called)`. The version banner, `[PHASE]` timings,
  `[PROF]`/`[FUSE]`/`[EVENT-EDGE]`/`[COV]` counters, `[CACHE]` hits and
  `--compile`'s design summary now need `--verbose` (or `XEZIM_VERBOSE=1`,
  `--profile` or `--sim-debug`). The apb AVIP transcript went from 189 lines
  to 150.
* Parse, preprocessor and elaboration errors print `file:line:col` with the
  source line and a caret. A line that came from an `include`d file names
  that file, with an "In file included from" chain, and text produced by a
  macro names the macro invocation. Runtime locations (hang reports,
  port-width warnings) are no longer shifted by `include`s.
* Preprocessor errors and warnings print once per run instead of once per
  pass. Parser warnings, such as an unsupported UDP table, now carry their
  location, and `` `__LINE__ `` after a multi-line `define` reports the right
  line.

**Performance** (instruction counts, output identical)

* UVM class code, round three: member reads, calls, member-target
  assignments and comparisons take direct paths, and flat names, handle
  keys and element names are built without `format!` or clones. UVM stress
  bench: 10.8 M -> 9.0 M host instructions per item (-17%); the six AVIPs
  run 13-15% fewer instructions (axi4 46.6 G -> 39.8 G).
* RTL: process FSMs resolve `<top>.path` reads (a testbench monitor ran on
  the AST path with name lookups every clock), `check_edges` reuses its
  fired-edge buffers, and FSM wait terms are cached: c906 CoreMark 471.5 G
  -> 459.5 G (-2.5%), c906 memcpy -2.1%.
* UVM class code, round two: formal snapshots by position, plain-type
  fast paths for locals and formals, per-class property-owner indexes,
  forward-declared classes (`typedef class X;`, 110 in UVM) on the plain
  paths, and name-set gates in member and call lookup. UVM stress bench:
  11.5 M -> 10.1 M host instructions per item (-12%); axi4 AVIP -6.2%.
* c906 CoreMark -0.4% (elaboration -2.4 G): loop-feedback analysis only
  walks reads when a write reaches a combinational block, waiter wake
  ranks are tracked only once a nonzero rank fires, and the legality
  checker hashes scopes faster and builds messages lazily.
* UVM class code: name-kind checks answer from per-design name tables and
  per-class memos, bare names read and write on direct paths, class task
  bodies are shared instead of cloned per call, and `foreach` over
  associative arrays uses the element index. UVM stress bench: 20.6 M ->
  11.5 M host instructions per sequence item (-44%); the six AVIPs run
  19-27% fewer instructions; c906 CoreMark unchanged.
* A two-state block that reads an x or z bit no longer re-runs on the
  four-state VM: the same lowered stream runs on an x-plane executor that
  applies the VM's four-state rules (Kleene logic, `&&`/`||`/`!` through
  definite-1/definite-0/unknown, ambiguous equality, all-x arithmetic,
  plane shifts, an x selector merging its arms, `if (x)` taking the else
  arm, an x index reading all-x and writing nothing). On the SoC
  benchmarks three quarters of what the VM still executed were such
  re-runs — tiny muxes whose unselected arm or an unwritten register file
  holds x. `XEZIM_TS_X=0` restores the VM re-run; `x_plane_runs=` counts
  them in the profile report.
* Dynamically indexed reads of memory elements wider than 64 bits — a
  vector register file, a cache-line array — now run on the two-state
  executor, both as operands (`vrf[rs]` in wide logic) and as the fused
  memory-read flop (`rdata <= line[raddr]`). They were the largest labelled
  reason for a clocked or combinational block to stay on the four-state VM
  on both SoC benchmarks (21% of a C910 SoC's interpreted combinational
  evaluations, 31% of a C906's).
* `scripts/build-pgo.sh` run without a training command builds a
  profile-guided binary from a bundled trainer (the `tests/perf` shapes,
  `scripts/pgo-train`, `xezim-bench`): about −0.5% host instructions on the
  SoC benchmarks, −12% on a loop-heavy DRAM model, output identical. It is
  the recommended release build; see README.
* Loop-heavy clocked blocks with `int` counters — memory models that walk
  lanes, byte-lane writes into a packed memory, per-lane write pointers —
  now run on the two-state executor instead of the four-state VM. The
  counter's signed tag, its increment, a slice read at a run-time offset and
  range stores at a run-time offset (blocking and non-blocking, into
  vectors wider than 64 bits) all lower now. A DRAM-model reproducer runs
  in a third of the instructions.
* Clocked blocks whose range-store bounds only become constants after
  folding — a generate arm writing `v[g*8 +: 8]` or `mem[g][7:4]` — now
  compile to constant range stores on the element, which the two-state
  executor runs; the dynamic and array forms they used to take kept the
  whole block on the four-state VM (16 such flops on a c906 SoC, 400 on a
  C910 SoC).
* A block that bails on an x read every time it runs (an unwritten memory
  element it keeps reading) no longer pays the two-state entry and guard on
  every evaluation before running interpreted: after eight bails in a row
  it sleeps, doubling the sleep while the bails continue, and wakes for one
  attempt so a value that clears after reset gets its fast path back. x-read
  bails on a c906 SoC fall by 72%.
* Combinational blocks with registers wider than 128 bits — up to 512 —
  run on the two-state executor by default now (`XEZIM_TS_WIDE512=0`
  restores the old behaviour). Three shapes that kept such blocks on the
  four-state VM lower as well: a bus read and then rewritten in the same
  block (`bus = {bus[..], ..}`), a mux between two wide values, and a
  wide `'x` reset default (`{N{1'bx}}`). c906 memcpy runs 1.2% fewer
  instructions from the wide class alone, a C910 SoC 2.5%.
* `tests/perf/loop_block_counters.rs` guards this path with work-counter
  ceilings (two-state admission, bytecode length, backoff engagement).
* The bytecode such blocks compile to is leaner first: a loop variable's
  reads are forwarded into their consumers instead of being copied into a
  temporary each time, `& K`, `* K`, `- K` and `| K` fold their constant
  operand like `+ K` already did, a constant that only becomes one after
  folding is fused too, and `i = i + 1` is two instructions rather than
  five. The same reproducer executes 34% fewer VM instructions before the
  two-state gain above.
* A clocked block whose inputs did not change may now skip an idle edge even
  when another block writes a different part of the same register or a
  different element of the same array. Generated logic that gives
  `status[0]` its own `always` block beside one for `status[6:1]`, or
  `mem[0]` beside `mem[1]`, used to keep every such block firing every
  cycle; each owns its own bits, so all may rest. Blocks that write
  overlapping bits, and any block beside a writer whose element index is
  computed at run time, still fire on every edge. A C910 SoC runs 6.6% fewer
  instructions, a c906 SoC 1.2%.
* Writes that combine two dynamic steps into a packed vector now compile
  instead of falling back to the interpreter: `q[i][j]`, `mem[a][(i*W) +: W]`,
  `s.arr[i].field` and their blocking forms, with either index dynamic. A
  fallback inside a loop over a register-held variable demotes the whole loop,
  so these statements cost microseconds each; a memory model built from them
  runs 41x faster here, and a c906 run is unchanged.
* Selects whose declared dimension does not start at zero or runs ascending
  compile instead of falling back: `v[hi:lo]`, `v[b +: w]`, `v[b -: w]` and
  bit selects on such a vector, and on an element of a packed array. A lane
  vector selected in a loop runs 11x faster here. An element of an UNPACKED
  array keeps the interpreter path, where its label mapping already agrees
  with the writes.
* `XEZIM_FALLBACK_SITES=1` reports every construct handed to the interpreter
  with its reason, source byte span and scope, so the statements worth
  compiling on a slow design can be found without guessing. It now also
  reports the expensive case, a statement that takes its whole loop to the
  interpreter because a fallback cannot be emitted inside one.
* Merging same-sensitivity clocked blocks no longer costs the idle-edge
  skip. The blocks folded into a merged one keep their original statements,
  and the skip census still counted those writes, so every merged output
  looked like it had two drivers and the merged block was disqualified: on a
  c906 SoC only 618 of 3605 blocks could skip, and 55 million flop fires that
  used to be skipped ran. Merging is now worth 6.9% on that design instead of
  costing 18%.
* Reading an element of a packed memory no longer copies the whole memory.
  `mem[addr]` on a `logic [N-1:0][W-1:0]` loaded every bit of `mem` into a
  register and selected the word from that, so each read cost as much as the
  memory is big: a 16x deeper memory cost 3x per read. The slice is taken
  where it lies now, read cost is flat in the memory's size, and a
  32-port read benchmark runs 42% faster.
* Starting a design with large memories is faster again: deciding which
  clocked blocks may skip idle edges asked a hash map, once per element of
  every memory a block writes, whether anything else wrote it. A c906 SoC
  spent 1.8 seconds and several hundred megabytes on 19 million of those
  questions; one byte per signal answers them now, and whole arrays are
  counted as ranges. The pass went from 2.8 seconds to 30 milliseconds and a
  c906 run drops another 4%.
* Designs with large memory arrays start simulating sooner: the pass that
  decides which clocked blocks may skip idle edges named every element of
  every memory a block writes, one string per element, and then read names it
  discarded. A c906 SoC spent 33 seconds there, a second per one-million-entry
  SRAM; that is now arithmetic on the element range, with the same skip
  decisions. The whole run drops 36% of its instructions.

# What's new in 0.10

### 0.10.6 — assertion engine, instance-scoped collections, faster small testbenches (September 2026)

**Correctness**

* Concurrent assertions evaluate as attempts with real sequence matching
  (issues #176–#183): a multi-cycle antecedent (`a ##1 b |-> c`) triggers on
  its last cycle; a ranged consequent (`##[1:2] b`) fails only when the whole
  window has passed; `not (a ##1 b)` fails only on a match; `$past` inside a
  deferred consequent reads the previous clock cycle and returns the type's
  default before enough history exists; `disable iff` cancels attempts in
  flight; property `and`/`or` parse and evaluate; and `(##2 (b))` means the
  same as `##2 b`.
* Sequence operators that used to be accepted and ignored now evaluate:
  repetition `s[*n]`, `s[*m:n]`, `b[->n]`, `b[=n]`; `throughout`, `within`,
  `intersect`, sequence `and`/`or` (also inside an antecedent);
  `first_match`; `strong(...)`/`weak(...)`, with a strong property still
  pending at the end of simulation reported as a failure; `@(negedge clk)`
  and `@(posedge clk iff en)` clocking events; a clockless `assert property`
  under `default clocking`; and named sequences and properties with formal
  arguments used inside a property body.
* Package-qualified variables inside tasks and functions: a write such as
  `store_pkg::scalar = d;`, `pkg::arr[i] = v;` or `pkg::v[3:0] = x;` in a
  subroutine body was silently dropped, and a read of `pkg::arr[i]` or
  `pkg::v[h:l]` there returned 0; both worked from an initial block.
* `%m` inside a continuous assignment whose expression falls back to the
  interpreter names the assignment's instance, and `$time` there scales to
  that instance's timescale, as always-block fallbacks already did.
* With the native compiler on (`XEZIM_JIT=1`), a `$display` or other
  interpreted statement inside a compiled clocked block now runs under the
  block's own instance: `%m` names the instance, bare names resolve there,
  and `$realtime`/`%t` use its timescale. Previously such statements ran
  under the top scope.
* With the native compiler on, a bit-select, part-select or array element
  read whose index or bound carries x/z now yields x across the full
  width, and an element or part-select write with such an index modifies
  nothing, matching the interpreter and IEEE 1800 7.4.6 / 11.5.1. The
  native code previously zeroed the unknown bits and used the result as
  the index.
* A constant part-select write with a negative label (`x[4:-1] = v`,
  `x[0:-1] <= v`) in a clocked block or compiled process keeps its in-range
  bits (IEEE 1800 §11.5.1); the compiled path folded the bounds unsigned,
  dropped the write, and the non-blocking form stored a bit past the MSB.
* A non-blocking read of an array element through an index that holds x or
  z (`q <= mem[idx]`) queues an all-x element instead of element 0, in the
  two-state and four-state engines alike.
* A constant part-select `[l:r]` whose bound is a variable is now an
  elaboration error, as IEEE 1800 §11.5.1 requires; only `[base +: width]`
  and `[base -: width]` take a variable base. Every engine previously read
  the bound from the variable's current value.
* Bit- and part-selects follow IEEE 1800 §11.5.1 in every engine: an
  index or bound with an x or z bit reads x and discards the write instead of
  using bit 0; a constant select below a vector's declared low bound reads x
  instead of the low bits; an ascending vector (`logic [3:10] a`) places label
  `p` at bit `10 - p` for reads and writes alike, so `a[3] = 1` sets the MSB;
  an out-of-range or unknown element index into a packed 2-D value reads a
  whole-x element. A part-select that is partially out of range keeps the
  §11.5.1 per-bit form (only the out-of-range bits read x);
  `XEZIM_OOB_SELECT=whole` makes such a select read x as a whole and
  discards a write through it, as some simulators do.
* An intra-assignment delay inside an edge-triggered `always` block is
  honoured: `q <= #5 v;` schedules the update five time units out and
  `q = #5 v;` suspends the block, as in an `initial` block. Both forms
  previously assigned at once with no warning (#160).
* A class property that is a fixed array of collections (`int q[2][2][$]`,
  `int d[3][]`, `int a[2][int]`) has storage for every element: `q[i][j]`
  accepts `push_back`, `size`, `new[n]`, `exists` and element reads and
  writes, from inside the class and through a handle. Previously each
  element collection was silently empty while `$size(q)` and `foreach`
  answered off the outer shape.
* An unpacked array parameter whose elements are assignment patterns
  (`localparam cfg_t A [3] = '{'{4,2}, …}`) evaluates each element instead
  of reading 0, for packed-struct, unpacked-struct and packed-vector
  element types declared at compilation-unit scope. A constant function
  containing `signed'(e)` or `unsigned'(e)` is now evaluated at
  elaboration, so a `localparam` or typedef width derived from it in a
  sub-instance is correct (it read 0, giving one-bit typedefs).
* A continuous assignment accepts the rise/fall/turn-off delay form,
  `assign #(rise, fall[, turnoff]) net = expr;`, and applies the delay by
  transition as §10.3.3 specifies; a transition to x takes the smallest.
* Collections declared in a module keep one copy per instance. Sibling
  instances of the same module no longer share a queue, dynamic array or
  associative array, and a packed-struct element of such a collection
  (`q[i].field`) reads and writes correctly inside any sub-instance. A
  UVM-style BFM with ten per-client request queues now grants requests.
* Constrained random: without `solve … before`, the antecedent of an
  implication is drawn in proportion to the solution space it selects, as
  §18.5.10 requires, and the consequent's variables are drawn inside the
  implied ranges. `solve … before` keeps the antecedent uniform.
* VPI: `vpi_iterate(vpiPort, module)` yields port objects with name,
  full name, direction and size for the top module and sub-instances, and
  values read through the connected signal; nets and variables keep their
  `vpiNet`/`vpiReg` types. `vpiPort`, `vpiPortBit`, `vpiDirection` and the
  direction values are in `include/vpi_user.h`.
* Write-path fixes: a bit-select write whose index is x or z modifies
  nothing; a continuous assign with a constant right-hand side drives its
  net in a UVM testbench; a non-blocking assignment to a packed-array
  element through a virtual interface keeps its width; a write to a nested
  packed-struct member (`f.hdr.d = v`) lands; elements of a class
  unpacked-struct array read back what was written.
* `always @(sig)` keeps firing after a write made by a process that a
  clock edge resumed.
* A named event or signal written by a process that a clocking block
  resumed (`##2; -> ev;`) wakes its waiters in the same time step instead
  of one clock toggle later.
* A narrow actual bound to an `int`, `logic signed` or `bit signed`
  class-method or constructor formal is extended correctly (`new(2'b10)`
  read −2).
* `#delay` inside a package class, a compilation-unit class, a `$unit`
  task or function, or a `program` scales by the timescale in effect; a
  `timeunit` declared inside a package is honoured.
* Handle chains of any length (`w.r.c`) read correctly from a task inside a
  sub-instance, including a task-local handle named like a sibling
  instance.
* Locals declared inside an instance's tasks, functions and blocks shadow
  the module's own names.
* `bind` with a parameter value assignment is applied; previously the
  bound harness never existed.
* A `ref` formal named like its actual no longer overflows the stack.
* Class methods reach sibling module instances by hierarchical reference
  (issue #155): `core.seq`, `core.get_seq()` and `u_w.p.peek()` work from a
  method of a class declared inside a module.

**Performance** (instruction counts, output identical)

* A compiled stimulus process (the default for clocked initial-block loops)
  now runs its wait-to-wait segments on the two-state executor: waits are
  opcodes that suspend the executor, the process keeps its own two-state
  register file, and a segment that reads an unknown value re-runs on the
  four-state VM from the values it started with, backing off after repeated
  bails. The 100k-cycle self-checking testbench: 2.20G to 2.08G
  instructions; c906 neutral. `XEZIM_PROC_FSM_TS=0` keeps the VM.
* Two more per-tick trims for small testbenches: the check of whether a
  compiled stimulus process can clobber a combinational entry's outputs is
  cached per entry, and a compiled block's two-state stream is no longer
  reference-counted on every evaluation. The 100k-cycle self-checking
  testbench: 2.29G to 2.20G instructions; c906 neutral.
* A clocked block whose only uncompilable statement is a `$display`-style
  call on a cold branch (a check macro's failing arm) now runs on the
  two-state fast path; the interpreter is called for that one statement
  only when it is reached. Blocks that read a signal they later overwrite
  (`failures++`) are admitted too, with those signals restored on a bail
  so the four-state re-run stays exact, and `'0`/`'1` fill literals lower.
  A 100k-cycle self-checking testbench: 2.50G to 2.28G instructions;
  c906 neutral.
* Small clocked testbenches run about 15% fewer instructions: the per-tick
  scratch vectors of the waiter drain, the clock generators and the
  in-process combinational snapshot are reused instead of reallocated,
  parked waiters that do not fire stay in place, and a compiled stimulus
  process skips snapshotting the outputs it can never write. A 100k-cycle
  self-checking testbench: 2.94G to 2.50G instructions, output identical;
  c906 neutral.
* Experimental, opt-in: `XEZIM_TS_WIDE512=1` lets the two-state path carry
  registers up to 512 bits (the c906 vector-unit buses) instead of 128. It
  is off by default because on c906 it costs 5% more instructions: the
  admitted blocks read x-holding buses and fall back every evaluation.
* A testbench's stimulus loop in an `initial` block — clocked waits,
  blocking or non-blocking assignments, `if`/`case`, counted loops and
  system tasks — now runs as a compiled process by default instead of on
  the AST interpreter. A 100k-cycle self-checking testbench went from
  2.8 s to 0.37 s. `XEZIM_PROC_FSM=0` restores the interpreter for every
  initial block; `XEZIM_PROC_FSM=1` still compiles every body that can be.
* The `--profile` report prints the time that fell outside combinational
  entries and clocked blocks (interpreted processes, waiters) as its own
  line, so a run dominated by an interpreted testbench loop no longer
  reports 90% attributed to the little that was sampled.
* `--profile` no longer slows the run it measures: construct times come
  from a 10 kHz sampler thread instead of two clock reads around every
  evaluation. On a 16k-cell gate-level DRAM the profiled simulation phase
  went from 39% slower than an unprofiled run to about 5%; the report's
  percentages are unchanged and its header now states the sample count.
* Two- and three-part concatenations on the two-state fast path carry their
  operands in the instruction, and the settle loop prefetches the header of
  the next injected entry. C906 memcpy at 300 iterations: 2 % fewer cycles
  across both changes, output identical.
* A testbench loop that writes a memory through an absolute hierarchical
  path (`tb.x_soc.<...>.ram0.mem[i][7:0] = ...`) now compiles to bytecode
  like a local-array store. The XuanTie C910 and C906 memory-image loops
  (2.1 million stores at time 0) run in 0.9 s instead of 2.9 s; C906
  memcpy at 300 iterations: 7.7 % fewer instructions, output identical.
* A successful run exits as soon as its output is complete instead of
  first freeing the whole design; on the C910 that was 1.6 s after the
  last line of output.
* Statements run by the interpreter no longer scan the design's whole
  instance list to tell an interface instance from a signal on every
  execution; C910 memcpy at 200 iterations: 135 s -> 127 s.
* The edge detector no longer re-baselines every edge signal after each
  pass: under the dirty-edge scan only the signals that changed are
  re-baselined (plus the operands of sampled-value functions), which is
  what the scan already did for them. C906 CoreMark: 4.0 % fewer
  instructions, output identical; UVM benchmarks unchanged.
* `casez`/`casex` decoders compiled to a jump table now lay their wildcard
  chains out with forward jumps only, and a `casez`/`casex` compare against
  a constant pattern runs on the two-state fast path. The C906 instruction
  decoders (up to 7,500 instructions each) leave the 4-state interpreter;
  CoreMark 0.3 % fewer instructions, output identical.
* The bytecode compiler folds constant register chains: an unrolled
  `i = 0; bus[i] = v; i = i + 1; …` sequence (the C906 decode blocks carried
  230 chained constant adds each) becomes static bit writes, and constants
  nothing reads are dropped. C906 CoreMark: 5.2 % fewer instructions,
  output identical; UVM benchmarks unchanged. `XEZIM_FOLD_CONST_REGS=0`
  disables it.
* The idle-edge prefilter that decides whether a clocked block runs at a
  clock edge now reads one packed state byte per block instead of four
  flag arrays. C906 CoreMark: 1.3 % fewer instructions, output identical.
* Combinational blocks that write one bit or a constant-bound slice into a
  bus wider than 64 bits (`bus[k] = v;`, `dst[63:0] = src[127:64];`, the
  C906 decode-bus shapes) now run on the two-state fast path instead of the
  4-state interpreter, and clocked blocks that read wide buses skip idle
  edges in the prefilter. C906 CoreMark: 2.6 % fewer instructions on top of
  the array-arming change, output identical; UVM benchmarks unchanged.
* Edge-triggered blocks that read or write an unpacked array through a
  dynamic index (register files, memories: `q <= mem[raddr]`,
  `mem[waddr] <= d`) now take part in the idle-edge skip. Every element of
  the array arms the block on write, so an edge with no input change is
  skipped instead of re-executed. On the C906 CoreMark run 858 of 906
  previously always-executed flop blocks now skip: 8.1 % fewer
  instructions, 11 % fewer cycles. UVM benchmarks unchanged.
* `XEZIM_CYCLE_MODE=cycle` selects the cycle-based engine (default
  `event` is the engine as before). Its first stage evaluates the clock
  tree eagerly at each clock-generator edge instead of through the
  combinational worklist; on the C906 CoreMark run it converts the clocks
  of 64 % of the edge blocks, cuts settle passes by 12 %, and produces
  identical output for 0.4 % fewer instructions. Later stages will add
  cycle stepping after reset with event-driven fallback.
* The statement interpreter's three largest routines keep smaller stack
  frames (the statement dispatcher went from 7.8 KB to 3.8 KB per nested
  call), so a deeply nested testbench statement chain stays in cache: the
  UVM benchmark runs about 1.7 % fewer cycles, output identical.
* Process wake-ups are cheaper: the scheduler no longer hashes with
  SipHash, allocates an empty continuation, or clones the process scope
  string on every wake-up, and the timing wheel covers 4096 ticks before
  spilling to the ordered overflow; the next event time is memoized and
  an empty waiter list is skipped. A timed real-number model runs 26.9 %
  fewer instructions; the UVM and CPU benchmarks are unchanged.
* A delay-driven `always` block with a compound body, the timed
  integration step of a real-number model, runs from compiled bytecode
  instead of the AST interpreter: 3.8x faster per step on a fitted-lag
  model (4.1 µs to 1.07 µs), results unchanged (#159).
* Combinational settle passes track entries triggered mid-pass in a bitset
  instead of a heap, array element accesses in compiled blocks resolve
  inline, and an assignment whose value already has the target width copies
  it directly: 5.6 % fewer instructions on the C906 CoreMark run.
* Reads and writes from class methods no longer build a scoped name string
  for every lookup, and virtual-interface bindings are probed without
  allocating, an assignment no longer probes for a pending interface
  return on every write, and the width of a plain variable target is
  remembered per statement: 3.6 % fewer instructions on the axi4 AVIP.
* Clocked monitor blocks that contain a rare `#delay` run compiled instead
  of interpreted, with the same process semantics: 0.3 % fewer instructions
  on C906 CoreMark.
* Combinational settle passes evaluate each entry once per pass, and
  clocked blocks with a blocking statement no longer copy their body on
  every activation: 5.3 % fewer instructions on the C906 CoreMark run.
* Two-state blocks check for x/z as they load: 3.6 % fewer on C906
  CoreMark.
* Wide values (over 64 bits) are copied, tested and resized a word at a
  time; 128-bit concatenations and single-bit replications are built in
  place: 20 % fewer on C906 CoreMark.
* Faster process re-parks and two-state block execution:
  2 % fewer on C906 CoreMark.
* Arithmetic operands are evaluated once when their width is needed: 9 %
  fewer on the axi4 AVIP.
* Parked `wait(cond)` processes that read only class state stay parked
  until that state changes: 11 % fewer on a UVM bench.
* Fewer per-identifier lookups inside class methods: 4.9 % fewer on the
  axi4 AVIP.
* Clocked blocks that only need arming skip the value compare, two-state
  blocks run from one contiguous code arena with a packed per-entry
  header, and the settle loop takes the two-state path before touching
  the entry table: C906 memcpy (2000 iterations) 375 s to 313 s, output
  identical.
* Two-state blocks may now carry signed narrow registers (`integer` loop
  counters and their compares), dynamic bit selects, non-blocking dynamic
  bit writes (`q[i] <= v`) and range writes into buses wider than 64 bits;
  a block is admitted after a read-before-write analysis of its control
  flow instead of a fixed statement-order rule. Every clocked block is
  offered to the two-state lowering regardless of size, and the eight most
  common adjacent instruction pairs run fused. The four `for`-loop flop
  blocks that dominated the C906 interpreter time (arbiters, fill buffer,
  GPIO) now run two-state: memcpy 313 s to 295 s, CoreMark 18 % fewer
  cycles, output identical; UVM benchmarks unchanged.
* Single-bit gates whose output only clocked blocks read (half of all
  combinational evaluations on the C906) are evaluated in one batch at the
  end of each settle pass instead of through the worklist; a default
  assignment of a wide bus (`bus = {265{1'b0}}`) and a 65..128-bit window
  written into a wider bus now lower to the two-state path; the signal
  mirror that only native (JIT) code reads is no longer maintained on
  every write in ordinary runs. C906 memcpy (2000 iterations) 295 s to
  275 s, output identical; UVM benchmarks unchanged.
* A write to a signal that no clocked block or edge sensitivity observes
  (59 % of all writes on the C906) skips the write observer after one
  lookup, and single-bit gates feeding only clocked blocks commit through
  a direct inline path. C906 memcpy 275 s to 263 s, output identical; UVM
  benchmarks unchanged.

### 0.10.5 — class covergroups, DPI exports and unit scope, faster UVM (September 2026)
* **Typedef'd packed arrays keep their dimensions inside instances**: a
  `u7_t [4:0][1:0] a` declared in an instantiated module (including every
  top of a multi-top design, which runs under the synthetic wrapper) had no
  packed geometry recorded, so `foreach (a[i, j])` walked its 70 bits
  instead of its 10 elements while the same module run as the selected top
  was right. The declared dimensions are now chained with the typedef's for
  instance variables, ports and nets alike.
* **`--profile`** prints the end-of-run profile report (by design unit,
  instance and construct, plus the opcode and entry histograms); the same
  as `XEZIM_PROFILE_REPORT=1`.

* **`foreach` and `std::randomize` over multi-dimensional targets**: a
  `foreach (a[i, j])` over a purely packed array (`u7_t [4:0][1:0]`,
  `bit [6:0][4:0][1:0]`) now iterates every named dimension, declared
  dimensions first and then the typedef's (it used to iterate one and leave
  `j` x). `std::randomize(...) with { foreach (a[i, j]) ... }` now draws a
  packed target wider than 64 bits and every element of a 2-D or N-D unpacked
  array (both were left at 0), checks the constraint body with all loop
  variables bound (it passed vacuously before), and repairs per element:
  relational bounds, `elem == e` pins, and `$countones(mask[i][j]) ==
  count[i][j]` couplings, which draw the mask with exactly that many ones.
  A `rand` class property wider than 64 bits is drawn in full as well.
* **`export "DPI-C"` aliases and package-scope exports reach C**: an export
  with a C linkage name (`export "DPI-C" c_reg_write = task reg_write;`)
  now emits the `c_reg_write` symbol the loaded library calls (it emitted the
  SV name, and the library died with `undefined symbol: c_reg_write` on its
  first call). Exports declared inside a package are registered whether the
  package is wildcard-imported, imported by name, or never imported (they
  name a global symbol either way); an unimported package's subroutine is
  reached under its qualified name.
* **Loop variables shadow a same-named variable of an inlined instance**: a
  `for (integer i = 0; ...)` or `foreach (a[i])` inside a child module that
  also declares `integer i` at module scope now binds `i` to the loop. The
  inliner used to prefix every use of `i` to the child's module variable
  while the loop's own declaration stayed bare, so the loop compared an
  x-valued `u.i` and never ran (a gray-code pointer decoder stayed at x and
  an asynchronous FIFO popped the same word forever). The interpreted form
  had the matching runtime defect: the loop variable was written by name
  through the process scope, which for a `foreach` re-triggered the block
  on its own write.
* **Associative-array probes no longer scan the whole signal table**:
  `exists()` on an absent key, the nested-element probe behind every
  associative-array check, and `first()` / `next()` key enumeration now read
  the per-array element index (one set per array) instead of comparing
  every signal name in the design against a prefix. The associative-array
  check itself exits after one byte scan when the name can only be a plain
  collection, the static-collection key is borrowed instead of allocated on
  every builtin-method call, and a dozen per-call debug and tuning flags
  (`XEZIM_ACTIVE_REGION`, `XEZIM_TRACE_SPIN`, `XEZIM_PSETTLE_STATS`, the
  `*_DBG` switches) are read once. The axi4 AVIP retires 6.7 % fewer
  instructions, output identical. `XEZIM_BM_CENSUS=1` prints every builtin
  method call as `[bm] <receiver> <method>` for aggregation.
* **`obj.randomize()` over multi-dimensional properties**: a packed
  multi-dimensional class property (`rand u7_t [4:0][1:0] d`) now has element
  geometry, so `d[i][j]` reads and writes address the element (they were
  single-bit selects) and `foreach (d[i, j])` iterates every element from a
  method or from the module. Elements of a 2-D array property wider than
  64 bits are drawn (they stayed 0). Every fixed array property is drawn on
  every call and the constraint repair then runs over the fresh draws: arrays
  under a `foreach` used to be skipped by the draw and repaired from their
  previous values, so `e[i] < 100` kept zeros and repeated calls returned the
  same values, and the draw used to clobber element pins (`a[0] == 5`
  returned 0) and `foreach` bodies that read another drawn array
  (`$countones(m[i][j]) == e[i][j]`).
* **A `forever` / `always` process no longer re-clones its loop body on every
  wake-up**: the continuation it parks with is built once per loop and
  shared afterwards (C906 memcpy retires 4.2 % fewer instructions, output
  identical). A subroutine-local `virtual` interface variable now binds in
  the frame that owns it, so two class tasks interleaved on delays keep their
  own bindings instead of reading each other's. `cover property` sites are
  tallied as covers in every clocked path, including `s_eventually` /
  `s_always` watchers and vacuous implications. From Thomas Burg's PR #150:
  the two condition-waiter drains are one parameterised routine, the
  `--max-time` hang report lists processes parked for the NBA region, and the
  `this`-property probe no longer clones the class name per lookup.
* **Packed-struct member selects no longer collide with same-named arrays**:
  inside an instance, `inp.sram_renA[2]` on a struct port compiled as a
  two-bit element select whenever any other module declared a packed
  multi-dimensional array called `sram_renA`, because the compiler's
  element-width and dimension lookups fell back to the bare leaf name. They
  now try the exact name, then the instance-scoped name, and use the bare
  leaf only for single-segment names. Elaboration now also removes the bare
  declarator keys that inlining a submodule registers for its own body
  (element widths, packed dimensions, struct layouts, string signals) once
  that instance is fully inlined, so they can no longer be matched from
  anywhere else in the design.
* **System-function results keep their LRM width in compiled blocks** (§20,
  §21): `$countones`, `$clog2`, `$bits`, `$size`, `$countbits`, and the other
  `int`-valued functions contribute 32 bits to an expression's context, the
  `bit`-valued ones 1, `$time` 64, and `$signed`/`$unsigned`/`$past` their
  argument's width. Inside an `always_ff`, `narrow <= $countones(be) >> 3`
  used to size the shift at the 4-bit target and truncate the count before
  shifting; the procedural path was already right. From the audit that
  followed: `int`-valued results are now SIGNED everywhere (`$countones(x) - 8
  < 0` compares signed, `$fgetc` end-of-file tests below zero), the
  interpreter no longer sizes a system call by evaluating it (`$fgetc(fd) &
  mask` consumed two bytes and `$urandom % n` advanced the generator twice),
  `$test$plusargs`/`$value$plusargs` return `int`, a procedural `$past(v)` is
  no longer one edge late and reports "no history" at the operand's width,
  `$sampled(e)` evaluates outside properties, and `$onehot`/`$onehot0`/
  `$isunknown` fold to one bit in constant expressions.
* **Performance round (measured with interleaved `perf stat`, output
  byte-identical in every case):** whole-net identity buffers (`assign y = x`)
  now collapse onto their source by default (`XEZIM_BUF_COLLAPSE=0` opts
  out) — the pass leaves alone any net that is a `force`/`release`/procedural
  `assign` target, any source a process writes (the copy's delta step stays
  observable), gate-driven nets, 2-state/4-state pairs, SDF designs, and
  designs with DPI/VPI libraries; C906 memcpy runs 10.7 % fewer instructions
  and 14 % less wall time, bit-exact against the reference transcript. On UVM
  workloads the runtime scalar-index helper no longer hands calls and member
  accesses to the elaboration-time constant folder (which cloned the whole
  function table per attempt), process contexts are moved rather than cloned
  across wakeups, and clocking blocks poll their clock by signal id; the axi4
  AVIP base test retires 3.7 % fewer instructions.
* **Default timescale for untimed units is `1ns/1ns`** for any module,
  interface, or package without a `` `timescale `` directive (IEEE 1800
  §3.14.2.2 leaves the default tool-defined; this matches the reference
  simulator). Previously an untimed unit reported `1s/1s` while its delays
  counted the design's global tick; now `#1`, `$time`, and `$realtime` all agree
  on nanoseconds and `--dump-timescales` flags every defaulted unit. Pass
  `--module-timescale` to pick a different default.
* **Covergroups declared inside classes work** (§19.3): the class-body
  covergroup is registered, the implicit variable it declares exists, `cg = new`
  in the constructor instantiates it, `cg.sample()` reads the object's
  properties (also when sampled from outside through `obj.cg`), a derived class
  that redeclares `cg` gets its own coverpoints, constructor formals
  (`covergroup cg (int lo, int hi)`) reach the bins, `with function
  sample(...)` formals are bound per call, `option.auto_bin_max` (coverpoint or
  covergroup level) and `cg::type_option.<field>` are honoured, and
  `$get_coverage()` reports the mean over covergroup types. Covergroup and class handles no longer share one
  integer namespace, which had dispatched class object 1 as covergroup 1.
* **DPI at compilation-unit scope** (§35.5.4): `import "DPI-C"` and
  `export "DPI-C"` written at the top of a file are visible in every module,
  like a `$unit` function; they used to be reported as undeclared. Small
  integral returns (`byte unsigned`, `shortint`) read at their declared width
  and sign in expressions, and a 1-bit `logic` argument carries x/z as svLogic.
* **A real assigned to an integral subroutine local rounds** (§6.12.2), as it
  always did for module variables; the local used to keep the real value, so
  `int div = freq / rate;` compared as 32.55 forever and a baud-clock divider
  written that way never toggled.
* **Concurrent assertions inside instantiated modules and interfaces** are
  registered and fire; inlining used to drop them silently. Sequence
  consequents (`a |-> a ##1 b ##1 c`, `a |=> s`) walk their steps cycle by
  cycle, named sequences with unclocked bodies expand, and `cover property`
  is tallied as cover with misses not counted as failures.

### 0.10.4 — power intent, packed-struct codegen, opt-in waveforms (September 2026)

* **Packed-struct member assignments compile** instead of falling back to the
  AST interpreter. `s.m`, nested `s.p.m` (and every `union`-in-struct form),
  `arr[i].m`, an assignment pattern into an array element (`arr[i] <= '{...}`),
  and a function whose body is `return '{...}` were all interpreted at roughly
  3.8 µs per statement. On a struct-payload pipeline benchmark — 8 lanes × 3
  stages of an 88-bit struct, 20k cycles — this took the run from **16.05s to
  0.83s**, reference-exact throughout. Neutral where the shape is absent
  (Ibex is instruction-identical).
* **Streaming concatenations and 2-D array stores compile.** `{>>{…}}` and
  `{<<N{…}}` lower to constant range selects plus one concat instead of the
  AST interpreter (a byte swap written `{<<8{x}}` was ~32% slower than the
  same swap written by hand; it is now within 7%). A store to a 2-D unpacked
  element (`a[i][j] <= v`) reuses the row-major flat index the read path
  already had — a 4×4 array written element-wise every cycle went from 1.92s
  to 0.16s (**12×**), and a loop containing one no longer drops to the AST
  path wholesale. The 1-D memory case always compiled.
* **Mailbox and semaphore ARRAY elements allocate on `new()`.** `mb[i] = new()`
  stored a live-looking handle with nothing behind it, so every `put` silently
  vanished, `num()` stayed 0 and `try_get` always failed, while the same
  mailbox declared as a scalar worked. Fixed for every lvalue shape: module
  scope, inside a class method, through `this.`, through a class handle, and
  in associative / dynamic / queue / multi-dimensional collections.
* **Waveform dumping is opt-in** via `--wave` (see Features). An active dump
  forces loops that would otherwise compile onto the AST path and builds a
  per-signal trace table, so a run that never dumps no longer pays for it, and
  a design that calls `$dumpvars` no longer starts writing a file
  unannounced. `--fst`/`--xtrace` imply it, so existing command lines are
  unchanged.

* **IEEE 1801 power intent** via `--upf` / `--upf-top`: supply nets with
  state and voltage, power switches, corruption of powered-down elements,
  isolation clamps and retention, driven from the testbench through the
  standard `UPF` package (`supply_on` and friends). Multi-file intent chains
  with `load_upf -scope`, `-update` merges into a named strategy, and
  `-elements {.}` names the scope instance. `examples/upf/` is a runnable
  example; see "Power intent (UPF)".
* **`release` inside a level-sensitive block** (`always @(en)`, `@*`, or a
  process resumed by `@(en)`) now returns the net to its continuous drivers
  immediately. It used to keep the forced value until the driver happened to
  change again, because the re-drive was lost inside the settle pass.
### 0.10.1 – 0.10.3 — native compilation and process conformance (August 2026)

* **AOT native backend** (`XEZIM_JIT=1 XEZIM_AOT=1`, needs a `--features jit` build) — the
  compiler emits Rust for eligible combinational entries, edge blocks, and
  process FSMs, builds it with `rustc`, and loads it through a single exported
  API symbol. On the C910 CoreMark run 18,916 / 21,305 edge blocks and
  108,248 / 215,494 combinational entries compile natively.
* **Persistent native cache** — the generated library is keyed on the source,
  optimization level, and xezim build, then stored under
  `$XEZIM_CACHE_DIR` / `$XDG_CACHE_HOME/xezim/native` / `~/.cache/xezim/native`.
  The first run pays the `rustc` cost; later runs load the cached `.so`
  directly. `XEZIM_NO_NATIVE_CACHE=1` opts out.
* **Compiled process FSMs** (`XEZIM_PROC_FSM=1`) — a blocking `always` body
  compiles into a bytecode state machine with explicit wait instructions, so a
  resume re-enters at the saved program counter with per-process registers
  instead of re-walking the AST continuation chain. Blocking tasks and
  `initial` blocks inline into the same FSM, and the FSMs themselves are
  eligible for the native backend.
* **Blocking task calls inside clocked blocks follow process semantics**
  (§9.2.2) — an `always @(posedge clk)` body that calls a task which consumes
  time is no longer executed on the fast edge path. Previously the callee's
  delay advanced simulation time while that slot's non-blocking updates were
  still queued, so a `q <= d;` scheduled before the call committed a few
  picoseconds late; edges arriving mid-call were also mishandled. Such blocks
  now run as processes: NBAs commit in their own slot, and edges that arrive
  while the body is busy are missed, matching the reference simulator.
* **Delays quantize at the declaring scope's precision** (§3.14.3) — a constant
  fractional delay such as `#0.002` is folded and snapped to the precision grid
  of the scope that declares it, at elaboration time, including delays inside
  interface and class methods.
* **`bind` by instance path** (§23.11) — `bind top.u_dut.u_sub tb u_tb();` and
  the colon form specialize only the named instances; module-name binds are
  applied before path binds, and upward references from the bound module
  resolve against the instance it was bound into.
* **Opt-in combinational region fusion** (`XEZIM_REGIONS=1`) — dependency-
  connected compiled entries fuse into topologically ordered region blocks.
  Measured net-negative on the current benchmark set (recompute cost outweighs
  the dispatch saving), so it ships off by default and stays available for
  experiments.

### 0.10.0 — waveform integrity, cocotb, and class-storage fixes (August 2026)

* **FST dumps are correct at scale** — a break-even compression case wrote the
  time table raw while flagging it compressed, corrupting large dumps in
  GTKWave; the writer now records what it actually wrote, dumps are finalized on
  `Ctrl-C`, the final time slot is flushed, and values render on the writer
  thread instead of the simulation thread. Cross-format agreement is checked by
  decoding VCD, FST, and XTrace, not by comparing file sizes.
* **cocotb backend** — Python testbenches run against xezim via
  `contrib/cocotb/xezim_runner.py`, backed by VPI timed and synchronous
  callbacks and a repaired VPI object model.
* **Scheduling-region fixes** — the postponed region is serviced from the nested
  event loop and on livelock recovery, and a delay closes out the slot it
  resumed in, which removed a `$monitor`-vs-waveform timestamp skew.
* **Class and scope storage** — class member arrays resolve through the runtime
  class, `localparam` arrays inside classes elaborate, each instance gets its
  own static task local under non-blocking assignment, packed-struct formals
  read their members from the call frame, and `%m` no longer leaks the scope of
  a suspended task.

---

# What's new in 0.9

### 0.9.8 — reference-parity audit campaign (August 2026)

Dozens of differential test batteries were run against a commercial reference
simulator; every divergence found was measured construct-by-construct, fixed,
and pinned with a regression test citing the LRM section:

* **Per-evaluator continuous-assign propagation is now the default** (#35) —
  combinational updates propagate with LRM evaluation ordering instead of a
  single batched settle, resolving process-observation orderings that were
  previously unattainable with either batching mode. Escape hatches:
  `XEZIM_EAGER_PROC_SETTLE=1` (previous default) and `XEZIM_LAZY_PROC_SETTLE=1`.
* **UVM `run_test()` termination** (#109) — the phase scheduler advances time
  through run-phase objections; live regression pins run the real Accellera
  library (1.2, 1800.2-2017, 1800.2-2020) in every CI gate.
* **Package export semantics** (§26.6) — `export P::*`, `export P::sym` and
  `export *::*` are honored: a wildcard import re-exposes a package's own
  imports only when exported, and a wildcard export covers only names the
  exporting package references — unexported/unreferenced names are rejected
  exactly as the reference rejects them.
* **Implicitly-static initializer legality** (§6.21) — a local variable with an
  initializer in a static-lifetime task/function is now a compile error
  (explicit `static`/`automatic` required), matching reference behavior;
  for-header declarations, block locals and class methods stay accepted.
* **`alias` as true net unification** (§10.11) and `trireg` charge storage
  (§6.6.4) — aliased nets share one signal slot rather than lowering to an
  assign cycle.
* **Cycle delays synchronize** (§14.11) — `##0` (and a runtime `##(n)` that
  evaluates to 0) waits for the default clocking event when off-edge and is a
  no-op at the edge; `##n` without a designated `default clocking` is rejected.
* **Formatting parity** (§21.2.1.7, §21.2.1.3) — associative arrays print with
  the reference's `'{k:v, ... }` spacing; explicit-width `%h`/`%b`/`%o`
  zero-pad to the minimal core without truncation.
* **Array-method iterators** (§7.12) — `q.sort(x) with (x)` binds the declared
  iterator (sorts and `with`-reductions no longer act on zeros); event controls
  on packed-struct fields (§9.4.2) arm the base vector with a field-value
  compare instead of waking spuriously.
* **`ref` formals alias the actual** (§13.5.2) — callee writes are visible to
  parallel observers mid-call, observer writes reach the callee, and the
  element identity of `ref arr[i]` is frozen at call time.

* **UVM 1800.2-2020.3.1 runs green** — the reference testbench passes against the
  2020.3.1 library (`UVM_ERROR : 0` / `UVM_FATAL : 0`, in/out monitors agree).
  Closing this required a general preprocessor fix (inline
  `` `ifdef ``/`` `endif `` mid-line, §22.6), class-body `localparam` constants,
  and sequencer-path fixes (`process::self()`, fork/join_none automatic-variable
  sharing).
* **User-defined nettypes** (LRM §6.6.7) — `nettype` declarations with
  user resolution functions, Z-skip, and built-in resolution.
* **Per-module timescales** — `$time`/`$realtime` scale to the calling module's
  unit; `timeunit`/`timeprecision` declarations scale delays; `$timeformat`/`%t`
  and `$printtimescale` honored; sub-ns precision down to `fs`; new
  [`--module-timescale`](docs/user-guide.md#module-timescale-extension) CLI extension for
  legacy RTL with no source-level timescale.
* **String & aggregate conformance fixes** — `s[i]` read/write on string
  variables (§11.4.13), `ref`/`output` queue arguments copy back on return
  (§13.5.2), `%p` renders function-local queues/associative arrays (§21.2.1.7),
  `foreach` over a string iterates its content length, `q = {}` clears string
  queues, and a never-touched module-scope queue reports `size() == 0`.
* **Free functions no longer see the caller's class context** (§13.4) — a bare
  name in a package/module function that collided with a caller class property
  used to silently alias the property; queue-property access from outside the
  class (`obj.q.push_back(x)`, `%p` of `obj.q`) now resolves correctly.
* **Gate-level & structural robustness** — multi-dimensional packed arrays of
  unpacked elements (`arr[i][j]`, §7.4), `foreach` over negative/descending and
  packed dimensions (§12.7.3), non-ANSI ports completed by a `reg`/`logic`
  declaration (§23.2.2.1), and per-iteration uniquification of declarations
  inside **nested** generate-for loops (`for(a) for(b) localparam Idx = f(a,b)`).
* **Behavioral clocks & PLLs** — a clock generator whose delay reads a runtime
  variable (`always #(half) clk = ~clk`) now re-evaluates its period every
  toggle, so a PLL reprogrammed at runtime actually changes frequency; verified
  against a commercial simulator alongside UDP primitives, tristate/pull
  strengths, `specify`/timing-check, and divider chains.
* **Dead-clock watchdog** — `XEZIM_STUCK_CLOCK` flags a process parked on a
  clock/reset that never changes while the design keeps churning edges (an
  undriven-net / dropped-cell hang), turning a silent multi-minute grind into an
  immediate, actionable diagnostic (`warn` by default; `abort` for CI).

---

# Verified Workloads

End-to-end TEST PASSED with bit-identical results vs the workloads' own
golden expectations:

| Design | Test | sim_time / cycles | baseline wall | +O1 wall |
|---|---|---|---|---|
| XuanTie C910 (dual-core) | hello | sim_time 44695 | 95s | **73s** (1.30×) |
| XuanTie C910 | memcpy ×7000 | sim_time 101965 | 216s | **166s** (1.30×) |
| XuanTie C910 | cmark ×1 (`+iterations=1`, INIT_ZERO=1) | 167124 cycles | 87 min | **73 min** (1.19×) |
| XuanTie C906 (single-core) | memcpy ×50 | — | 99s | **88s** (1.13×) |
| XuanTie C906 | cmark ×1 (INIT_ZERO=1) | 295294 cycles | 714s | **587s** (1.22×) |
| riscv-dv (UVM 1.2) | `+num_of_tests=10` random RV32IMC | — | — | 10/10 assemble clean |

Larger runs measured during the 0.10 campaign:

| Design | Test | Result | wall |
|---|---|---|---|
| lowRISC Ibex (`simple_system`) | CoreMark ×10 | score 2.477304 CoreMark/MHz, 2,765,321 instret, halt at 41,454,505 ns — byte-identical | 447s |
| XuanTie C906 | cmark ×2 | TEST PASSED, 286,469 cycles/iteration | 516s |
| XuanTie C910 (dual-core) | cmark ×2 | TEST PASSED, CoreMark 6.327752, halt at 34,985,250 | 8,028s, including a cold native compile of the whole design |
| mbits-mirafra AVIP suite (UVM) | apb / spi / i3c / axi4 / axi4Lite / uart base tests | 6 of 6 reproduce the reference's `UVM_ERROR` counts and end times, run unmodified with no `--module-timescale` (the untimed BFMs take the `1ns/1ns` default; uart alone needs `--module-timescale 1ps/1ps`); `ahb` runs in xezim but the reference fails to elaborate it | 33s for axi4Lite (28s with FSM + AOT), about 60s for uart, seconds for the rest |

On these CPU workloads a commercial reference simulator is still roughly
4–5× faster; the campaign narrowed the Ibex CoreMark gap from about 30× to
4.3×. **Where the remaining cost sits depends on the design**, and the two
cores profile as opposites:

* **C906 is scheduling-bound.** Running the reference with its optimizer
  disabled (321s) against optimized (77s) and xezim (489s) puts ~4.2× on its
  optimizer and only ~1.5× on the kernel itself, and a symbol profile spends
  ~34% of the run evaluating the design against ~22% deciding what to
  evaluate. Every net stays externally visible, so each combinational result
  is published and its readers notified — the cost the reference's optimizer
  removes by keeping intermediate nets in registers.
* **Ibex is evaluation-bound.** ~62% of the run is in the bytecode
  interpreter (`exec_insns` alone is 38%) against ~22% scheduling. It has
  1,553 combinational entries to C906's 35,267, so the same work is spread
  over ~23× fewer, ~37× hotter blocks.

That split is why native compilation is opt-in rather than default: it is
worth ~23% on Ibex and a net loss on C906 (see **Native compilation**).

The picture is design-shape dependent, and the benchmark set above — all
CPU cores and class-based UVM — under-represents struct-heavy modern RTL. On a
struct-payload pipeline microbenchmark (8 lanes × 3 stages of an 88-bit packed
struct written member-wise, 20k cycles) xezim runs it in 0.83s against the
reference's 59.5s. That is a microbenchmark, not a workload, but it is the
shape the table above contains none of.

UVM run-phase (see [docs/uvm-guide.md](docs/uvm-guide.md)):

| Testbench | Result |
|---|---|
| GettingVerilatorStartedWithUVM vs **1800.2-2017** (`data0`/`data1`/`random`/`many_random`) | 4/4 — exact Verilator parity (monitors agree, `UVM_ERROR`/`UVM_FATAL` = 0) |
| GettingVerilatorStartedWithUVM vs **1800.2-2020.3.1** | green — in/out monitors agree (77/77 packets), `UVM_ERROR`/`UVM_FATAL` = 0 |
| sv-tests UVM 1800.2-2017 example suite | 32/35 pass (3 out of scope: deprecated UVM-1.0 macros, DPI backdoor) |

---

# Compliance

Full [sv-tests](https://github.com/chipsalliance/sv-tests) run with the
suite's own `xezim` runner (`make report RUNNERS=Xezim`), xezim 0.8.1. The
generated HTML report and per-test CSV are checked in under `reports/`
(`svtests_index.html`, `svtests_report.csv`, and `sv-tests-compliance.md`).

| Category | Pass / Total | Rate |
|---|---|---|
| **All tests** | **4354 / 4768** | **91.3 %** |
| &nbsp;&nbsp;UVM (1800.2-2017) | 484 / 487 | 99.4 % |
| &nbsp;&nbsp;non-`ivtest` | 2153 / 2237 | 96.2 % |
| &nbsp;&nbsp;Icarus `ivtest` suite | 2201 / 2531 | 87.0 % |

An earlier run scored only 52 % because a `-I` library directory
(`ivtest/ivltests/`, ~1000 mutually independent single-file tests) was scanned
too eagerly: xezim honors IEEE §23.3.2 library semantics — an `-I` dir supplies
module definitions to satisfy unresolved instantiations — but it was adopting
*every* definition in the directory, so typedefs/enums from unrelated sibling
files leaked into the primary design and failed a spurious §6.18 base-type
check. `resolve_library_modules` now pulls in only the library modules actually
reachable from the compiled design (transitively), which reclaimed ~1870
`ivtest` cases with no change to the native LRM/UVM results.

---
