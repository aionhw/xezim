# xezim technical notes

Release notes, verified workloads and compliance results. The user guide is
in [docs/user-guide.md](docs/user-guide.md); building and contributing are in
[docs/building.md](docs/building.md) and [CONTRIBUTING.md](CONTRIBUTING.md).

# What's new in 0.11

### Unreleased

**Command line**

* `XEZIM_OOM_TRACE=memory.jsonl` streams phase, source, instance, table-count
  and resident-memory records during compilation and simulation. Periodic
  samples preserve the active work when a process is killed (#721).

**Correctness**

* Hierarchical references reach static task/function locals before their first
  call and preserve instance-specific storage (§6.21, §23.6).
* Preprocessor warnings preserve the run's exit status when stderr closes
  (§20.10, #1037).
* Typed constructor arguments use the caller's parameters; defaults and
  initializers use the constructed class's parameters (§8.8, §13.5).

* Typed constructor calls build the named class, including type parameters
  and specializations (§8.8, #1051).

* `randomize()` propagates aligned packed-concatenation equalities field by
  field and establishes unconditional hard bounds before translating bounded
  wide arithmetic. Satisfiable relations could exhaust the search budget or
  be rejected depending on constraint order (§18.5, §11.4.12, #1003).

* An index naming a class property selects the virtual interface it names:
  `dst[j] = src[j]` with `j` a property, `@(posedge vifs[idx].clk)`,
  `vifs[n-1]`, `vifs[cfg.idx]` and a static index copied a null handle or
  armed the wait on nothing, so the wait returned at once and its loop spun at
  one time step (§25.9, §9.4.2, #988, reported by Ganesh-TNL).

* `@(vif)`, `@(vifs[0])` and `@(cfg.vif)` on a virtual-interface variable wait
  for it to refer to another instance; they never resumed, or returned at once
  (§9.4.2, §25.9, #988, reported by Ganesh-TNL).

* An edge on a select of a class property (`@(posedge bus[1])`), on a property
  of a handle-array element (`@(posedge subs[1].clk)`), and an `or` list
  mixing a signal with a class property (`@(posedge clk or posedge h.rst)`)
  wait for their edges; they spun at one time step, or at the head of a
  `forever` loop missed the property's edge (§9.4.2, #993, reported by
  Ganesh-TNL).

* `randomize()` solves relations that read a signed variable unsigned
  (`row_bytes == line_width * 8` with an `int row_bytes`, `byte` or `shortint`
  sizes, `x < 32'h8000_0010` on an `int x`), products of two variables (`start
  + cnt * pitch - 1 <= top`), a wrapped side bounded by an open one
  (`(start_addr - 8'h70) <= top - row_bytes`), and `if`/`->` branches that
  fail only once enforced; these satisfiable classes returned 0 (§11.8.2,
  §11.4.3, §18.5, #994, reported by Ganesh-TNL).

* A signed member of a packed struct class property reads signed, a struct
  member's signedness decides a relation's (`m.s + 4'sd1 < l.s + b.u`
  sign-extended its left side), and `randomize()` solves `p.s inside
  {[-4:-1]}` (§7.2.1, §11.8.2, §18.5, #994, reported by Ganesh-TNL).

* A packed struct member that is a packed array of a typedef element (`t3_t
  [3:0][7:0] f0`), or whose dimensions live in its typedef, selects an element
  at every index on variables, ports, locals and formals, and an x/z index
  into it in compiled code reads x at the element width (§7.2, §7.4.1,
  §11.5.1, #984, reported by Ganesh-TNL).

* A non-ANSI port of a packed struct type records its members' packed shapes,
  so `arg.vec[1]` selects a byte on the port and in a callee whose formal has
  the port's name (§23.2.2.1, §7.4.1, #984, reported by Ganesh-TNL).

* An element select of a packed-array member of a call result (`set_td(a, i,
  j, k).f0[1][1][1]`) selects by the member's dimensions instead of a bit
  (§13.4.1, §7.4.1, #984, reported by Ganesh-TNL).

* `$size`, `$left`, `$right`, `$high`, `$low`, `$increment` and `$dimensions`
  of a select that leaves several packed dimensions (`$size(a3[1])`) describe
  the dimensions left (§20.7, §7.4.5, #984, reported by Ganesh-TNL).

* A path from a nested generate block through an enclosing one names the
  enclosing block's declaration when both declare the name (`g.x`, `g.f()`,
  `g.t`, `disable g.t`): the call recursed until the stack overflowed and the
  read gave the nested block's value. An interface or module instance of a
  generate block can be a value or path head in the block (a virtual interface
  handed to a class, `u.a[i].f = ...`), where it was undeclared, and `-> ev`
  in a method of a class declared in the block fires the class's own event
  (§23.6, §23.9, §25.9, §15.5.1, core #184 / #989, contributed by philipaxer).

* A generate loop's genvar reads its iteration's value in properties,
  sequences, covergroup bins, gate terminals, formal defaults, typedefs and
  the other items of the loop body; a formal, local, class member or property
  formal of the same name hides it (§27.4, §13.5, §8.3, §16.8, core #185 /
  #990, contributed by philipaxer).

* `disable lab.sub` of another process's block, from inside a nested `begin
  ... end`, stops that block and lets the nested block go on; it ended the
  disabling block too. `disable l[1].lab.sub` stops that iteration's block,
  not the first `sub` found (§9.6.2, #952).

* An unpacked array of unpacked structs declared in a generate block stores
  its elements member-wise (`g.q[2].f` read x), and `%p` prints an element or
  nested member of a generate block's variable (`q[2]`, `o.in`, `l[1].t[1]`)
  from its declared type (§7.2, §21.2.1.7, §27, #952).

* An assignment pattern written to a row of a multi-dimensional unpacked array
  assigns each element of the row, in a blocking store and in a continuous
  assignment: `c[0] = '{V, V}` on `logic [39:0] c [2][2]` and `assign c[0] =
  '{a, b};` left the row x. Rows of struct elements are spread too (§10.9.1,
  §10.10, §7.6, §10.3, #999, contributed by chuanseng-ng).

* A nonblocking store, a continuous assignment or a delayed nonblocking store
  to a packed struct element of an unpacked array writes the whole struct:
  `b[0][1] <= v` on `le_t [1:0] b [2]` kept one bit, and a row of two structs
  kept two (§7.4.1, §7.4.5, §10.4.2, §10.3, #997, contributed by
  chuanseng-ng).

* A packed element selected inside an unpacked array's element reads right in
  compiled code: `d[1][0]` on `logic [1:0][7:0] d [2]` is the low byte of
  `d[1]`, for queues, dynamic arrays, run-time indices and arrays of packed
  structs. It read `0X` or one bit. A three-dimensional array element at
  run-time indices read `000X`, and an x or z literal index read element 0
  instead of x (§7.4.1, §7.4.5, §11.5.1, #985, contributed by chuanseng-ng).

* `always_comb` and `always @*` follow a whole read of an unpacked struct that
  is a continuous copy of another (`assign q = p; always_comb c = q;`), of an
  element of an unpacked struct array (`c = pa[0]`) and of the whole array;
  they missed every change after time zero (§9.2.2.2, §9.4.2.2, §7.2, #991,
  contributed by ds54e).

* A store to an element of a multi-dimensional packed member of a struct array
  element (`sa[1].f0[3][7] = v`) was dropped, a nonblocking one kept one bit,
  and the same select in compiled code took one bit (§7.4.1, §7.2, §10.4.2,
  #984).

* An assignment pattern on a whole multi-dimensional unpacked array whose
  element is a multi-dimensional packed type assigns every element: `n =
  '{default: 80'h0}` on `logic [1:0][39:0] n [2][2]`, the same declaration
  initializer, and positional, keyed, nested and nonblocking patterns left
  every element x (§10.9.1, §7.4, §6.8, #1013, contributed by chuanseng-ng).

* Header parameters and ports hide wildcard-imported names throughout the
  module body; package functions and classes retain their own parameter values
  (§26.3, #995, reported by ds54e).
* Finite zero-delay phases stay in their timestamp through deferred events,
  mailboxes, semaphores and edge relays. Stall checks include local data and
  suspended loop control, while explicit limits retain their strict behavior
  (§4.4, #996, reported by Ganesh-TNL, contributed by Thomasb81).
* Collection call results and concatenations bind element by element to fixed
  array and queue formals, with per-call storage, reference aliases and staged
  output writeback. Handle members, package arrays and class-scoped associative
  queue elements preserve their collection contents (§10.10, §13.5,
  #1001, #1004, #1006, #1008, #1018, #1028, reported by Ganesh-TNL and
  auty1395, contributed by auty1395).
* Unpacked struct concatenations retain nested members, strings, real values
  and collection fields, including when the destination also appears in the
  concatenation (§10.10, #1035, reported by auty1395).
* Chained packed typedef indices retain their declared dimensions in compiled
  locals and members of selected unpacked-array elements (§7.4.1, #1002,
  reported by Ganesh-TNL).
* Whole multidimensional unpacked-array patterns spread across packed-array
  elements. Loaded artifacts retain parameter-keyed patterns and lexical struct
  widths, and prepared continuous assignments retain falling and turn-off delays
  (§10.9.1, §20.6.2, §10.3.3, #1013, #1014, #1015, #1016, reported by
  chuanseng-ng and ds54e, contributed by chuanseng-ng).
* Randomization preserves class typedef and specialization handles, solves
  sub-object back references jointly, and draws rand members of dynamic unpacked
  struct arrays. Concatenated `$countones` operands and inline caller-scope
  `foreach` constraints participate in the solve (§18.4, §18.5.9, §18.7,
  §20.9, #1010, #1020, #1022, #1024, #1030, #1034, reported and contributed
  by auty1395).
* Covergroups sample implicit cross operands in their owning instance's scope
  (§19.6, #1026, reported and contributed by auty1395).
* Clocking output edge skews drive on the declared edge. Aliases keep their
  clockvar identity and distinct pending drives survive checkpoint restart
  (§14.3, §14.16.2, #1032, core #186, reported and contributed by auty1395).

* Sparse integral constraints propagate through packed concatenations, squares,
  constant division and remainders, split packed fields and wide arithmetic.
  Wide array reductions and arrays beyond 8,192 elements join the solve, and
  indexed record fields retain their own dependencies (§18.5, §11.4.3,
  #1003, reported by Ganesh-TNL).


* Nested generate labels resolve within the declaring block and instance,
  including child functions and indexed generate arrays after cache or artifact
  reloads (§23.6, §23.8, §27.6, core #187 / #188, reported and contributed by
  chuanseng-ng).
* An inlined function hides its caller's loop constants while executing its
  body, then restores the caller's bindings before copying output arguments
  back to local variables or indexed destinations. Nested returns resume the
  innermost caller before output writeback (§13.4, §13.5.3, #1041 / #1042,
  reported and contributed by chuanseng-ng).
* Explicit static locals of automatic tasks and void functions retain shared
  state across clocked calls, nested blocks and conditional updates. Automatic
  and write-first locals keep their ordinary behavior (§6.21, #1049).
* `$dumpflush` waits until the threaded VCD writer has flushed preceding
  records, so another reader can see them during simulation (§21.7.1.6,
  #1047 / #1048, core #189, reported and contributed by ds54e).
* Closing stdout or stderr early preserves the run's exit status and waveform,
  including library and core diagnostics for runtime failures (§20.10, #1037 /
  #1038, reported and contributed by chuanseng-ng).
* Method formals written without a type keyword (`input [W-1:0] v`) keep
  their declared width, including class specializations, defaults and narrow
  expression actuals. Compiled results retain their specialized width, and
  packed-formal `foreach` loops use the bound width
  (§13.5.1, §11.8.2, §12.7.3, #1044, reported by Ganesh-TNL).
* Assignment patterns write unpacked-array slices in declared order, including
  dynamic arrays and class properties. Each item takes the element type, and
  all source values are read before the selected elements are overwritten
  (§7.4.5, §10.9.1, #1045, reported by Ganesh-TNL).

* Nets inside module instances resolve their drivers as top-level nets do: a
  drive strength on a continuous assignment or a net declaration assignment is
  kept, `%v` reports it, and several bit or part-select drivers of the same
  bits are all resolved (§6.5, §6.7, §10.3.4, §28.12, #1062 / #1063, reported
  by philipaxer, contributed by Thomasb81).

* An unsized `'0`, `'1`, `'x` or `'z` fills every bit of a net resolved by
  strength, as a weak or pull driver and as the released value of a strong
  one, on nets wider than 64 bits too (§5.7.1, §28.12, #1061, reported by
  philipaxer, contributed by Thomasb81).

* A tri0, tri1 or supply port merges with the net connected to it: the net
  outside pulls as the port does, with or without a driver, also on a bit or
  part select, across an instance array and when the net has the port's name.
  A tri0 or tri1 net driven only through selects pulls the bits they release
  (§6.6.5, §23.3.3.7, #1060, reported by philipaxer, contributed by
  Thomasb81).

* An `interconnect` array takes the types of the ports bound to its elements,
  so a `real` nettype element carries its value, and an interconnect bound to
  a struct nettype port, whole or as an array element, keeps the struct's
  members (§6.6.7, §6.6.8, §23.3.3.7.1, #1039 / #1040, reported by ds54e,
  contributed by Thomasb81).

* A `disable`, `break` or `continue` run in a `forever` body before the body's
  delay ends the iteration there. `disable` of a process's own top-level
  block, an `always` body or a named `forever` body stops that block, and `%m`
  in such a body names it (§9.6.2, §12.8, #1068, reported and contributed by
  chuanseng-ng).

* Each driver of a net applies its own rise, fall and turn-off delays before
  the net resolves them. A primitive's fall delay used to delay the whole net,
  so `buf #(1,5) g(w,a); assign #(1,1) w=a;` read 1 where the drivers
  disagreed (§28.16, §28.12, #1065, contributed by ds54e).

* Delayed tristate and MOS primitives that share a net are resolved with the
  net's other drivers; the net read as the last such primitive alone,
  undelayed. A primitive with a zero rise delay (`buf #(0,5)`) falls after its
  fall delay (§28.6, §28.7, §28.16, #1065).

* A UDP output that shares its net with a continuous assignment or another UDP
  is one driver of the resolved net; the last writer used to win (§29.3,
  §28.12).

* A whole-variable nonblocking assignment after a partial one to the same
  variable in the same time step wins: `bus.vld <= 1'b0; bus <= PKT;` in a
  task, initial block or class method left `vld` at 0. This covered struct
  members, bit-, part- and indexed part-selects, concatenations, and
  interface, package, generate, submodule and virtual-interface targets. A
  queued partial update also keeps a blocking write to the variable's other
  bits (§4.6, §10.4.2, #1050, reported and contributed by Ganesh-TNL).

* A nonblocking assignment to a class property is sized by its declared type.
  Through a handle or the class name, a property wider than 32 bits kept only
  its low 32 bits, a `real` one (also through a type parameter) took an
  integer, a `super.` one was dropped, and one sharing a module signal's name
  wrote that signal (§10.4.2, §8.5, §6.12, #1050, reported and contributed by
  Ganesh-TNL).

* A reference through an interface modport (`ci.drv.bus_a`) names the
  instance's own variable; writes went to a copy that reads did not see
  (§25.5, #1050, reported and contributed by Ganesh-TNL).

* A member of a packed-struct static class property (`C::sp.vld`, `h.sp.addr`,
  `sp.hdr.tag` inside the class) reads the static's value; it read 0 (§8.9,
  §7.2.1, #1050, reported and contributed by Ganesh-TNL).

* `wait fork` no longer waits for a grandchild whose parent finished before
  it, including in DPI-exported tasks; `disable fork` and `kill()` still
  terminate it (§9.6.1, §9.6.3, §9.7, #1072, reported and contributed by
  chuanseng-ng).

* A packed array or packed struct compared with a bare assignment pattern (`a
  == '{2,2,2,2}`, `ps != '{lo:1, default:2}`) takes the pattern in the
  operand's type and compares the two packed values bit for bit, from every
  declaration form: a variable, a packed member or element of an unpacked
  aggregate, a class property (also through implicit `this`), a function
  result or formal, a plain vector, a parenthesized operand, a continuous
  assignment and a concurrent assertion. The pattern was concatenated at its
  items' 32-bit widths, so the comparison always reported unequal (§10.8,
  §10.9, §11.4.5, §11.4.6, #1043, reported by Ganesh-TNL, contributed by
  Thomasb81).

* `===` and `!==` on unpacked arrays, queues, dynamic and associative arrays
  and unpacked structs compare the elements' 4-state bits; they answered
  "exactly equal" for any two same-shaped arrays (and "not equal" for
  associative arrays). `==?`/`!=?` compare unpacked arrays and structs
  element-wise, and `==`/`!=` compare module-level associative arrays by
  content (§11.4.5, §11.4.6, §7.8, #1043, reported by Ganesh-TNL, contributed
  by Thomasb81).

* `==` and `!=` on unpacked arrays and structs are x when x or z elements
  leave the relation open, and a differing element decides wherever it sits;
  both answered 0, and an element differing after an x one was ignored
  (§11.4.5, #1043, reported by Ganesh-TNL).

* An element select of a packed-array member of a packed struct reached
  through a class property (`h.frame.lanes[0]`, `frame.lanes[0]` in a method,
  `arr[i].lanes[0]` of an array property) reads and writes the element;
  indexed and constant part-selects (`lanes[0 +: 2]`, `lanes[1:2]`) address
  lanes, not bits; `super.frame.lanes[0] = v`, `foreach (frame.lanes[k])` and
  an ascending inner member (`g.pairs[0].pair[0]`) select the declared
  element. The select read one bit and a write through a typedef-typed member
  hit the wrong bits (§7.2.1, §7.4.1, §11.5.1, §12.7.3, #1046, reported by
  Ganesh-TNL, contributed by Thomasb81).

* An explicit import of a package nettype (`import p::N; N w;`, in a module
  body, a module header or at compilation-unit scope) declares nets of that
  nettype; elaboration stopped with "Symbol 'N' not found in package 'p'". The
  resolver named after `with` is the package's function, even where the
  importing module declares a function of the same name, a typedef or nettype
  of the same name in another package no longer takes the imported nettype's
  place, and a nettype that two wildcard-imported packages declare is an error
  where it is used (§26.3, §6.6.7, #1083, contributed by ds54e).

* A delayed continuous assignment to a vector net takes the rise delay when
  its value changes to x or to a partly-z value, as for any change other than
  one to 0 (fall) or to z (turn-off): `assign #(4,2) w = x;` changed `w` after
  the smallest delay, the gate rule §10.3.3 keeps for a scalar target. This
  covers net declaration assignments, `[0:0]` vectors, packed-struct nets and
  a select another driver overlaps (§10.3.3, §28.16, #1076, contributed by
  ds54e). A whole vector variable follows the same rules, and an assignment to
  a part-select or bit-select that no other driver overlaps, and a primitive
  whose output is a bit-select (`and #(4,2) g(w[2], a, b);`), apply their fall
  and turn-off delays; they took the rise delay for every change. Disjoint
  select drivers of one net that change in the same step keep their own
  delays: `assign #2 d[3:0] = a; assign #5 d[7:4] = b;` changed both halves
  after 2 (§10.3.3, §28.16, #1076).

* A warm `--cache-dir` run starts continuously driven nets at x before their
  drivers' first update, as an uncached run does; with the prepared tables
  from the cache they started at z, so `assign #2 w = a;` read z at t1 and a
  zero-delay loop read z for the whole run (§6.7.1, #1079, contributed by
  ds54e).

* A continuous assignment of a structure pattern gives an unpacked array
  member every element its nested pattern sets by `default:`, by index key (a
  parameter name too), by type key or by replication (`assign v = '{tag: x, a:
  '{default: x}};`), and a structure-level `default:` or type key reaches the
  members and elements of nested arrays and structures, through a child
  module's output port and on nets of a user nettype with or without a
  resolver. The array member read 0, or x on a nettype net, and a replication
  set only its first element (§10.9.1, §10.9.2, #1081, reported by ds54e).

* A `defparam` cannot set a parameter that cannot be overridden, wherever the
  child is defined: a body `parameter` of a module that has a parameter port
  list, a `localparam`, a type parameter, or a parameter of one of the child's
  generate blocks. With the child in another file, the value was applied or
  dropped without a message (§23.10.1, §6.20.1, §6.20.3, core #199,
  contributed by chuanseng-ng).

* A `defparam` written in a generate block reaches that block's instances. It
  was dropped, so `u.A` kept its default (§23.10.1, §27).

* A bind directive with several instances (`bind t c #(3) a(), b();`) binds
  each of them, also in the colon form and into an instance path. A bound
  array of instances builds its elements, and a bind directive the parser
  cannot read is an error (§23.11, §23.3.2, core #200, contributed by
  chuanseng-ng).

* A module can name an instance that a compilation-unit `bind` adds to it
  (`a.y`), from its own processes and from below through an upward name.
  Another top cannot (§23.11, §23.8, core #201, contributed by chuanseng-ng).

* A variable declared through a type parameter has the members of the type the
  parameter is bound to. This covers class-method locals and formals of every
  kind, collections, class typedefs of `T`, structs with `T` members, static
  properties, static methods called through a handle, and module, interface
  and program subroutine locals. Member reads returned 0 while the whole value
  was right. A store into an element of a packed-struct or packed-union array
  local is fitted to the element's width: `a[0] = '0;` kept one bit, so later
  member writes read x (§6.20.3, §8.25, §7.2, §10.7, #1082, reported by
  Ganesh-TNL).

* A later ANSI port that omits its direction, port kind and data type takes
  all three from the port before it; a port that gives any of them takes only
  the direction. In `input logic [7:0] d, input clk, rst_n` the port `rst_n`
  is one bit. A list opened by a port kind or packed dimensions now parses as
  ANSI (§23.2.2.3, #1078, reported by ds54e).

* The equality operators compare parenthesized unpacked arrays and structs
  element by element (member by member): `(a) === (b)` was 1 for two different
  arrays and `(a) == (b)` x, so a UVM `compare()`, whose comparer macros write
  `(LVALUE) !== (RVALUE)`, reported differing `uvm_field_array_*` and
  `uvm_field_sarray_*` fields equal. A call returning a fixed-size, dynamic
  array or queue compares its returned elements too (`f() == g` was x)
  (§11.3.2, §11.4.5, §13.4.1, #1085, reported by philipaxer).

* An unpacked array of unpacked structs copies member by member between
  differently shaped arrays: fixed-size to dynamic or queue and back,
  variables and class properties (`da = fx` in a method, `g = h.fx`), and out
  of a function, also a local queue or dynamic array of structs. The copy had
  the right length and default members, or was empty. A member written from a
  variable into an element of a subroutine's local dynamic array of structs
  (`d[i].a = s[i].a`) was taken for the whole struct and lost (§7.6, §7.2,
  §13.4.1, #1057, reported by Ganesh-TNL).

* `sort()` and `rsort()` with `with (item.<member>)` order a queue of packed
  structs by the member: the members were unbound, every key read 0 and the
  queue kept its order. `sort`, `rsort`, `min`, `max` and `unique` compare
  elements and keys at their full width and as their type does: a value wider
  than 64 bits was cut to its low word, a signed `int` sorted as unsigned
  (`'{0, 5, 7, -10, -3}`) and a `real` by its bit pattern. A signed packed
  member read in a `with` clause is signed, and a pattern pushed into a queue
  of packed structs (`q.push_back('{k, v})`) takes the element type (§7.12.1,
  §7.12.2, §7.12.3, §10.9, #1059, reported by Ganesh-TNL).

* A call returning an unpacked array, used as a whole operand of `inside`,
  `case ... inside`, `%p`, `$size` or an unpacked array concatenation, reads
  the elements it returns and runs once: `v inside {f()}` compared `v` with 0,
  `%p` printed 0 and a constraint `x inside {f()}` drew 0. A bare call in a
  class method (bound to the class first), `this.f()`, `pkg::f()`, `C::f()`
  and a method known by its `extern` or `pure virtual` prototype are
  recognized as returning an array, also for `size()`. An associative array
  variable in an `inside` set contributes its elements (§11.4.13, §21.2.1.7,
  §13.4.1, §18.5.11, #1036, reported by git-hevangel).

* `$urandom(seed)` reads its seed and never writes the generated value back
  into it, whatever the argument's storage or context (a `for` loop over a
  seeded index ran once). The seed now initializes the calling thread's
  generator: the calls without a seed that follow repeat for the same seed,
  and other threads are untouched. A seeded call returns the same value as
  before (§18.13.1, §18.14.2, #1058, reported by Ganesh-TNL).

* A property a subclass redeclares hides the base property only in code
  written in the subclass. Base-class methods (`f.add()`, `f.n++`), base-type
  handles at any link of a chain (`b.f.tag`, `b.f.n = 50`, `b.f.add()`) and
  `super.f` keep the base's property, also when one of the two is static. A
  sequencer that declared its own `m_req_fifo` took the requests, and the
  driver waited in `get_next_item` forever (§8.14, §8.15, #1064, reported by
  philipaxer).

* A member write to a packed-struct static class property (`Cls::sp.m = v`)
  updates the shared cell, and an element select of its packed-array member
  (`Cls::sp.lanes[0]`, `sp.lanes[1]` in a method) slices it. The write went to
  a stray copy that only module-scope reads saw (§8.9, §7.2.1, §7.4.1, #1046,
  reported by Ganesh-TNL).

* `$root.top.u.s.m` reads like `top.u.s.m`, and `top.u.s.lanes[i]` selects the
  element; both read x or a single bit. In a module instance, including every
  top of a multi-top design, `s.pairs[1].pair[0]` and an unpacked struct's
  `us.lanes[0]` select whole elements in declared order (§23.6, §7.2.1,
  §7.4.1, #1046, reported by Ganesh-TNL).

* A member of an element of an ascending packed array of packed structs inside
  a struct (`b.slots[0].dir` on `hdr_t [0:2] slots`) reads the right element
  in continuous assignments, `always_comb` and functions, through ports and in
  instances; `slots[0]` read x and `slots[2]` read element 0 (§7.4.1, §7.2.1,
  #1088, reported by Ganesh-TNL).

* `push_back`, `push_front` and `insert` on a queue of queues or of dynamic
  arrays store a concatenation (`{1, 2}`, `{tmp, 3}`) or an assignment pattern
  (`'{3, 4, 5}`, `'{3{7}}`) as the new row; the row was empty, or had one
  element for a class property or task local (§7.10.2.2, §10.10, #1086,
  contributed by chuanseng-ng).

* Real math functions in constant expressions, including logarithms,
  trigonometric functions and nested real/bit conversions, produce their
  real results at elaboration. `$rtoi` can use those results in dimensions,
  and parameter substitutions in subroutine signatures preserve the declared
  width and signedness. Recompile saved designs made with an earlier build
  (§11.2.1, §20.5, §20.8.2, #1089, reported by ds54e).

* Continuous assignments passing an unpacked struct array to a function
  reevaluate when an element's member changes, including nested members and
  instance-scoped arrays; the result previously retained its old value
  (§10.3.2, §7.2, #1090, reported by ds54e).

**Performance**

* Deferred instance type contexts share exact-equal type, width and array-query
  maps, including the thread-local snapshots used during materialization.
  A synthetic 2,048-instance design with 64 type parameters peaks at 269 MiB
  instead of 449 MiB; different widths and bounds remain independent
  (§6.20.3, §20.7, #721).

* Unused compilation-unit properties and sequences remain on one root for
  validation instead of being copied into every unrelated module definition.
  A synthetic case with 1,024 unused modules and four 512-term properties
  peaks at 35 MiB instead of 1,141 MiB (§3.12.1, §16.8, §16.12, #721).

* Edge-dispatch plans execute serially by default
  (`XEZIM_PARALLEL_SERIALIZE=1`). Set it to `0` to allow worker threads
  when the parallel dispatcher qualifies.

* Callable locals with literal or name-free dimensions stay shared during
  elaboration, avoiding copies for unrelated parameters (§6.20.2, #721).

* Preparing parameterized modules preserves shared function/task bodies when
  local declarations need no dimension or type folding. The public core #192
  reproducer's peak requested heap falls from 150.3 MB to 9.7 MB; genuinely
  instance-specific rewritten bodies still retain their own storage (§13.3,
  §13.4, #721).

* The class collector also runs when the name-keyed store has grown by 100,000
  values, so objects with large array members are reclaimed before 20,000 of
  them exist: 16,384 objects with a 256-byte array peak at 156 MB instead of
  1.5 GB (§8.7, #1054, contributed by Thomasb81).

* The command line simulates on the large-stack worker it already runs on
  instead of reserving a second stack; a run under a 4 GiB address-space limit
  no longer fails at the default stack size (#1055, contributed by Thomasb81).

* An instance's initial block is materialized when its process starts, not at
  setup: 512 instances of a 512-statement block take 47 MB instead of 349 MB
  after scheduling, and 4 ms instead of 254 ms (§9.2.1, #1056, contributed by
  Thomasb81).

* The reference census of dotted names is released when the last compilation
  session ends (xezim-core #191, contributed by Thomasb81).

* A constant-function call during elaboration (a generate condition, a
  dimension or a parameter that calls a package function) shares the design's
  typedef tables instead of copying them on every call, and copies of an
  elaboration share its typedef table until one of them is written. 64
  generate conditions calling a package function beside 256 packed-struct
  typedefs take 0.89 G instructions instead of 4.67 G, 256 calls beside 1,024
  typedefs 3.6 G instead of 63.8 G, and 2,048 typedefs peak at 373 MB instead
  of 484 MB (§13.4.3, #721, reported by amal-khailtash).

**Build**

* Building from source requires Rust 1.100 or newer. The simulator, core and
  parser share this minimum supported Rust version, checked directly in CI
  for default and optional native-backend builds.

* Native C/C++ dependencies, DPI libraries and build shims default to Clang
  and LLVM archive tools. Linux x86_64 Rust builds link through Clang and
  Rust's bundled LLD. Local core overrides now live in the ignored
  `.cargo/local-core.toml`, preserving the tracked compiler configuration.

### 0.11.5 — generate-block scopes, checkpoints, class and randomization correctness (November 2026)

**Command line**

* `--relax-library-packages` (`XEZIM_RELAX_LIBRARY_PACKAGES=1`) compiles the
  packages a `-v` library file declares, as if the file were also a source
  file; by default a library file supplies only modules, interfaces,
  programs and primitives (#937, requested by Ganesh-TNL). A package a
  source file declares wins.
* Checkpoints: `--save-at <time>[,<file>]` saves the state at the end of a
  time slot and `--restore <file>` continues from it, and `$save`,
  `$incsave` and `$restart` do the same from the design (IEEE 1800 Annex D,
  #738). A restored run prints what the uninterrupted run prints after the
  saved slot; open files are reopened at their positions, plusargs come from
  the restoring command line, and a checkpoint of another design or xezim
  build is refused. The interpreter, `XEZIM_JIT` and `XEZIM_AOT` may differ
  between the two runs. Not yet restored: a process forked to call a task
  while it is still inside the task, which is refused with an error.
  `XEZIM_CKPT_AUDIT` lists the fields a run changes after setup.
* On the command line, `-svseed <n>`, `-seed <n>`, `-svseed=<n>` and
  `-seed=<n>` seed the run, as they did in args files; they were read as a
  glued `-s<top>`. `-sv_seed=<n>` is accepted in both places. A run given a
  seed prints `[xezim] seed: <n>` on stdout, so the output and the `-l` log
  name it, and `+seed=random` prints the seed it drew there too instead of
  on stderr. `-top <module>` is `-s <module>`, on the command line and in
  args files.

**Correctness**

* A generate block is a scope of its own. Each definition gets a scope tree
  and a binder that resolves every simple name, and the generate rename
  follows that binding:
  - an unlabelled block keeps its data under `genblk<n>` (§27.5, §27.6);
  - a bare upward subroutine call resolves through the instance tree
    (§23.8.1);
  - a `disable` names its block by path (§9.6.2);
  - a block's typedefs are the block's (§6.18);
  - an undeclared bare name inside an instance is an error, as it is at the
    top level (§6.10, §23.9).

  (#952, contributed by philipaxer)
* A generate block's tasks, functions and processes take their time scope and
  clocking blocks from their instance (§3.14.2.1, §14.3, contributed by
  philipaxer).
* A task or void function with no written lifetime is static on the compiled
  tier too, so a local or output formal that carries a value keeps it between
  calls from a clocked process (§6.21, §13.3.1, #970, contributed by
  philipaxer). A subroutine whose variables are all written before they are
  read stays inlined.
* An implicit net created inside a sub-module counts as declared for a later
  initializer in that module (§6.10, #982, #983, contributed by davekeeshan).
  An implicit net of one module no longer declares the same name in an
  unrelated one. An implicit net used in a generate block belongs to that
  block: each loop iteration has its own, and `g[0].n` reaches it.
* Array queries and `$bits` over a generate-block variable are answered from
  its declared shape, also from outside the block (`$size(g[0].arr)`)
  (§20.6.2, §20.7, xezim-core#178, xezim-core#179, contributed by
  chuanseng-ng).
* A comparison between two elements of a class-member array in a compiled
  method (`base[i] < base[j] + len[j]`) evaluates at the elements' declared
  width; it used only their lowest bit (§11.6.1, #977, #980, contributed by
  shabbarvejlani).
* A `foreach` modulo equality on array elements (`(base[i] % sz[i]) == 0`)
  is solved by drawing the element from its congruence class, within the
  element's `<`, `<=`, `>` and `>=` bounds. `randomize()` used to report no
  solution (§18.5.8.1, #976, #979, contributed by shabbarvejlani).
* A bit, part or element select written to a class property lands. This
  covers properties declared through a typedef or an enum, and `int`,
  `byte`, `shortint` and packed struct or union properties; the write was
  dropped (§7.4.1, §6.11, #972, #973, contributed by davekeeshan).
* A select past an associative array's dimensions selects into the element
  (`a[k][3]`, `p[k][1]`, `p[k][3:2]`), and elements of multi-dimensional,
  offset and ascending packed types are stored whole (§7.8, §7.4.1, §11.5.1,
  #956, #958, #957, #959, contributed by chuanseng-ng).
* A static method called through a handle runs on the handle's declared class
  and specialization (§8.23, §8.25, #953, #954, contributed by davekeeshan).
  This holds through a null handle and in compiled code, where it returned 0,
  and through a base-typed handle to a derived object, where it ran the
  derived class's method.
* `@(posedge x)`, `@(negedge x)` and `@(edge x)` on a class property or a
  procedural local wait for the transition; they returned at once (§9.4.2,
  #950, #951, contributed by davekeeshan). The edges follow Table 9-2 on the
  LSB, and `or` lists and `iff` guards over class properties wait for their
  own transitions.
* VPI handles to an unlabelled generate block's variables read the block's
  own value (§27.6, §37).
* A typedef that depends on the genvar of a generate-for block inside an
  instance has each iteration's width (§6.18, §27.4).
* `$unit::x` in a generate block that declares `x` reads the compilation
  unit's `x` (§3.12.1).
* Regression suites for base-class properties shadowed by a derived class's
  parameter or static (§8.14, #944, #960, contributed by Thomasb81).
* A class property that a derived class redeclares as a `localparam`,
  `parameter`, `static` property or typedef resolves in the scope of the
  reference (§8.14, §8.23, #944, reported by Ganesh-TNL). This covers
  `this.x`, bare `x`, compound, select and nonblocking writes in base
  methods, base-typed handles (package-qualified ones included), collection
  elements, static handles and member chains, base-class constraint blocks
  and inline `randomize() with` blocks. Writes went to the derived class's
  static or constant.
* A `foreach` over a packed array of packed structs, or over another
  multi-dimensional packed class property, covers every element: in a
  constraint `foreach (c[i]) c[i].valid == 1'b1` is solved and checked, and
  a procedural loop runs once per element instead of once (§7.4.1,
  §18.5.8.1, #928, reported by Ganesh-TNL).
* Live frames of different processes that declare a local of the same name
  each keep their own type, and switching between them no longer rescans the
  module's struct and packed-layout tables; the reporter's matrix drops from
  78 s to under a second. A fork child's block local named like a variable
  of its parent keeps its own value (§6.21, §9.3.2, #943, reported by
  Ganesh-TNL).
* An ascending packed array of a multi-bit typedef (`u8_t [0:7]`) declared as
  a non-ANSI port keeps its element order; it was mirrored across the port.
  A module-local typedef no longer sizes such a port at 32 bits per element,
  packed struct-array ports get their shape, and a module-local
  unpacked-array typedef keeps its dimensions in a submodule (§7.4.1,
  §23.2.2.1, #942, reported by Ganesh-TNL). A function local or formal with
  an ascending outer packed dimension no longer comes back element-reversed
  from a continuous assignment or `always_comb`.
* A compiled read of a module-scope queue or dynamic-array element at index
  64 or above read x, so a `for` loop `t += q[i]` over 65 elements summed to
  0, and a read past `size()` returned a stale element (§7.5, §7.10). Such
  reads now use the live collection.
* `s += x` on a 2-state local inside a `foreach` of a compiled function kept
  `s` at x (§6.11.1). `d[i] = new[n](d[i])` keeps the row's elements
  (§7.5.1), and `$size` and the other array queries of a class member's
  dynamic array of fixed rows, and `$bits` of a module-scope row, count the
  row (§20.6.2, §20.7).
* In a design with more than one top-level module, a `$unit` nettype's
  resolution function runs; the net read one driver alone. Two drivers of a
  nettype without a resolution function, through ports or from a child and
  its parent, are an error, as §6.6.7 requires.
* A read of a queue or dynamic-array element at or past `size()` gives the
  element type's default (x, 0, null or "") instead of the element a
  `delete`, `pop_front` or `pop_back` left behind (§7.10.1, §7.8.6, #949 and
  #948, contributed by davekeeshan). This includes a row of a fixed array of
  queues, `fq[1][3]`, and a queue of class handles, which reads null.
* A package array whose first dimension is sized by a parameter,
  `logic [31:0] d [N][2]`, keeps all its dimensions (§7.4.2, §26.2, core #169
  and #168, contributed by chuanseng-ng); `p::d[3][1]` read 0.
* A user-defined nettype without a resolution function takes an
  assignment-pattern driver through an output or inout port (§6.6.7, core
  #170, #945 and #946, contributed by ds54e); the parent net read 0 in every
  member. A continuous assignment that selects into such a net,
  `assign p.V = v;`, is now an error (§10.3).
* A name declared nowhere in the design is an error in a class parameter
  override, nested ones included (`R #(A #(bad))`), in a class
  value-parameter default and in a class property or package variable
  initializer (§6.20.2, §8.25, #947 and #888, contributed by Ganesh-TNL).
  Literals and system calls in a nested override, such as `8'hFF`,
  `$clog2(16)` and `10ns`, are not taken for names.
* A whole-struct write through a port reaches an instance-array element
  actual and a nested struct member actual member by member (§23.3.3.5,
  §7.2, core #166 and #917, contributed by ds54e); the parent read every
  member as 0.
* A nonblocking assignment to a row of a multi-dimensional unpacked array,
  `q[1] <= x`, updates every element of the row (§10.4.2, §7.6, #906,
  contributed by auty1395).
* A class specialized by a parameterized virtual interface, `res#(virtual
  mon_if)`, is the same type under each spelling, and `$cast` to it succeeds
  (§8.25, §25.9, #908, contributed by auty1395).
* A class-typedef formal of a 2-state type is 2-state, and a function-name
  return variable has the declared width before the method compiles
  (§6.11.2, §13.4.1, §8.25, #910, contributed by auty1395).
* `C::q` and `D::q` of a static queue copy the queue in a whole-array
  assignment, also through a subclass (§8.23, §8.9, §7.6, #912, contributed
  by auty1395).
* Assigning a struct class property copies its nested struct and queue
  members (§7.2.2, #914, contributed by auty1395).
* `unique {a, b}` over rand dynamic arrays, fixed-size arrays and scalars
  makes every element distinct; `randomize()` failed, or allowed duplicates
  with a scalar in the list (§18.5.5, #918, contributed by auty1395).
* A continuous assign that calls a function reading a member of a
  packed-struct formal is compiled instead of interpreted (§7.2, §13.4,
  #923, contributed by auty1395).
* A class queue of unpacked structs keeps its declaration initializer,
  element by element (§7.10, §10.9, #926, contributed by auty1395).
* Shift, `/`, `%` and `**` on a property sized by a class parameter give
  the right result once the method runs compiled (§11.4.10, §11.4.3, #929,
  contributed by auty1395).
* A bare identifier used as the key of an array assignment pattern,
  `'{ CMD_VOUT : v }`, is an index (§10.9.1, #931, contributed by
  davekeeshan); the item was dropped. A type name as the key is still not
  supported.
* A time or real bound of an `inside` range keeps the integers it covers,
  `x inside {[0:2000ns]}` (§11.4.13, #933, contributed by davekeeshan);
  `randomize()` failed every trial.
* A fork child's write of the value a shared automatic held at the fork
  reaches the parent, whether the child or a sibling changed it before;
  and a child that only received a sibling's write no longer sends it back
  over the parent's own later write (§6.21, §9.3.2, #935, contributed by
  davekeeshan).
* The dead-clock watchdog fires only when the awaited signal has no live
  driver, so a control signal held during a slow power-up is not reported
  (#938, contributed by davekeeshan).
* `$cast` takes a scoped and a bare spelling of one class or
  specialization for the same type, a bare `new()` inside a parameterized
  class keeps the active specialization, a program `initial` process that
  calls a class task across a delay keeps its context, and a compiled
  method keeps a real type-parameter return real (§8.16, §8.25, §24.3,
  #940, contributed by Thomasb81).
* An instantiated module's `import` of a package the design does not
  declare is an error at that import (§26.3, #937, reported by Ganesh-TNL).
  The import was skipped, so a name used from the package was reported as
  undeclared, or nothing was reported. A package declared in a `-v`/`-y`
  library file is still not compiled, since a library file supplies only
  modules, interfaces, programs and primitives; the error now notes where the
  library file declared it. An error inside a module adopted from a library
  file names a line of that file instead of a line of a primary file, and
  `import std::*;` no longer fails with "Package 'std' not found".
* A function or task declared in a generate block belongs to that block
  (§23.9, §27.6, #897, reported by philipaxer). A bare call from the block's
  processes, continuous assigns, other subroutines or nested blocks, and a
  `disable` of its task, bind to the innermost enclosing block that declares
  the name, then to the module; a formal shadows same-named block data; a
  block's task no longer collides with a same-named module task. Two blocks
  that each declared `look()` both ran the one inserted last, and a
  generate-for in an instance left its calls on the module's table.
* A `[N]` array declared in a generate block and sized by a module-level or
  package parameter is a fixed array (§7.4.2, core #162). It was registered
  as associative as well, so `foreach` ran zero times and reads gave x.
* `.*` connects no null port or unnamed port expression of a non-ANSI child,
  and the missing-connection check no longer reports their placeholders
  (§23.3.2.4, core #163).
* An upward hierarchical name is resolved at elaboration to the nearest
  enclosing scope it names (§23.8, #890, reported by Ganesh-TNL): the module
  name of an enclosing instance (`node1.req1` in a module bound into an
  instance of `node1`), a child instance of an enclosing scope, or an
  enclosing scope's generate label, in continuous assigns, procedural blocks,
  function bodies, event controls, generate blocks and on the left-hand side
  of a store. Such a name used to fall back to a same-named signal of the
  top module. A class collection reached through a handle named by an
  instance path (`u_n.obj.arr[k]`) reads and stores the object's collection
  instead of x (§23.6).
* A real stored into an integral target rounds to the nearest integer, ties
  away from zero, at the target's declared width on every store path
  (§6.12.2, #889, reported by Ganesh-TNL): associative-array elements of any
  key type and nesting, class instance and static properties, including
  inherited ones and those reached through handle chains or copied from task
  and constructor formals, struct members of collection elements and package
  structs, and `ref` formals. `%d`, `%h`, `%o`, `%b` and `%c` of a real
  argument print the value converted to a 32-bit integer; they printed the
  real's digits or its IEEE-754 bits.
* A subroutine local's packed geometry no longer leaks into another
  subroutine's same-named local (§23.9, #916, reported by Ganesh-TNL). After
  a function declaring `u8_t [0:7] v` had run, a compiled function with a
  plain `logic [63:0] v` wrote single bytes through the stale geometry when a
  statement fell back to the interpreter. A fresh local's registration is
  removed when its call ends, and compiled code's fallbacks shadow stale
  bare-name geometry.
* A class property whose type is a packed struct with packed dimensions
  written before the declarator, `rand cand_t [0:2] a;`, a
  `typedef cand_t [0:2] carr_t` or an inline `struct packed {…} [0:2] a`, is
  a packed array of structs (§7.4.1, §6.20.2, §18.5, #928, reported by
  Ganesh-TNL): `a[i]`, `a[i].f`, `this.a[i].f`, nested and 2-D selects read
  and write the element's bits, and constraints on `a[i]` and `a[i].f`
  (equality, `inside`, `dist`, implication, `soft`, cross-element,
  part-selects) are solved at the property's full width. The dimensions were
  lost with the typedef name, so `a[i]` read 0 and such constraints were
  dropped. Still open: a field named through a `foreach` loop variable,
  `foreach (c[i]) c[i].valid == 1'b1`, and an unpacked array of struct
  arrays, `cand_t [0:1] g[2]`.
* The array reductions `and()`, `or()`, `xor()` and `product()` keep every
  bit of elements wider than 64 bits in the evaluator and the solver, element
  chains such as `a[0] + a[1]`, `a[0] - a[1]` and `a[0] + b[0]` accept
  targets above 2^64 - 1, and `unique {a}` over wide elements no longer
  disables the rest of the constraint set (§7.12.3, §18.5.8, #928, reported
  by Ganesh-TNL). Only `sum()` had the wide path.
* A collection of virtual interfaces keeps its element handles when passed
  as a subroutine or constructor argument (dynamic, queue, fixed, 2-D,
  typedef'd, `ref` or `output`, or an interface instance array as the
  actual), declared as a task local, returned from a function, or held in a
  2-D class property (§13.5.1, §23.9, §25.9, §25.10, #925, reported by
  Ganesh-TNL): `f[i] != null`, `f[i].sig`, `@(posedge f[i].clk)` and
  `wait (f[i].sig)` resolve through it; the elements read null or x. Still
  open: `f[i][j] != null` and `.sig` reads of a fixed-size 2-D formal, and a
  module-scope 2-D array variable of virtual interfaces.
* A module's own package imports are searched before the compilation unit's
  wildcard import when a contested name sizes an unpacked array `a[N]`
  (§26.3, §7.4.2, #895, reported by philipaxer). The value was right and the
  dimension used the unit's candidate.
* A parameter with a declared type is folded at that type's width wherever
  it sizes a declaration, not only in its value (§10.8, §11.6.1, #894,
  reported by philipaxer): `localparam int A = (8'd128 + 8'd128) >> 8` gives
  `logic [A:0]` two bits; it gave one. An untyped parameter keeps the
  self-determined width.
* The constant checks of the elaboration lint, a zero-width indexed
  part-select (§11.5.1), a zero-size unpacked dimension (§7.4.2) and
  duplicate enum values (§6.19), run on every instance of a child module,
  and `int m [N]` with a zero `N` is rejected at the top too (#893, reported
  by philipaxer). A child with any of them elaborated silently.
* An equality that reaches its random variable through `^`, `~^`, `~`, `+`
  or `-`, such as `(data ^ 32'hf0) == 32'h3`, is solved exactly, in class
  constraints and in `std::randomize() with` (§18.5.12, §11.8.2).
  `std::randomize` returned 0 even for `(data ^ 'hf) == 0` on a 32-bit
  variable. A class constraint solved it only when the constant happened to
  keep the expression linear on the solver's probe points, as `'hf` does and
  `'hf0` does not. The solution still has to fit the variable once extended
  to the comparison's width, so an 8-bit `p` gives
  `(p ^ 32'h100) == 32'h5` no solution.
* An input port whose actual holds its declaration initializer takes that
  value as an edge at time 0 (§23.3.3, §9.4.2). With `logic clk = 0`, a
  child's `always @(negedge clk)` runs at time 0, and with `logic rst_n = 0`,
  an `always_ff @(posedge clk or negedge rst_n)` resets at time 0. These
  blocks used to wait for the first later change. The edge starts from x for
  a 4-state port and from 0 for a 2-state one. The declaring module's own
  blocks on the variable still see no event (§6.8), and neither do the
  instance's initial procedures or `iff` terms. The same update reaches a
  parent's block on the port's hierarchical name (`@(negedge v.clk)`, §23.6)
  and the readers of a whole-net identity assignment from such a variable
  (`wire w = v`, `assign w = v`, §10.3.3), whose net took the variable's
  initializer silently. When a process also writes the variable at time 0,
  the port's edge comes first and a block both wake runs once; a `#0` or
  nonblocking write wakes it again.
* A constraint that joins one-bit terms with `&` or `|`, such as
  `s == S_DATA & data == 32'h1234`, is solved like `&&` and `||` (§11.4.5,
  §18.5, #899). `randomize()` returned 0 and left the members unchanged,
  because the 32-bit term was only ever sampled. A `rand bit`, a bit select
  such as `data[0]` and a relation the solver inverts, such as
  `(data ^ 'hf) == 0`, count as one-bit terms too; an operand wider than one
  bit keeps its bitwise meaning (PR #900, contributed by davekeeshan).
* A continuous assignment to a path that enters a scope through an index,
  such as `assign tb.g[0].u.a = s;`, drives its target (§23.6, §10.3.2,
  #901). Written with the top module's name, or with a delay, it drove
  nothing and printed nothing. A bit or part select on the target,
  `assign g[0].u.a[3:1] = 3'b011;`, drives those bits, and the generate
  block no longer becomes a 1-bit implicit net with a warning (PR #902,
  contributed by davekeeshan).
* An event control on a path that enters a scope through an index, such as
  `@(posedge g[0].u.clk)`, `@(posedge g[0].u.c[0])` or `@(g[0].u.c[2:1])`,
  wakes on every change (§9.4.2, #903). The process woke once at time 0 and
  never again, so a flop clocked through such a path stood still (PR #904,
  contributed by davekeeshan).
* The lint reads a child module's names in the child's own scope (§23.9,
  #891). A variable of one module is no longer taken for a same-named queue
  of another, and a child's part-select widths, enum values, array sizes and
  queue bounds are no longer folded with the top module's parameters, which
  rejected legal children. A child's check still runs when its expression
  reads only literals and package constants. A parenthesized integral value
  assigned to a queue, `q = (x + 1)`, is rejected like `q = x + 1` (§7.6)
  (PR #892, contributed by philipaxer).
* A reference in a generate block binds only to declarations visible from it
  (§23.9, §27.5). An outer `W` beside a nested block's own `localparam int W`
  was taken for the nested one, giving a 1-bit parameter type, a value of x
  or 0, or an undeclared `g.W`; parameter types, unpacked dimensions,
  initializers, declarations, assigns and statements now read the outer `W`
  (core #162, PR core #165, contributed by chuanseng-ng).
* A function or task declared in a generate-for block can be called by its
  bare name from inside the block: from an `initial` or `always`, a
  continuous assign, another subroutine of the block, the function's own
  result variable or a nested block (§27.4, §23.9). Each iteration's copy is
  named `g[i].f`, but the call was left bare, so elaboration stopped with
  "Undeclared identifier 'f'". `g[i].f()` from outside the loop already
  worked.
* Module parameters in class and virtual-interface specialization arguments
  fold in their calling scope even when an unrelated class declares a
  parameter with the same name (§8.25, #874). A virtual-interface key such
  as `data_if#(DATA_WIDTH)` now matches `data_if#(32)` when the module's
  `DATA_WIDTH` is 32, allowing an inherited agent's `uvm_config_db::get`
  to retrieve the interface. Compound expressions resolve using the active
  module or class parameter bindings.
* A trailing comma in a `#(...)` parameter value list is an error in strict
  mode (Annex A.4.1.1, #874). An empty list, `#()`, and named default
  assignments such as `.WIDTH()` are accepted.

* `size()` applies to the queue a locator method returns: `q.find_index
  with (item > 2).size()` read 0 (§7.12.1, §7.10.2, #779). A locator on a
  row of a nested associative array, `m["a"].find_index with (item > 1)`,
  searches that row by its own index type, where it returned `'{0}`
  (§7.12.1, §7.8.1, #780). A string-keyed array whose keys all read as
  integers is ordered as strings: `first()` gave the number 9 instead of
  `"10"` (§7.8.2, §7.9.4, #781). `%p` of a locator call prints an enum
  index or element by its label, `'{BLUE}` where it printed `'{2}`
  (§21.2.1.6, #782). Every queue method and the `with` reductions apply to
  a locator result too (`q.find with (item > 2).sum()`, `q.unique.size`), a
  row of a class member's nested array is searched the same way, and an
  enum index named through a typedef alias or held by a function-local
  array prints its labels. Reported by philipaxer, with PRs #783, #784,
  #785 and #805.
* A string key may hold `[`, `]` or `\` everywhere: such a key is escaped in
  the element's name, where `%p`, `num()`, nested rows and the walks split
  it at the bracket and read another key. `next()` and `prev()` from a key
  that is not in the array move to the nearest key, where they returned 0
  (§7.8.1, §7.8.5, #775, PR core #134 and #776). The locator methods search
  an associative array by key: a string-, signed- or wide-keyed array gave
  an empty queue, and the `_index` forms and `item.index` take the array's
  index type (§7.12.1, §7.12.4, #777, PR #778). Reported and fixed by
  philipaxer. An index type wider than 64 bits names one element per
  value, so `w[5]` and `w[100'h5]` are the same element and a `next()`
  walk over such keys ends (§7.8.4).
* A fixed-size array declared in an `initial`, `always` or `final` block,
  a nested block, a fork child or a static task keeps the element writes a
  `for (int i ...)` loop makes: a loop the process compiled stored each
  element through the signal table, where a block-local array has no cells,
  and every write was dropped, so the array read 0 or x after the loop
  (§6.21, §12.7.1, reported by Ganesh-TNL).
* A constructor call is checked against the class's own `new` like any
  other call: too many actuals, or a formal left without an actual and
  without a default, is an error, for `new`, `new(...)` and `C::new(...)`,
  in assignments and declarations. A class that declares no constructor has
  the implicit `new()`, which takes no arguments. Such calls ran with the
  missing formals left at their defaults and the extra actuals dropped
  (§8.7, §13.5, #812, reported by dhanunay).
* Concurrent processes whose automatic locals share a name but not a type
  keep each local's own type. The declaration that ran last registered its
  enum typedef, struct members, packed layout and class under the bare
  name, so a class task suspended across another's declaration read
  `c.num()` from the other enum, printed the other struct's members with
  `%p` and selected `w.f[3]` with the other packed layout (§6.21, §13.3.1,
  reported by Ganesh-TNL).
* An `event` declared in a labeled block of two processes is two events,
  stored under the blocks' names: a wait on one resumed in the same time
  step instead of at its trigger, so `@(e)` woke at time 0, even with
  nothing triggering it (§6.21, §15.5, reported by Ganesh-TNL).
* Two class handles declared under one name in nested blocks of a process
  each read their own members: the block rename gave every reference the
  declaration's source position, and `p.b.size()` read `p.a`'s length
  (§6.21, §23.6, reported by Ganesh-TNL).
* A `dist` on a dynamic array's or a queue's `size()` is drawn on every
  `randomize()`; an object randomized again kept the length it had (§18.4,
  §18.5.4, reported by Ganesh-TNL).
* `and()`, `or()`, `xor()` and `product()` equalities in a constraint are
  solved and checked, where `randomize()` returned 1 with them violated. A
  reduction or locator `with (item.field)` over packed-struct elements
  reads the field, which read 0 for every element, and the result has the
  field's width, so `sum() with (item.code)` over 4-bit codes wraps. An
  associative array is never resized by randomization, so an unmet
  `aa.size()`, `num()` or `exists()` constraint fails the call instead of
  passing with the array empty (§7.12.1, §7.12.3, §18.4, §18.5.8.2,
  reported by Ganesh-TNL).
* `dist` weights hold for an element of a fixed array and for a field of a
  dynamic array's struct element (`p[i].mask dist …`), which were drawn at
  the uniform baseline, and for an item whose branch adds constraints: a
  failed trial no longer uses up its dealt item, the retries deal it again,
  and the joint solver that follows keeps it where it can. That solver
  repaired the failed trial toward the other item, so a 1-in-10 wrap mode
  came out 1 in 30 to 1 in 50. A fresh object's first draw is no longer
  always the heaviest item, which a testbench creating an object per call
  saw as a 1-in-10 item never drawn. A field equality on an element
  (`p[i].valid == 1`) is solved too, where it made the element be drawn
  again whole (§18.5.4, §18.5.7, reported by Ganesh-TNL).
* A run with programs ends when every thread that started in a program's
  initial procedures has ended, by an implicit `$finish`, and a program's
  remaining threads end with its initial procedures. The run went on to its
  time limit, so `final` procedures reported that time and a module's later
  output still printed (§24.3, §24.7).
* Each operand of `&&`, `||`, `->` and `<->` is self-determined in
  compiled continuous assignments, `always_comb` and `always_ff`: both were
  sized to the wider one, counting a call as wide as its widest argument, so
  `~f(x) || 1'b0` with a 1-bit `f(x)` of 1 read true (§11.6.1, #810,
  reported by chuanseng-ng).
* A value change made by `vpi_put_value` inside a VPI routine is reported
  once that routine returns, in the same time slot, not by calling the
  value-change routine from within it. cocotb 1.x aborted with "We are
  calling up again" when a test drove a clock from Python and awaited its
  edge (§38.36.1, #811, reported by chuanseng-ng).
* `$cast` to an interface-class handle succeeds when the object's class, or
  a base class, implements that interface class or one extending it; it
  returned 0 because only the `extends` chain was searched (§8.26, §8.16,
  #806, reported by dhanunay).
* A time literal and `$time` in a package or `$unit` task, function or
  class use that scope's time unit, as its delays do; they used the calling
  module's, so `time t = 1ns; #(t);` in a 1 ns package waited 1 µs from a
  1 ps module (§3.14.2.3, §5.8, #807, PR core #139 and #808). Reported and
  fixed by philipaxer.
* The array queries of a type parameter (`$left(T)`, `$right`, `$high`,
  `$low`, `$size`, `$increment`, `$dimensions`, `$unpacked_dimensions`)
  answer from the type it is bound to, in constants and at run time; they
  read the type's width alone as `[w-1:0]` in a constant and 0 in a
  statement (§20.7, PR core #138, contributed by chenbhil).
* An array pattern fills a packed array element by element where the array
  is a member of a packed struct and where its elements are packed structs:
  a nested `'{default: 4'hA}` member pattern, a direct `cfg.dom = '{...}`
  with positional, `default:` or `index: value` items, and `'{default:
  '{...}}` on a packed or unpacked array of structs kept only the low
  element, wrote nothing, or laid the items out at their own widths.
  `index: value` items name their element on any packed array, where they
  were concatenated in the order written. A struct-level `default:` fills
  each innermost element of a multi-dimensional packed-array member, which
  is no simple bit vector type, so `bit [16:0][3:0] m` takes 4'hA seventeen
  times from `'{default: 8'hAA}`, in module, class-property, array-element
  and unpacked-struct storage (§6.11.1, §10.9.1, §10.9.2, #771, reported by
  Ganesh-TNL).
* Class-member events, `$bits`, assignment widths and compiled methods
  (#786-#791, PRs #795-#800, contributed by AaronKel). An assigned
  class-member event names the source's event object, so a trigger through
  either name wakes `@` and sets `.triggered` on both, and a process
  already blocked on the old event stays on it (§15.5.5). `$bits` of an
  empty string is 0 and of a class-member queue its current size
  (§20.6.2). `return e` and a store into an associative element evaluate
  `e` in the return or element width, a `real` element excepted (§13.4.1,
  §11.6.1). A store `s.f = v` through a class handle is no longer dropped
  after a method with an enum local `s` has run, or while a caller has one
  (§23.9). A class method gives its interpreted result once compiled, for
  redeclared string members, string-collection compares, member operand
  widths and signs, unpacked formals, bare-member stores, `ref` and `inout`
  copy-backs, shadowed handles, specialized element widths and `&&`, `||`
  and `?:` whose unselected operand calls a function or `$cast`
  (§11.4.7, §11.4.11, §13.5.2).
* A header default that reads a type parameter, `parameter int H =
  $bits(T)`, folds with this instance's `T`, also for a genvar-sized
  override `.T(logic [g*2-1:0])` and a struct type parameter; it read the
  previous instance's type, 0 for the first (§6.20.3, §20.6.2, §27.4, PR
  core #135 and #803, contributed by chenbhil).
* In a parameterized class, a formal, return or local typed `bit [W-1:0]`,
  or by a class typedef of it (`typedef bit [W-1:0] vec_t;`), is as wide
  as the object's specialization makes it, with the type's signedness, also
  once the method is compiled hot. A formal kept the actual's 32 bits (and
  `$bits` read 32), an actual was not evaluated at the formal's width, a
  typedef formal read 1 bit, a typedef return was not truncated, and a
  typedef local was 32 bits wide. Each element of a `randc` array now
  cycles through its values, where it was drawn as `rand` (§8.25, §13.5,
  §18.4.2, #768, reported by Ganesh-TNL).
* `randomize()` fails when every weight of a `dist` is 0: an item of
  weight 0 is not in the set, so `v dist { 5 := 0 }`, `v dist { [0:15] :/
  0 }` or weights read from variables that are all 0 admit no value, yet
  the call returned 1 with the first item, also from `std::randomize`
  (§18.5.4, #770, reported by Ganesh-TNL).
* `obj.arr[i].rand_mode(0)` on an element of an unpacked rand array keeps
  that element's value through `randomize()`, and the other constraints see
  it; the call stopped the run with a false null-handle error. The query
  form reads the element's mode, and `arr.rand_mode(1)` enables every
  element again (§18.8, #769, reported by Ganesh-TNL).
* An array's size can be ordered in a `solve … before` list: `solve sel
  before arr.size();` failed to parse, and now draws `sel` first and the
  size in its turn, so the sizes guarded by `sel` hold and `sel` keeps its
  `dist` weights (§18.5.10).
* A constant function sizes a cast by a value parameter of its instance,
  `W'(x)`, so a child's localparam computed with one folds at elaboration
  for each instance's `W` (§6.24.1, §13.4.3, #763, PR core #130 and #764);
  an element select of a packed localparam declared in a child module's
  body keeps the element's width and remaining dimensions instead of
  reading one bit (§7.4.1, §7.4.5, #766, PR core #131 and #767). Reported
  and fixed by ds54e.
* Each row of a nested collection is the collection its declaration names
  at that depth, keyed by that level's own index type, at every scope. A
  queue held in an associative array (`int m[int][$]`) kept its element after
  `pop_front()` or `pop_back()`, and `delete(i)` stopped the run with a false
  null-handle error; a whole-row assignment `m[k] = q` stored one `x`; `%p`
  and `foreach` over a queue or dynamic outer level (`int d[][string][$]`)
  printed each row as a scalar; and a receiver select with a side effect
  (`m[f()].push_back(x)`) ran four times. A write outside a fixed parent's
  bounds or past a queue or dynamic row's size is now ignored, and a copied
  object or a by-value formal keeps every row (§7.4.5, §7.4.6, §7.8,
  §7.10.1, §12.7.3, §21.2.1.7, #765, PR core #132, #133 and #772–#774).
  Reported and fixed by philipaxer.

* A queue method on an element of a class's collection of queues of
  handles, `m[k].push_back(o)` in a compiled method, applies to that queue;
  it reported a null handle (§7.4.4, §7.10.2.7, #820, PR #821). Reported and
  fixed by philipaxer.
* A clockvar bound to an expression in an interface, `output q = q_drv;`,
  drives and samples that instance's signal, not a same-named net `q`; it
  drove nothing (§14.3, §14.5, #822, PR core #143 and #826). Reported and
  fixed by philipaxer.
* A size cast's width sizes the expression around it: `(9'(c) + s) >
  8'hff` with 8-bit `c` and `s` sums at 9 bits, where it wrapped to 0, and
  a parenthesized or computed size, `(W)'(x)`, `(W+1)'(x)` or
  `$clog2(N)'(x)`, keeps its value, where it was dropped (§6.24.1, §11.6.1,
  #827, PR core #144 and #828). In compiled code a function call in
  such an expression is as wide as its return type, and a size cast keeps
  its operand's signedness, so `(W)'(sa)` of a signed -1 sign-extends.
  Reported and fixed by davekeeshan.
* A nonblocking write of one field of an unpacked-struct array element
  inside an instance, `storage[0].value <= v`, completes; its expansion
  recursed and the time slot never ended (§10.4.2, #831, PR #832). The same
  write to an element of a queue or dynamic array wakes the processes that
  read it. Reported and fixed by davekeeshan.
* A parameter override list must fit the definition it instantiates,
  wherever either is written: excess ordered values, a parameter set twice
  and a name that is not an overridable parameter are errors (§23.10.2,
  #818, PR core #146), and so is a list mixing ordered and named entries
  (A.4.1.1, core #150, PR core #151). A parameter in a `generate` region of
  a module without a parameter port list takes an ordered slot, and its
  override applies; it kept its default (§23.10.2.1, §27.3). Reported and
  fixed by chuanseng-ng.
* A compiled delay expression reads its value: `always #1 a = a + 1;` under
  `` `timescale 1ns/100ps `` ran away with stall warnings, and `always
  #(1*2)` ran at the wrong period (§9.4.1, §22.7, PR #835, contributed by
  ds54e).
* An element of an inlined function's dynamic-array formal keeps its
  declared type: a `real` element divided as an integer, so `d[k] / x` read
  0, and a signed `int` element read as unsigned (§11.3.1, §13.5, PR #836,
  contributed by ds54e).
* A compiled process copies a whole unpacked struct member by member, where
  `always_comb l = t;` left `l` at its default, and an `always_comb` that
  reads a whole struct re-fires when a member changes, inside instances too
  (§7.2, §9.2.2.2.1, PR #838, contributed by ds54e). An inlined function's
  struct local is not taken for a module struct of the same name.
* A child module that writes a whole unpacked struct to its output port,
  `assign o = '{x, 7};`, drives the parent, which read the members' defaults
  (§7.2, §23.3.3, PR core #152 and #839, contributed by ds54e).
* A header parameter used as a port actual inside its module, `leaf u
  (.a(VREF));`, creates no implicit 1-bit net and no warning (§6.20, §6.10,
  PR core #154 and #840, contributed by ds54e).
* With `solve pkt.psel before pkt.paddr` over fields of a packed-struct rand
  member, `psel` is drawn over its range; it came up 2 on every draw
  (§18.5.10, #815, PR #829, contributed by Ganesh-TNL).
* The members of a function's unpacked-struct result start at their declared
  defaults: `function s_t f(); f.c = 1; endfunction` returned 0 or x in a
  member declared `int a = 3` (§13.4.1, §7.2.2).
* A UDP table row may write its symbols next to each other, `01:0;` or
  `(01)00:?:0;`, as A.5.3 defines them; such a table was rejected and the
  primitive became an empty module with an undriven output (§29.3.4, #841,
  reported by git-hevangel).
* An out-of-block method of a `$unit` class that writes a member by its bare
  name elaborates when the top module declares a variable of that name; the
  use-before-declaration check took the module's variable for the member
  (§8.24, §23.9, #843, reported by git-hevangel).
* A class property is the object's storage, whatever module variable shares
  its name: a nonblocking write in a method, `s.f <= v`, and an element write
  through a handle, `h.v[1] = x`, wrote the module's `s` or `v` (§8.4,
  §10.4.2).
* A clockvar bound to a part-select, `input a = bus[3:0];` or `output o =
  o_drv[3:0];`, samples that slice at the clocking event and a drive of it
  writes the slice and leaves the other bits alone; such a clockvar read x
  and drove nothing (§14.5, #823, reported by philipaxer).
* The output skew of a renamed clockvar, `output #2 q = q_drv;`, applies to a
  drive issued between clocking events: the drive took effect at the next
  clocking event instead of two time units after it (§14.4, §14.16, #824,
  reported by philipaxer).
* In a module that is not the top, a member of an element of a packed array
  of packed structs whose dimension is that module's parameter, `par[i].a`
  with `s_t [N-1:0] par`, reads and writes the member's bits of element `i`;
  the declaration was sized against the top's parameters, where `N` was
  missing or another module's, so a 1-bit first member landed on bit `i` of
  the whole array (§7.2.1, §7.4.5, §23.10, #816, reported by hernloh).
* A `localparam` or `parameter` of a packed array of packed structs type in
  an instantiated module takes its assignment pattern element by element,
  so `s_t [1:0] P = '{'{a: 8'h33, b: 8'h44}, '{a: 8'h11, b: 8'h22}}` is
  32'h3344_1122 and `$bits(P)` is 32; the pattern was packed as one struct
  (§6.20.2, §10.9.1, §10.9.2, #817, reported by hernloh). A header
  parameter overridden with such a pattern packs the same way.
* An `always_comb` that writes one element of a packed vector and reads
  another stays sensitive to the vector: `ds[1] = ds[0] + 1`, `w[7:4] =
  w[3:0] + 1` and `st[1] = st[i] + 1` never woke when the element they read
  was driven from outside, because every read of a name the block also
  writes was dropped (§9.2.2.2.1, #844, PR #845, contributed by auty1395).
  The rule follows the LRM text: the vector is kept unless the block writes
  it whole or at a non-constant index, or writes every element it reads;
  the block's own write does not re-queue it.
* Inside a class, `sub.rand_mode(0)` on a `rand` class-handle property
  changes the handle's own mode, as `this.sub.rand_mode(0)` does; it switched
  off every random variable inside the object instead, so the object's
  constraints were still checked and `randomize()` failed (§18.8, #846, PR
  #847, contributed by auty1395). A derived class randomizes the elements
  of an inherited array or queue of rand handles, and an element switched
  off with `rand_mode(0)` keeps its object (§18.8, §8.13).
* A class with a rand associative array is solved jointly, its elements
  indexed by the stored keys, so `main + sub.sum() == total` beside one no
  longer fails by repeated guessing (§18.4, §18.5, #848, PR #849, contributed
  by auty1395). The `foreach` key of such an array is bound at its declared
  index type, so an `int` key `-3` stays negative, and a string-keyed array
  is left to the trials without taking the rest of the class with it
  (§7.8.2).
* An `inside` or `dist` item written with a unary operator keeps its place
  in the set: `en inside {!cur}`, `n inside {~k}` and `en dist {cur :/ 2,
  ~cur :/ 1}` dropped the item, so the first had no solution and the last
  always drew `cur` (§18.5.4, §11.4.13, #850, PR #851, contributed by
  auty1395).
* `$countones` of a whole variable wider than 64 bits is a constraint the
  solver reads as the sum of its bits, so `std::randomize(m) with
  { $countones(m) inside {[1:2]}; }` on a 78-bit `m` succeeds (§18.5,
  §20.9, #852, PR #853, contributed by auty1395).
* A `ref` dynamic-array formal bound to one row of a fixed array of
  collections (`item_c rows[2][]`, passed as `rows[1]`) shares that row with
  the caller, so sizing and filling it in the callee is visible after the
  call (§13.5.2, #854, PR #855, contributed by auty1395).
* A subroutine's scalar formal or local hides a same-named collection of a
  caller or of a subclass: `str.len()` on a `string str` formal read the
  caller's `str[$]` queue size (§23.9, #856, PR #857, contributed by
  auty1395). It hides a same-named module-level collection too, as does a
  class property: a `$unit` function's `str.len()` read the module's
  `str[$]` size, and `return str` returned the queue (§23.9, §8.10).
* A class first named through `typedef class C;` resolves to the class, not
  to the placeholder the forward declaration leaves, so `q[i].label == ""`
  on a string member compares text and returns 1 or 0 instead of x (§8.27,
  §6.16, #858, PR #859, contributed by auty1395).
* A rand sub-object that no constraint of the class reads stays out of the
  class's joint solve, so a sub-object holding a rand dynamic array no
  longer sends the class's `sum()` constraint to the trial loop and makes
  `randomize()` fail (§18.5.9, §18.3, #860, PR #861, contributed by
  auty1395). A class constraint that reads a rand sub-object's member or
  array method, `soft sub.v == 7` or `sub.r.sum() == 60`, joins the
  sub-object's own solve as an in-line item that outranks its class
  constraints, where the first panicked and the second failed by chance
  (§18.5.14.1).
* A rand enum member of an unpacked struct declared in a package draws its
  named constants only; the member's type was looked up under its bare name
  and missed (§18.3, #862, PR #863, contributed by auty1395). An enum member
  of a rand *packed* struct is not restricted, as §18.4 says.
* The parameter override checks of #818 are pinned from the binary: exit
  status, the rendered line, `--compile` and `--no-strict`, children in
  other files, generate blocks, interfaces and binds (§23.10.2, A.4.1.1,
  PR #866, contributed by chuanseng-ng).
* An assignment pattern takes the type of its assignment-like context, and
  a typed pattern `T'{...}` takes `T`, everywhere: a function's `return
  '{...}`, a subroutine argument, a class property initializer, a cast
  operand, a conditional operand, a port actual and `==` against a pattern
  were laid out as a concatenation of the items at their own widths, so
  `s_t'{1, 0}` was 64 bits and `f() !== 2'b10` (§10.8, §10.9, #813, PR
  #814 and core #140, contributed by philipaxer). One plan builder lays a
  pattern over a packed type as bit slices for the interpreter and the
  bytecode compiler alike; `default:` assigns a value whose type matches the
  element whole (§6.22.1), a typed pattern of an unpacked type places its
  items as an untyped one does, and `$bits(T'{...})` is `$bits(T)`. A
  pattern assigned to a child's output port of a package struct type with a
  typedef-array member, and a `default:` naming a parameter of the child or
  a package parameter imported bare, took the port's flattened layout and
  missed the parameter's declared type, so the value was split by member
  (§23.3.3, §26.3). A plan is built on every interpreted execution: a loop
  of interpreted pattern assignments runs about 40% more instructions than
  before (compiled blocks and methods are unaffected); caching the plan per
  pattern is the open follow-up.
* A slice of an unpacked array, `a[l:r]`, `a[b +: w]` or `a[b -: w]`, is
  read and written as an unpacked array whose elements pair left to right
  with the other side (§7.4.6, §7.6, #867, reported by git-hevangel): as the
  source or target of a blocking or nonblocking assignment, slice to slice
  in opposite directions or overlapping, as a row or part of a row of a
  two-dimensional array, as an `input` or `output` argument, in `$size`
  (its element count, not its width in bits), in a continuous assignment
  (a variable base is followed) and on a port. The target kept its value,
  took the wrong elements, took them in reverse order or read x, depending
  on the place. A row of a two-dimensional array (`m[1]`) passed as an
  argument reads its elements too, and an `always_comb` that reads a whole
  unpacked array, `s = b`, wakes when an element changes; it never re-ran
  after time zero (§9.2.2.2.1).
* A sequential UDP that declares its output `output reg q;` in the body of a
  non-ANSI header keeps state: it was taken for a combinational UDP and its
  output stayed x (§29.3.2, A.5.2, #868, reported by git-hevangel). An
  initial value in the declaration, `output reg q = 1'b1`, gives the start
  state in either header form, as A.5.2 allows.
* The array-size check counts an indexed part-select of an unpacked array,
  `a[b +: w]`, as `w` elements: it took the base and the width for the
  bounds of a range, so `a[0 +: 6] = b` was rejected ("the arrays have 7
  and 6 elements"), some real mismatches were accepted and others reported
  with wrong sizes (§7.4.5, §7.6, §11.5.1, #869, reported by git-hevangel).
* A method of a nested class defined out of block inside the enclosing
  class body, `task inner::run(...)` written in `outer`, is linked to the
  nested class, in a package, through a constructor, as a virtual method and
  with type parameters; the call ran the empty prototype (§8.24, §8.23, #842,
  reported by git-hevangel). A nested class whose name collides across
  packages keeps its qualified spellings.
* Follow-ups to the recent randomization and argument fixes: a constraint on
  a selected part of a counted vector counts its bits with the rest
  (§20.9, #852), a fixed class-row actual and a side-effecting receiver are
  evaluated once (§13.5.2, #854), a rand sub-object's collections join the
  parent's solve and its `foreach` indexes the elements, not their bits
  (§18.5.9, #860), and a package struct member keeps its own enum domain
  when packages share a typedef name (§18.3, §26.3, #862). Packed and
  unpacked element reads of an `always_comb` share one dependency collector
  (§9.2.2.2.1, #844).

* Module data must be declared before it is read as well as before it is
  written (§6.5, #819). The check includes call arguments, event controls,
  assertions, declaration dimensions and module-local class bodies, in child
  definitions as well as the top, while respecting local scopes and
  hierarchical references. For sources that
  require the earlier behavior, `--relax-use-before-decl` (or
  `XEZIM_RELAX_USE_BEFORE_DECL=1`) accepts these references with a warning.

**Performance**

* Constrained randomization of large and chunked arrays with `foreach`
  constraints is much faster: 2.2× and 125× fewer instructions on the two
  benchmarks of #876 (§18.5.8.1, #955, contributed by Thomasb81). The
  rewritten shapes can pick different solutions than before for the same
  seed. Seeded runs remain reproducible.
* Binding a scope with many local names is linear in the number of names
  (§23.9).
* A package function call no longer copies the parameter table (#920,
  contributed by auty1395); 33 s to 2.6 s on the reported design.
* More code runs compiled (#940, contributed by Thomasb81): `repeat`,
  `forever`, `while` and `do-while` loops in processes, `disable` of an
  enclosing named block, process status reads, static class properties
  and methods through handle chains, and methods with type-parameter
  formals, locals and returns. `randomize()` reuses the translated
  constraint graph of a class while every value its constraints read is
  unchanged, solves element-independent members of a rand dynamic array
  element by element, and reuses the geometry of a constant `foreach`
  `dist`. Method dispatch and child-process cleanup allocate less.
* Instances whose parameters differ only in the instances above them share
  one prepared body. The body cache was keyed on every parameter in the
  scope, the ancestors' included, so a deep hierarchy formatted a long key
  and kept a body copy and that key for nearly every instance until
  elaboration ended (§23.10, #721, reported by amal-khailtash).
  `XEZIM_COMPILE_PHASES` reports the cache as `[elab-stats]` lines.
* More class methods compile (#792, PR #804, contributed by AaronKel): a
  statement the compiler cannot lower runs in the interpreter while the rest
  of the body stays compiled (`XEZIM_METHOD_STMT_FALLBACK=0` turns this
  off), compiled-to-compiled calls dispatch directly, `ref` and `inout`
  scalars included, and a method with a loop compiles on its first call.
  The #792 testcase at 100,000 calls takes 8.8 s instead of 14.3 s.
* A parked `wait(cond)` is re-checked only when something it reads may
  have changed, and class-member event waiters are indexed by event and by
  process (#793, PR #802, contributed by AaronKel): 2,000 parked watchers
  take 0.8 s instead of 75.4 s. `XEZIM_GROWTH_TRACE=<ticks>` prints the
  heap, signal and waiter table sizes over a long run.
* The class-heap collector traces from the previous survivors and the
  objects allocated since, so it runs every 20,000 allocations again
  (§8.29, #794, PR #801, contributed by AaronKel): 400,000 churned objects
  peak at 261 MB instead of 1,292 MB in the same time.
* A string without X or Z bits converts to text by whole bytes (PR core
  #137, contributed by AaronKel).
* An element of a nested collection is read and written through its
  declared shape, found with one lookup: 600,000 writes to `int d[][]` take
  16.0 G instructions instead of 18.2 G, and as many reads 8.2 G instead of
  14.2 G (§7.4.5, #772).
* An instance is elaborated against only the package parameters its
  definition can name, not a copy of the whole package, `$unit` and top
  parameter table (§23.10, §26.3, #740): with 4,000 package parameters no
  module names, compiling 1,000 instances took 375.6 G instructions before
  and 5.7 G after, and 250 instances 96.6 G and 4.0 G. The elaborated design
  is unchanged for every design under tests/ and examples/. Reported by
  git-hevangel.
* A frame local whose type metadata a deeper frame's same-named local
  displaced gets it back when that frame returns, and only then: the live
  frames were rescanned after every frame pop below the displacement, so a
  deep call chain running a loop of calls stalled for hours at one timestamp
  (§6.21, §13.3.1, #833, reported by Ganesh-TNL). `[PROF]
  local_meta_reapplies` counts the scans.
* An edge block that reads or writes one element of an unpacked array by a
  constant index is re-armed by writes to that element only. Every element
  of the array armed it, so N blocks each writing one element of a shared
  array (a generate loop of flops) marked N blocks per element write, N²
  per clock: 2,000 such blocks spent 23.1 s of a 24.8 s run in the NBA
  region, now 0.25 s (§9.2.2.4, §9.4.2). A RAM model's dynamic index keeps
  the whole-array arming.
* With no top named, a design whose only uninstantiated module stands
  beside a package, a `$unit` class or another non-instantiable declaration
  keeps the compilation-unit subroutine bodies in that module alone. The
  package or class counted as a second root, so every definition got every
  body again: 200 definitions and a 2,000-arm `$unit` function compiled in
  2,013 MB, now 56 MB, the same as with `-s top` (§23.3.1, §3.12.1, #834,
  reported by git-hevangel). Two or more uninstantiated modules still take
  the eager copy.
* A member of a struct resolution function's result, the `n.V =
  res('{...}).V` assign a struct nettype emits for each member, compiles:
  the function is inlined over one scalar per member (§6.6.7, §7.2, PR #837
  and core #153, contributed by ds54e). Each settle ran the resolver in the
  interpreter once per member, over 99% of a charge-pump regulator model's
  run. A struct with member defaults, or a call whose unused part has a side
  effect, keeps the call.
* An instance's parameter scope takes only what its own definition, the
  definitions it names and the scopes above it spell. The words of every
  package item and file-scope class, covergroup and typedef were added to
  every scope and to every prepared body's cache key, so a design whose
  packages spell most of their parameters gave every instance nearly the
  whole table again: 1,000 instances with 4,000 package parameters, 99 %
  spelled, compiled in 76.0 G instructions, now 2.0 G (§23.10, §26.3, #870,
  reported by git-hevangel). Package items are evaluated where they are
  declared, so they read the same values.
* The names a class constraint may read are looked up in the elaborated
  design as the constraint names them. Every class of every inlined
  instance copied every signal and parameter name of the design inlined so
  far into a set, also a class without constraints, so elaboration grew
  with the square of the instances: 2,000 instances of a module declaring a
  constrained class compiled in 53.7 G instructions, now 1.4 G (1.2 G
  without the class) (§18.3, #871, reported by git-hevangel). The names
  accepted are unchanged.
* The second-pass lint builds the last segments of the parameter keys and
  the map of global classes once per run. It split every parameter key for
  every width identifier and rebuilt the class map for every module:
  8,000 modules naming their own parameters in part-select widths took
  110.9 G instructions to compile, now 9.9 G, and 8,000 package classes
  41.3 G, now 1.7 G (§11.5.1, §8.18, #872, reported by git-hevangel). The
  verdicts are unchanged.
* A prepared body's cache key holds only the parameters of its scope that
  differ from the table, plus a shared bit mask of the ones absent from it,
  and repeated width bindings of unchanged typedefs are no longer journaled,
  so a large design keeps one copy of the common scope words instead of one
  per definition (§23.10, #721). With several uninstantiated modules and no
  top named, the first in source order carries the compilation-unit
  subroutine bodies instead of every definition getting a copy (§3.12.1,
  #834).
* A read of a declared module memory through a static hierarchical path
  inside an automatic task binds the memory directly instead of evaluating
  the instance prefix as a run-time receiver on every access; local, class,
  struct and typedef names that shadow it keep their meaning.
* The source line of a span is found by binary search over the line starts
  of its preprocessed file, taken once. Each lookup counted the newlines
  before the span, and setup locates every timing check of every cell
  instance in the cell library's file, so a gate-level netlist took time in
  the product of its timing checks and the library's size: 20,000 sky130
  cells against a 19 MB cell library took 55.7 s, now 8.5 s (#898, reported
  by Blebowski).

**Validation**

* xezim `603c9bf3`, pinned to xezim-core `212dde0`, passes 6,610 default
  tests with one existing ignored test, plus the two JIT smoke tests and the
  DPI tests on the nested-scheduler fallback. Core and parser pass 514 and
  233 tests. Both repositories pass their minimum supported Rust version
  checks. The release commits change only version numbers and the core pin.
* Artifacts written by 0.11.4 or earlier are refused with the "recompile
  with current xezim" message; recompile saved designs.
* Some `foreach` constraint shapes over large or chunked arrays can pick
  different solutions than 0.11.4 for the same seed (#955). Runs with one
  seed remain reproducible.

### 0.11.4 — class-object reclamation, linear elaboration, class and randomization correctness (October 2026)

**Correctness**

* Package type identity survives artifact reloads; nested specializations
  retain lexical value parameters and inherited type defaults. Collection,
  struct and mailbox patterns preserve member values and object lifetimes;
  foreign-function vector arguments and string results use the appropriate
  conversions and formatting (PRs #744–#762).

* Constants of a module's first instance see the struct typedefs the module
  declares in its body: a constant function's struct formal, local and
  result, a struct localparam's members, and a pattern actual of a struct
  formal read 0 or were packed by position there (§6.18, §7.2.1, #680,
  reported by git-hevangel); a subroutine local or associative key sized
  by a class parameter (`bit [N-1:0] x;` in `C #(N)`) takes the
  specialization's width (§8.25, #681, reported by Ganesh-TNL); a `randc`
  variable of up to 32 bits cycles, and a wider one warns that it is
  randomized as `rand` (§18.4.2, #699, reported by Ganesh-TNL); a second
  elaboration in the same thread no longer reads the previous one's
  constant-evaluation tables (#739, reported by git-hevangel).
* Class objects no handle reaches are reclaimed with their storage at a
  time advance, on by default (`XEZIM_CLASS_GC=0` turns it off), with
  collections spaced so their total cost stays linear in the objects
  allocated (§8.7, #722, PR #724, contributed by AaronKel).
* A class specialization argument written as an expression of package
  parameters (`.D(cfg_pkg::N + cfg_pkg::K)`) names the same type as the value
  it folds to, so a static or virtual-interface table set through one
  spelling is seen through the others (§8.25, #741, PR #742, contributed by
  auty1395).

* Randomization, arguments, compiled methods, fork state, interfaces and
  process memory (#698-#719, PR #720, contributed by AaronKel):
  associative-array values in `inside` constraints, fallback repairs of
  conditional and collection constraints, enum domains of rand dynamic
  collections, `$` subscripts through queue element member writes, formal
  widths for default and constant-bound arguments (`bit [64-1:0]`),
  selected array actuals and class-handle output formals (null at entry),
  caller collections and automatic variables shared by fork children,
  lexical enums and full object fields in compiled methods, `randomize`
  with inline constraints and no parentheses, `@(s.a or s.b)` on class
  properties, a user method named like a queue builtin in a `wait`,
  class field widths in a concatenation assignment, definition-qualified
  interface accesses in methods, and the execution caches and captured
  state of finished or killed processes released (memory no longer grows
  per process or per suspended `repeat` iteration). A whole-object
  `constraint_mode(0)` also turns off static blocks.
* Out-of-block methods and package scopes (xezim-core #115-#122, PR #123,
  contributed by AaronKel): an empty macro actual counts as one when the
  macro has formals, `ref` actuals are checked against the callee's own
  package signature, spaced implicit event controls survive attribute
  stripping, extern methods link through every package-class alias (now
  in place, not by copying the class per body), and macro line joining
  stays linear with backticks in strings ignored.
* Associative-array `first`/`next`/`last`/`prev`/`num` reuse the sorted
  key list until a key is added or removed, instead of rebuilding it per
  call (#723, PR #725, contributed by AaronKel).
* A relative instance reference inside nested generate scopes (#726), a
  directive-less package delay under `-timescale` (#728), a whole-array
  continuous assign with a hierarchical side (#730), a bind actual the
  target does not declare, an implicit net of the target (#732), and an
  associative array keyed by a class typedef over a class parameter
  (#734) (xezim-core #124-#128 and PRs #727-#735, contributed by auty1395).

* A bit-select of a subroutine formal or local declared with a range that
  does not start at zero (`input logic [12:1] a; a[i]`) reads the bit its
  label names: a compiled hot function read bit `i` of the storage, and a
  normalized select read a same-named design variable (an `initial`
  block's static `a`) instead of the formal (§11.5.1, §7.4.1, #650,
  reported by hevangel).
* Parser gaps (reported by amal-khailtash): a cross bin given as a list of
  value tuples, `'{'{1,2}, '{3,4}}` (§19.6.1.4, #651); `$changed(x, @clk)`
  (§16.9.3, #652); property and sequence declarations at compilation-unit
  scope (§3.12.1, #653); a class's out-of-block constraint inside its
  package (§18.5.1, #654); an unnamed formal in a DPI import or `extern`
  prototype (A.2.7, #655); a macro whose formal list spans lines with
  `` `ifdef `` in its body (§22.5.1, #656); `input a[4];` as a non-ANSI port
  (§7.4.2, #657); an empty `;` package item before `endpackage` (A.1.11,
  #658).
* A packed-struct assignment pattern written into a class property (`obj.p`,
  `this.p`, `super.p`, a bare `p` in a method, a property a derived class
  redeclares) or into a whole element of an instance's fixed array lays its
  members out at their declared offsets; the upper members were scrambled
  (§10.9.2, #587). Rand elements wider than 64 bits are drawn at full width,
  so the upper bits (a wide struct's leading `vld`) vary, and `sum()`
  targets and shares over wide elements keep every bit (§18.4, §7.12.3,
  #588). (PR #631, contributed by Ganesh-TNL)
* Class events, virtual interfaces, clocking blocks and class statics
  (#632-#648, PR #649 and xezim-core #107, contributed by AaronKel): a bare
  `@vif.cb` or `@h.ev` waits on the member (§9.4.2); a class event waits
  inside a `forever` loop, `ev.triggered` reads `this`'s event, and
  event-array elements and local events have their own identity (§15.5);
  a vif compares by binding through `this`, an indexed receiver, or the
  instance's own definition name, and binds when inherited through a
  base class's type parameter or given as a module-scope specialization's
  type argument (§25.9, §8.25, §23.8); a continuous assign may name its
  own instance by its definition name (§23.8); unpacked-array clockvars
  are sampled per element and a selected clockvar can be driven (§14.3,
  §14.16); a class static's initializer runs once; a string local no
  longer retypes a same-named formal.
* A macro usage built by a paste right before its argument list keeps its
  arguments (`` `uvm_pack_``TYPE``(ARG, p) ``), which the paste change of this
  release had broken for every `` `uvm_field_array_* `` macro (§22.5.1,
  xezim-core #102 and #103, contributed by urbanij).
* A struct formal's packed geometry comes from the active call's
  declaration, so two suspended tasks with a same-named formal of different
  struct types no longer read each other's layout (§13.5, PR #630). A
  packed array of a typedef'd type declared in a task (`e8_t [1:0] a;`)
  keeps its layout, on both paths.
* `##N (x)` and `##pkg::N (x)` read `N` as the cycle-delay count, not a
  call (§16.7, #609); a package's nets are nets: supply nets read their
  value and a `wand` resolves its drivers (§26.2, §6.6, #610); a coverpoint
  bin's `with (expr)` filter keeps only the values it admits (§19.5.1.1,
  #611); `restrict property` is read and set aside (§16.14.4, #614); `1step`
  is a delay value, one global time step in every delay form (§3.14.3,
  #615). Reported by git-hevangel.
* A macro usage ends at a `` `` `` paste (`` `JOIN(unit_`TAG, _n) `` is
  `unit_alpha_n`) unless the pasted name is itself a macro (§22.5.1, #616);
  the text after `` `undef NAME `` on its line is kept (§22.5.2, #619); a
  nested class's method may be defined out of block, `function int
  outer::inner::m()` (§8.24, #617); a property or sequence local may have
  unpacked dimensions (§16.10, #618). Reported by git-hevangel.
* A virtual interface binds to, and compares equal with, an interface
  instance nested in another (`o.leaf`, `o.arr[1]`) and a modport view of
  one (`o.leaf.mp`), assigned, passed, returned or stored in a static
  (§25.9, #612); a member nonblocking assignment updates the object its
  root names when it runs, whether the root is a class property, a module
  or block variable or a formal (§10.4.2, #613); a `$unit` wildcard import
  counts from where it appears, and the first reference binds a name both
  of two packages declare (§26.3, #620); `--relax-implicit-static` leaves
  an out-of-block method's locals automatic (§8.6, #621). Reported by
  git-hevangel.

* A compiled assignment pattern on a packed struct with a nested struct
  member packs its direct members only (§10.9.2); a named sibling such as
  `trans_id` read 0 (#589, PR #590, contributed by davekeeshan). `default:`
  fills each member of a nested struct member, on both paths.
* `always_comb` stays sensitive to a struct it reads whole beside a member
  write (#591), and to a module variable a nested callee reads when the
  caller has a formal of that name (#592) (§9.2.2.2.1); a compiled
  `super.m(...)` sizes its arguments by the parent's formals (§13.5.1,
  #593); a `foreach` constraint over an empty dynamic dimension adds no
  constraint (§18.5.8.1, #599). All four reported, with patches, by
  akashlevy.
* `d = new[n](src)`, a whole assignment and a by-value argument copy the
  rows of a dynamic array of collections (§7.5.1, #595); `other.h = new()`
  builds the class `other`'s specialization binds (§8.25, #596); `%p`
  prints a dynamic array's fixed rows from their left bound, a class member
  shadows a same-named module array in its methods, and `push_back` fills a
  descending row from its left bound (#597). Reported by akashlevy.
* A package may declare nets (§26.2, #602); a configuration declaration is
  set aside with a warning (§33.4, #603); `until_with` and `s_until_with`
  parse and check (§16.12.12, #604); a covergroup bin's `iff` guard parses
  and gates the bin (§19.5, #605); a macro's formal list may span lines
  (§22.5.1, #606); a `` `resetall `` after a module closed on its own line
  is legal (§22.3, #600). Reported by amal-khailtash.
* A labelled assertion keeps its label in the coverage results under
  `--code-coverage` (§9.3.1, #586, reported by hernloh).

* `$random` and the `$dist_*` functions run the standard's Annex N
  generators (§20.15), so a seed gives the sequence iverilog
  gives (`$random` with no seed starts 303379748). xezim used a generator
  of its own.
* `$monitor` of an automatic variable (a `for` or `foreach` loop variable,
  one declared `automatic`, a local of an automatic task or a method) and
  an ordered assignment pattern of the wrong size for an unpacked
  dimension (`'{1,2,3}` for a `[1:4]` row) are errors, as in
  iverilog (§6.21, §10.9.1). Both ran, and the monitor printed x.
* `foreach (a[i][j])` inside `foreach (a[i])` iterates the selected
  sub-array, and every loop variable runs in its dimension's declared
  direction (§12.7.3); `j` walked the first dimension again, and `[3:1]`
  ran 1 to 3.
* A fixed multi-dimensional class member can be assigned whole, by pattern
  and by row (`c.m = a`, `c.m[1] = '{..}`); every such write was dropped
  (§7.6).
* A write into an absent row of an associative array of collections
  (`aq[6][0] = 9`) creates the row, `%p` prints the rows as nested lists,
  `foreach (aq[k][j])` walks the row, and a string-valued row takes
  `m[k]["x"] = "hi"` (§7.8, §7.10). This holds at every depth
  (`byte a[string][int][string]` printed `'{"p":0}`) and for class
  members.
* Reading an output clockvar (`mif.cb.addr`) gives the driven value, as in
  Verilator, instead of x (§14.3).
* A variable index of a package array of packed structs selects an element
  (§7.4.1, §26.3): `ENC[fmt].exp_bits` in a package function was a one-bit
  select, so fpnew's `bias` read 2^32-1 and `1.0 + 2.0` came back as +Inf
  (#584, reported and fixed by davekeeshan, core #101 and #585). Also a
  fused `assign v[i]` on `logic [0:N]`, and `p::ENC[f].man_bits` read from
  a class method, which read 0.
* A bit, part or indexed part-select write to an absent associative
  element creates it from the element type's default (§7.8.6, §11.5.1);
  the write was dropped (#582, reported by Ganesh-TNL). An associative
  array returned from a class method (instance, virtual, static, or
  through the method's name) keeps its entries; it came back empty (#583,
  reported by Ganesh-TNL). `f()[k]` and `o.method().size()` on a returned
  collection read 0, and `%p` of a class member associative array of
  vectors printed decimal.
* White space, newlines included, may separate the size of a based literal
  from its base and the base from its value (§5.7.1, §5.3). A space before
  the apostrophe dropped the size with no message, so `2 'd1` was 32 bits
  wide, `3 'd 9` read 9 and a concatenation holding one was 32 bits too
  wide; a newline there was a parse error (#580, reported by git-hevangel).
* A macro defined with arguments may have its actual-argument list on a
  later line than its name, after blank lines or comments (§22.5.1). Each
  such call was rejected as "requires parentheses" (#578, reported by
  git-hevangel).
* A UDP instance may omit its name after a bare delay (`my_mux #0 (z, a,
  b, s);`, also from a macro), after a drive strength, and in a list
  (§29.3, #579, reported by git-hevangel). Its delay may also be a
  parameter (`#D`, which became an undelayed instance named `D`), a
  `min:typ:max` value, or a `(rise, fall)` pair, whose fall value was
  ignored (§28.16).
* `$bits` in a constant expression has the width of a cast's target
  (core #99, contributed by chenbhil), of a compound operand (an operator,
  conditional, call, or a concatenation holding a cast or a signal) and of
  an element or slice of a declared variable (`$bits(pk[2])` of
  `logic [3:0][1:0] pk` is 2). Each was 0 or one bit, so a declaration
  sized by it was one bit wide (§20.6.2).
* `h.p`, with `p` redeclared in a derived class, resolves by the static
  type of `h` while another process is suspended in a subroutine whose
  formal of another class type is also named `h` (§8.14). A UVM driver's
  `item.accept_time` landed in `uvm_transaction`'s private copy while the
  sequence waited in `finish_item` (#577, contributed by chenbhil).
* A class member typed by a class type parameter randomizes in the type
  the object binds (§8.25, §18.4). A `rand T beats[]` (dynamic, fixed,
  queue or associative) bound to a wider type than `T`'s default solved its
  elements at the default width, so a constraint on a field of the element
  (`beats[0].en == 1`, also under `foreach`, `dist`, `soft` or an
  implication) failed randomize(); a `rand T h` bound to a class was drawn
  as a number instead of randomizing the object, which left the handle
  dangling; `o.h = new()` and `o.arr[0] = new()` from outside the class
  built an object of class `T`; and a packed 2-D property through the
  parameter dropped element writes (#576).
* A class member with a dynamic or queue outer dimension over unpacked rows
  (`bit [7:0] raw[][2]`, `int q3[][2][3]`, `T cells[$][2]`, `int d[][]`)
  works (§7.4.5, §7.5, §18.4). It was a dynamic array of scalars:
  `o.raw[0][1] = 9` wrote nothing, `d[1] = new[3]` left one scalar row, a
  two-variable `foreach` saw no elements, the copy constructor dropped the
  rows, and a constraint on a row element (`cells[0][0].en == 1`,
  `d[i].size() == i + 1`, `foreach (q3[i, j, k])`) failed randomize(). Each
  object now shapes the member like a module-scope declaration of the same
  form, and the joint solver solves the outer size, then each row's size,
  then the elements (#576). `%p` of such a member, and of a module-scope
  variable of the same shapes, lists each row as a nested list; it printed
  `'{0, 0}`.
* `std::randomize` of a variable with a dynamic or queue outer dimension
  over unpacked rows (`bit [7:0] raw[][2]`, `int d[][]`), at module scope
  or as a task local, solves (§18.12, §7.4.5). The scope solve split
  `raw[][2]` into rows over its 64-slot buffer, so `raw.size() == 2` read 64
  and failed, and took `int d[][]` for a dynamic array of scalars. The
  outer size, then each dynamic row's size (`d[i].size()`), are solved
  before the elements.
* An unsized `'1` or `'0` in a constraint fills the width of the other
  operand (§5.7.1, §18.3). The joint solver and the constraint check took
  `'1` as the 1-bit value 1, so `y == '1` on an 8-bit `y` gave `8'h01` and
  `y != '1` admitted `8'hff`, in class constraints and `std::randomize`.
* A cast to a typedef in a constant expression takes the typedef's width
  and signing (§6.24.1): `localparam int A = q_t'(45)` with a 4-bit `q_t`
  is 13, and the same holds for a declared width, a generate `if` or
  `case`, a header parameter default and a parameter override. It works
  for package and `$unit` typedefs, a child's own typedefs, and the type
  parameters of a module, the root included. A `$unit` constant function
  casts to the `$unit` typedef even when the calling instance declares one
  of the same name (core #98 and #575, contributed by chenbhil).
* The coverage results file (`XEZIM_COV_DB`) names a labelled `assert`,
  `assume` or `cover` site: `"name": "c_hit"` for `c_hit: cover property
  (...)` (§16.5, §16.14), so a tool matching cover points to a verification
  plan has a stable key. An unlabelled site has no `name` key (#573,
  contributed by hernloh). The name is also given for a module-level
  deferred assertion (§16.4), an unclocked `assert property`, and a labelled
  procedural statement (§9.3.1) that the parser hosts in a named block; the
  unclocked property's entry now carries its file, line and kind too, where
  it was keyed by a synthetic name and always tagged `cover`.
* IEEE 1800-2023 §8.17 constructor argument forwarding parses and runs:
  `extends Base(default)` hands a subclass constructor's arguments (implicit
  or explicit) to the base constructor, `function new(default);` declares
  the constructor with the base's argument list and forwards it, and
  `super.new(default)` forwards the formals. Each form was a parse error
  (core #74, #75).
* A multi-dimensional unpacked array is a randomize target (§18.4, §18.12):
  `std::randomize(t, a)` on a 2-D or 3-D local (static or automatic) and a
  `rand int m[2][2]` class member with inline constraints each solve, with
  `foreach (a[i]) foreach (a[i][j])` walking the declared bounds and
  `a[i][j]` naming the element. Such an array was turned away by the scope
  solve and barred the class solve, a nested `foreach` body passed the
  inline check unexamined, a fixed multi-dimensional local kept only its
  first dimension, and a 1-D `std::randomize` run after one gave up because
  the two locals shared a shape entry (#519).
* Inside a class method, `a[i] = v` on a bare array name the class declares
  writes the object's member. A same-named module-level array's element
  was found in the signal table first and took the write, so a method's
  `foreach` fill left the member at 0 while the read side already named the
  member (§8.3). A class solve's element repair likewise no longer writes a
  bare module-level copy of a class property's element.
* The `--compile` path rewrites an intra-assignment delay spliced in by
  `include or a macro the way the simulate path does, so a compiled artifact
  of such a design keeps the timing control (§9.4.5, #567).
* A run that ends on a run-time error exits 1, as one that ends on `$fatal`
  does (§20.10): a read or a task call through a null handle in a process
  body, or a wait on a mailbox that was never constructed (§8.4); a `forever`
  with no timing control, or a zero-delay loop, that can never let time
  advance; a nonblocking-assignment cascade that does not converge; a
  `foreach` that makes no progress; an SDF file `$sdf_annotate` cannot read or
  parse; and a failed internal consistency check. Each printed its error and
  stopped the run, but the exit status was 0, so a script or harness that
  checks it took the run for a pass.
  (#529, contributed by akashlevy)
* A task or function call through a null class handle is reported (§8.4):
  the run aborts at the top level, as the reference simulator does; inside
  class code the call is reported once and skipped. It ran nothing and
  printed nothing, so a test whose handle was never constructed left no
  trace. (#532)
* A time-zero port handshake between two instances completes: the
  continuous-assign and port-connection updates a compile-time start
  writes are held until every start of the time-0 batch has run, as the
  reference simulator's active-region FIFO orders them, so an
  `always @(port iff ...)` in another instance is armed before the X→0
  settle fires (§4.4.2, §9.7.4). An output port an `initial` of the child
  writes is no longer aliased to the parent net. The hold is released when
  time leaves zero whichever loop popped the batch. (#550 and #551,
  contributed by Ganesh-TNL)
* A call actual takes the context width of the callee's declared scalar
  formal (§13.5.1, §11.6.1): `wide9(a + b)` against `logic [8:0] v`
  evaluates the sum at 9 bits in the interpreter and, on the compiled tier,
  for bare, `super.` and free-function calls. A class static operand of a
  comparison (`E::ta + E::tb`) is sized from its declaration (§11.8.2).
  (#553, #555, #557 and #559, contributed by Ganesh-TNL)
* A bare call inside a class method takes its return type from the
  enclosing class chain before any same-named module function or `extern`
  method of another class, so `index >= size()` compares signed (§8.20,
  §23.9). (#541, reported by urbanij)
* Class specializations are keyed by argument type, however the argument
  is spelled: an import-bare, package-qualified or typedef'd class name
  denotes one specialization (§8.25, §6.18). A static method called through
  a null handle declared as a specialization runs in that specialization;
  a self-referential member typedef follows it inside the class's own
  declaration only. The string-valued probe behind a comparison reads the
  current class's property, not a same-named string elsewhere, and is
  shadowed by the current method's locals alone (§8.22, §6.21). (#542, #544
  and #546, contributed by Ganesh-TNL)
* `$size`, `foreach` and a whole-array copy of a class array constant
  through a typedef of a specialization use that specialization's
  arguments (§8.25), and a method local or formal of the same name shadows
  the class constant (§23.9). (#534 and #535, reported by akashlevy)
* A class constant inherited through `extends Base#(...)` takes the derived
  specialization's arguments, through a chain of parameterized bases
  (§8.25, §6.20). (xezim-core #76, reported by opensource-elearning)
* An assertion `or` succeeds as soon as one operand holds while the other
  is pending with its nonvacuity established (§16.12.8, §16.14.8); such an
  attempt reported nothing and stayed live for the rest of the run.
  (#533, contributed by akashlevy)
* Reading an inactive member of a tagged union is an error (§11.9, §7.3.2);
  it read X silently. A whole-union copy carries the source's active tag,
  so the copy's active member reads without a spurious error. (#470,
  contributed by opensource-elearning)
* A name two packages declare resolves in a compiled block through the
  module's own imports (§26.3): CVA6's ALU compared `operation == ADD`
  against another package's `ADD` and never fired its arm.
* A part-select whose bounds are members of a struct-typed parameter
  (`q[Cfg.LW/Cfg.AW-1:1]`) is a constant select and picks the element, not
  a flat bit (§6.20.2, §11.5.1). A function formal is not in the enclosing
  block's sensitivity.
* An `always_comb` is not sensitive to the packed-struct members it writes
  itself, at member granularity (§9.2.2.2.1); CVA6's decoder, which writes
  `instruction_o.op` and reads it back in the same block, re-ran forever.
* A typedef declared inside a module is a different type in every instance
  (§6.18, §23.10): an inlined declaration kept the bare name and resolved to
  the first instance's type, so a member write into a wide struct array
  element landed at the wrong bit.
* `$display`, `$write` and `$sformatf` print an unescaped `%` and an
  unknown conversion specifier literally (§21.2.1); they dropped the `%`
  and the character after it, and an unknown specifier no longer evaluates
  the next argument. (#548, contributed by rpjayaraman)
* An omitted `foreach` variable before a queue, dynamic or associative
  dimension is rejected (§12.7.3); the loop walked the element's packed bits.
  (#536, reported by chenbhil)
* A dotted name is a member select from the first segment that names a data
  object, so `resp.exception.valid` beside a `begin : exception` block is
  accepted (§23.6, §7.2). (#527, reported by davekeeshan)
* `%p` prints the `'{...}` form of a packed struct in every context
  (§21.2.1.7): a class property through a handle, `this` or bare, a static
  or specialized-class property, an interface or virtual-interface member,
  an element of a queue, array or associative array of structs, a queue
  slice, a function result and a task formal or local each printed one flat
  decimal. Inside an aggregate the members now print as the majority of
  three commercial simulators prints them: 2/4-state vector types (`bit`,
  `logic`, `reg`, packed arrays of them) as `'h` hex with uppercase X/Z,
  `byte`, the integer atoms and vectors declared `signed` as signed decimal,
  an enum as its label (also from a package), a packed array member flat; a
  packed union prints its first member only and an unpacked union every
  member over the shared storage, a `real` member reinterpreted; `%0p`
  prints as `%p`; an associative array closes with a tight `}`. A
  standalone scalar stays plain decimal. Tests that pinned the earlier
  single-simulator forms (decimal members, a space before `}`, a compact
  `%0p`) were updated. (#566, reported by Ganesh-TNL)
* An unpacked-array formal of a class method named like a class array
  constant shadows it for `$size`, `foreach` and element reads (§13.5,
  §23.9); the bare name was lifted to the object's member storage. (#534)
* A comparison operand reached by walking down from a class handle in
  module scope — a packed-struct member (`e.st.sa`) or an element of an
  array property (`e.arr[0]`) — and a comparison whose left operand is a
  subroutine formal or local take the context width (§11.6.1, §11.8.2).
  (#564 and #565, reported by Ganesh-TNL)
* An intra-assignment delay written in an `include`d file, or in a macro
  defined in one, is a timing control like one in the compiled file
  (§9.4.5, §22.4): the pre-parse rewrite now also runs over the
  preprocessed text. (#567, reported by git-hevangel)
* A blocking write to the whole of a variable in a nested interface
  instance, by hierarchical name or virtual interface, lands in the signal
  table also after a `$monitor` or `$readmemh` has filled the name-keyed
  copy (§10.4.1, §23.6). (#568, reported by git-hevangel)
* A nonblocking assignment to a member or element of a class object named
  through a task formal or local updates the object named when the
  statement executed (§10.4.2, §4.9.4), not whichever object the name
  resolves to in the NBA region. (#569, reported by git-hevangel)
* A virtual interface assigned `outer.leaf`, an interface instance nested
  in another, is bound to `leaf` (§25.9, §23.6); it was read as a modport
  view of `outer`. (#570, reported by git-hevangel)
* `-override_timescale` scales a bare `#delay` in a package or
  compilation-unit task or class method once (§3.14.2); the delay-only pass
  scaled it a second time after the forced-timescale pass. (#571, reported
  by git-hevangel)
* An input port also passed to a sub-instance declared with a non-ANSI
  header keeps its declared width and packed-struct layout (§23.2.2.1,
  §23.3.3): the body-driven scan now reads a non-ANSI child's directions
  from its body declarations. (#572, reported by git-hevangel)
* A positional assignment pattern or an unpacked-array concatenation
  written as the actual of an unpacked-array port supplies the port's
  elements (§10.9.1, §10.10), including `pkg::t'{...}` struct literals.
  (xezim-core #96 follow-up)
* Port connections keep the formal's own width and labels when the actual
  is a part-select, a constant range, a whole net with a different declared
  range, or a fill literal (`'0`, `'1`): `input [11:2] p` reads `p[5:2]` by
  its labels and `$bits` of a port tied to `'0` is the formal's width
  (§7.4.1, §23.3.3, §5.7.1). (#540, #563 and xezim-core #94, contributed by
  davekeeshan and chenbhil)
* An unpacked array declared in an instance keeps its declared index order
  through a port (§7.6), and a variable of a typedef'd packed array
  declared in an instance keeps its element dimensions (§7.4.1).
  (xezim-core #88 and #92, contributed by davekeeshan)
* A typed assignment pattern may name its type through a package scope,
  `pkg::t'{...}` (§10.9.2); only a scoped name is taken as the type.
  (xezim-core #96 and #77, contributed by davekeeshan)
* A port actual whose first segment is the definition name of an enclosing
  instance resolves upward to that instance (§23.8), as a statement already
  did; a module bound into `dut` can write `dut.clk`. (#561, reported by
  Ganesh-TNL)
* Casts to typedefs, enums and an instance's own types fold inside constant
  functions (§13.4.3, §6.24.1); array parameters take replication, keyed
  and `default:` patterns, including a struct literal as the default of a
  nested array (§10.9.1, §6.20.2). (xezim-core #90 and #91, contributed by
  chenbhil)

**Build**

* xezim-core moves to zstd 0.14 (core #100): zstd 0.13 and zstd-safe 7
  carry RustSec advisories (RUSTSEC-2026-0391, -0395 to -0406) that fail
  `cargo deny` and OSV-Scanner on every build. None was reachable; the
  compressed format is unchanged.
* The compiled-artifact, prepared-comb and method-cache formats are encoded
  with bincode 2 (serde, standard configuration) instead of the unmaintained
  bincode 1.x (RUSTSEC-2025-0141). Artifacts written by earlier versions are
  refused with the usual "recompile with current xezim" message; the caches
  are rebuilt (#531, core #87).
* The same design writes the same artifact bytes on every run: the module
  timescale table was filled from a randomly ordered map, so one design
  wrote two to four distinct artifacts in six runs, and three serialized
  name sets were randomly ordered too.

**Performance**

* Elaboration cost is linear in the number of instances for `bind`
  directives in a replicated module (§23.11, #679), interface-port formals
  (§25.10, #736) and constant-function calls (§13.4.3, #737); each copied or
  scanned the design-wide parameter table per instance (4,000 instances
  with a bind: 49.3 G instructions before, 1.6 G after). Reported by
  git-hevangel.

**Command line**

* Opt-in switches, each off by default and warning where it applies, for
  sources written against tools that accept what the standard does not
  (feature requests from git-hevangel): `--relax-duplicate-decls` ignores a
  second declaration of a name in a subroutine or block (§3.13, #622),
  `--relax-enum-no-base` reads `enum [N:0]` as `enum bit [N:0]` (§6.19,
  #623), `--relax-static-const` reads `static const` as `const static`
  (§6.20.6, #624), `--relax-event-this` reads `@this.ev` as `@(this.ev)`
  (§9.4.2, #625), `--relax-casex-inside` reads `casex (e) inside` as
  `case (e) inside` (§12.5.4, #626), `--relax-macro-space` reads
  `` ` NAME `` as `` `NAME `` (§22.5.1, #627), `--relax-nul-bytes` reads a
  NUL byte as white space (§5.3, #628) and `--relax-cross-hier-item`
  crosses a hierarchical reference through an implicit coverpoint (§19.6,
  #629). Each has an `XEZIM_RELAX_*` environment variable and is part of
  the design cache key.
* `--keywords <version>` chooses the keyword set outside every
  `` `begin_keywords `` region, any §22.14 version, also with `-noconfig`,
  and `--no-config-keywords` reads `instance`, `cell`, `use`, ... as names
  (#608, reported by git-hevangel).

* A bare top name inside a `-f` argument file names a top, as on the
  command line (#601, reported by amal-khailtash).
* A compiled artifact named beside source files, or under `--compile`, is
  refused with an error; it was read as source text and panicked (#738,
  reported by git-hevangel).
* `--fst-repack` repacks a dump path that is a symbolic link into its
  target, and removes its temporary file when the rename fails (#598,
  reported by akashlevy).

* `--fst-repack[=N]` gzip-wraps the finished FST dump on a clean close, as
  GTKWave's repack-on-close does (deflate level N, 1..9, default 6): about
  1.5x smaller on top of the per-block LZ4, at the cost of one pass over
  the file at close and an inflate on open. A run that dies earlier leaves
  the unwrapped, crash-safe dump as before.

**Validation**

* xezim `fec70d6e`, pinned to xezim-core `8ebdf4a`, passes 5,838 default
  tests with one existing ignored test, plus the two JIT smoke tests. Core
  and parser pass 176 and 186 tests. Both repositories pass their minimum
  supported Rust version checks.
* Artifacts written by 0.11.3 or earlier are refused with the "recompile
  with current xezim" message (the format is now bincode 2); recompile saved
  designs.

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
