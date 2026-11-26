# Coverage in xezim

xezim collects **functional coverage** (covergroups, IEEE 1800 clause 19) and
**assertion coverage** (`cover` statements, plus pass/fail counts of assertions,
clause 16) in every simulation, and writes the results to a JSON file when the run
ends. When asked (`--code-coverage`), it also collects **code coverage**:
statement, branch and toggle counts, in the same file. There is no condition,
expression or FSM coverage.

This guide covers what is supported, how to write coverage that xezim collects, and
how to read the results. Code coverage has [its own section](#code-coverage).

---

## What's supported

| Area | Supported |
|---|---|
| Where covergroups live | Any module (the top or an instantiated one), an interface, a package, file scope, and classes. See [Where a covergroup can be declared](#where-a-covergroup-can-be-declared) |
| Sampling | A sampling event (`covergroup cg @(posedge clk)`, `@(posedge clk iff en)`, a named event `@(ev)`), explicit `sample()`, `with function sample(...)` arguments, constructor arguments (`covergroup cg (int lo, int hi)`) |
| Bins | Values and ranges (`{0, [2:5], [8:$]}`), automatic bins, array bins `name[]` and `name[N]`, `wildcard` bins, transition bins (`(1 => 2 => 3)`, `([0:1] => [2:3])`, sets `(1, 5 => 3)`, lists `(1 => 2), (5 => 6)`, repetition `[*n]`, `[->n]`, `[=n]`), `default`, `ignore_bins`, `illegal_bins` |
| Guards | `coverpoint x iff (cond)`, `cross a, b iff (cond)` |
| Crosses | Automatic cross bins (every combination of the coverpoints' bins); `bins`, `ignore_bins` and `illegal_bins` selected with `binsof(cp)`, `binsof(cp.bin)`, `intersect {...}`, `!`, `&&`, `\|\|` and `with (...)` |
| Options | `option.at_least`, `option.weight`, `option.auto_bin_max`, `type_option.merge_instances`, `type_option.weight` |
| Queries | `get_inst_coverage()` and `get_coverage()` on a covergroup, a coverpoint or a cross, with or without `(covered, total)`; `cg_type::get_coverage()`; `$get_coverage()`; `start()` and `stop()` |
| Assertion coverage | Counts for `cover property`, `cover sequence`, `assert property`, `assume property` and the immediate `cover`, `assert` and `assume` |
| Code coverage | Statement, branch and toggle coverage with `--code-coverage` (or `+cover`). See [Code coverage](#code-coverage) |
| Results | The `xezim_cov.json` file, and `[COV]` summary lines with `--verbose` |

---

## Enabling coverage

Nothing needs enabling. Covergroups and assertions are always compiled and
evaluated, and a run that has at least one covergroup instance or assertion writes
`xezim_cov.json` to the current directory when it ends:

```bash
xezim alu_cov.sv          # also writes ./xezim_cov.json
```

- `XEZIM_COV_DB=<path>` writes the file somewhere else. `XEZIM_COV_DB=/dev/null`
  skips it.
- `+fcover` is accepted and does nothing.
- Code coverage is off unless asked for: see
  [Enabling code coverage](#enabling-code-coverage). `+cover`, `+cover=<spec>`
  and `-coverage` turn it on.

---

## Writing coverage

Give every coverpoint and cross a label (`cp_op : coverpoint op`). Queries and the
results file use the label as the name. An unlabeled coverpoint on a variable is
named after the variable (`coverpoint data` is `cg.data`); one on any other
expression gets the name `coverpoint#<n>`, `<n>` counting the covergroup's
coverpoints from 1, which a query cannot spell.

### A covergroup in a module, sampled on a clock

```systemverilog
module tb;
  logic       clk = 0;
  logic       valid;
  logic [1:0] op;
  logic [3:0] a;

  always #5 clk = ~clk;

  // Sampled on every rising clock edge.
  covergroup alu_cg @(posedge clk);
    cp_op : coverpoint op iff (valid) {   // skipped while valid is 0
      bins add       = {0};
      bins sub       = {1};
      bins logic_ops = {[2:3]};
    }
    cp_a : coverpoint a {
      bins zero = {0};
      bins low  = {[1:7]};
      bins high = {[8:15]};
    }
  endgroup

  alu_cg cg = new();

  initial begin
    valid = 1;
    op = 0; a = 0; @(negedge clk);
    op = 1; a = 3; @(negedge clk);
    op = 0; a = 5; @(negedge clk);
    valid = 0; op = 3; a = 9; @(negedge clk);   // op not sampled
    $display("cp_op  = %0.2f%%", cg.cp_op.get_inst_coverage());
    $display("cp_a   = %0.2f%%", cg.cp_a.get_inst_coverage());
    $display("alu_cg = %0.2f%%", cg.get_inst_coverage());
    $finish;
  end
endmodule
```

```text
$ xezim alu_cov.sv
cp_op  = 66.67%
cp_a   = 100.00%
alu_cg = 83.33%
Simulation finished at time 40 ($finish called)
```

`logic_ops` was never hit: the only `op = 3` came while `valid` was 0. So `cp_op` has
2 of its 3 bins, and the group is the average of its two coverpoints.

To print coverage when the run ends, however it ends (`$finish`, `$fatal` or
`--max-time`), put the queries in a `final` block:

```systemverilog
final $display("alu_cg: %0.2f%%", cg.get_inst_coverage());
```

### A covergroup in a class, sampled by `sample()`

A covergroup declared in a class is created in the class constructor. Its
coverpoints read the object's properties.

```systemverilog
class packet;
  rand bit [3:0] len;
  rand bit [1:0] kind;

  covergroup pkt_cg;
    cp_len : coverpoint len {
      bins short_p = {[0:3]};
      bins mid_p   = {[4:11]};
      bins long_p  = {[12:15]};
    }
    cp_kind : coverpoint kind;   // automatic bins: one per value, 4 bins
  endgroup

  function new();
    pkt_cg = new();              // a class covergroup is created in new()
  endfunction
endclass

module tb;
  initial begin
    packet p = new();
    void'(p.randomize() with { len == 2;  kind == 0; });  p.pkt_cg.sample();
    void'(p.randomize() with { len == 13; kind == 1; });  p.pkt_cg.sample();
    $display("cp_len  : %0.2f%%", p.pkt_cg.cp_len.get_inst_coverage());
    $display("cp_kind : %0.2f%%", p.pkt_cg.cp_kind.get_inst_coverage());
    $display("pkt_cg  : %0.2f%%", p.pkt_cg.get_inst_coverage());
    $display("overall : %0.2f%%", $get_coverage());
  end
endmodule
```

```text
$ xezim pkt_cov.sv
cp_len  : 66.67%
cp_kind : 50.00%
pkt_cg  : 58.33%
overall : 58.33%
Simulation finished at time 0
```

To pass the values in at the sampling call instead, declare the covergroup with a
sample function. This is the usual form for a monitor or subscriber:

```systemverilog
covergroup txn_cg with function sample (bit [3:0] addr, bit wr);
  cp_addr : coverpoint addr { bins lo = {[0:7]}; bins hi = {[8:15]}; }
  cp_wr   : coverpoint wr;
endgroup
...
txn_cg.sample(addr, wr);
```

### Crosses and `ignore_bins`

```systemverilog
module tb;
  bit [1:0] mode;
  bit [1:0] size;
  bit       en;

  covergroup cfg_cg;
    cp_mode : coverpoint mode {
      bins m[] = {[0:3]};            // one bin per value: m[0] .. m[3]
      ignore_bins reserved = {3};    // mode 3 is never used: drop m[3]
    }
    cp_size : coverpoint size {
      bins narrow = {0, 1};
      bins wide   = {2, 3};
    }
    mode_x_size : cross cp_mode, cp_size iff (en);   // 3 x 2 = 6 bins
  endgroup

  cfg_cg cg = new();

  initial begin
    en = 1;
    mode = 0; size = 0; cg.sample();
    mode = 1; size = 2; cg.sample();
    mode = 2; size = 3; cg.sample();
    mode = 3; size = 0; cg.sample();   // ignored mode: no cross bin
    en = 0;
    mode = 0; size = 3; cg.sample();   // cross not sampled (iff)
    $display("cp_mode     %0.2f", cg.cp_mode.get_inst_coverage());
    $display("cp_size     %0.2f", cg.cp_size.get_inst_coverage());
    $display("mode_x_size %0.2f", cg.mode_x_size.get_inst_coverage());
    $display("cfg_cg      %0.2f", cg.get_inst_coverage());
  end
endmodule
```

```text
$ xezim cross_cov.sv
cp_mode     100.00
cp_size     100.00
mode_x_size 50.00
cfg_cg      83.33
Simulation finished at time 0
```

The ignored value takes `m[3]` out of `cp_mode`, so the cross has 3 x 2 = 6 bins.
Three of them are hit: `(m[0], narrow)`, `(m[1], wide)` and `(m[2], wide)`. The last
sample is skipped by the cross's `iff` but still counts for the coverpoints.

To cover chosen parts of a cross instead of every combination, give the cross its
own bins. Coverage is then the fraction of those bins that were hit:

```systemverilog
axb : cross cp_a, cp_b {
  bins a_lo = binsof(cp_a) intersect {[0:1]};
  bins a_hi = binsof(cp_a) intersect {[2:3]};
}
```

### How coverage is computed

- **Explicit bins:** a coverpoint's coverage is the number of its `bins` that were hit
  divided by the number of `bins`. `ignore_bins` and `illegal_bins` values are removed
  from the other bins (a sampled ignored value hits no bin), and a bin left with no
  values is dropped. A `default` bin is not counted.
- **Automatic bins** (a coverpoint with no `bins`): one bin per value of the sampled
  expression's width, or `option.auto_bin_max` (default 64) equal ranges when there
  are more values than that. A 4-bit coverpoint has 16 bins; an `int` has 64.
  A coverpoint of a `real` expression has no automatic bins; its explicit bins
  match the sampled value numerically (`bins lo = {[0.0:1.5]};`) (§19.5).
- **Array bins** `name[]`: one bin per value, named `name[<value>]`. `name[N]`: N bins
  `name[0]` to `name[N-1]`; the listed values, in order, are split evenly among them
  and the last one also takes the remainder.
- **Transition bins** `(1 => 2 => 3)`: hit when consecutive samples of the coverpoint
  match the sequence. Each step is a value set (values and ranges, `1, [3:4]`) with
  an optional repetition: `v [*n]` (n consecutive samples), `v [->n]` (n samples,
  not necessarily consecutive, ending on the last), `v [=n]` (the same, followed by
  any samples of other values), each also as `n:m`. A bin with a list
  `(1 => 2), (5 => 6)` is hit by any of them; an array bin `name[] = (...)` has one
  bin per value sequence.
- **Crosses:** every combination of the crossed coverpoints' bins. A crossed variable
  that has no coverpoint of its own gets automatic bins. A `bins` of the cross body
  is one bin holding the combinations its select expression picks; an `ignore_bins`
  or `illegal_bins` removes the combinations it picks; every other combination
  stays an automatic bin of its own. `binsof(cp) intersect {v}` picks the
  combinations whose `cp` bin holds a value in `{v}`, and `with (expr)` those with a
  value tuple for which `expr` holds.
- **Covergroup:** the average of its coverpoints and crosses, weighted by each
  one's `option.weight` (default 1).

A hit on an `illegal_bins` value or transition is a run-time error: xezim prints
`** Error: Illegal bin hit at value 7: cg.cp_s.bad` with the time and scope, like
`$error`, and `--error-exit` counts it. The run carries on.

### Options and multiple instances

```systemverilog
covergroup opt_cg;
  option.at_least = 2;               // a bin needs 2 hits to count
  cp_x : coverpoint x { bins b0 = {0}; bins b1 = {1}; }
  cp_y : coverpoint y {
    option.weight   = 3;             // counts 3x in the group average
    option.at_least = 1;             // overrides the group setting
    bins b0 = {0}; bins b1 = {1};
  }
endgroup
```

After samples `(x, y)` = `(0, 0)`, `(0, 1)`, `(1, 1)`, `cp_x` has 50.00 (`b1` was hit
only once), `cp_y` 100.00, and `opt_cg` (50 + 3 x 100) / 4 = 87.50.

| Option | Where | Effect |
|---|---|---|
| `option.at_least = N` | covergroup, coverpoint or cross | A bin (explicit, automatic or cross) counts as covered once it has N hits (default 1). A coverpoint's or cross's setting overrides the group's |
| `option.weight = N` | coverpoint or cross | Weight of the coverpoint or cross in its covergroup's average (default 1) |
| `option.auto_bin_max = N` | covergroup or coverpoint | Maximum number of automatic bins (default 64) |
| `type_option.merge_instances = 1` | covergroup, or `cg_type::type_option.merge_instances = 1;` at run time | `get_coverage()` adds up the instances' hits: a bin is covered once their total reaches `at_least`, and `get_inst_coverage()` returns the same number. Default 0: the average of the instances' coverages |
| `type_option.weight = N` | covergroup, or `cg_type::type_option.weight = N;` | Weight of the covergroup type in `$get_coverage()` (default 1) |

Every `new()` creates a separate instance with its own hits. `get_inst_coverage()`
reports one instance; `get_coverage()` reports the covergroup type:

```systemverilog
covergroup val_cg with function sample (bit [3:0] v);
  cp : coverpoint v { bins lo = {[0:1]}; bins hi = {[14:15]}; }
endgroup

val_cg a = new();
val_cg b = new();

initial begin
  a.sample(0);    // a hits lo
  b.sample(15);   // b hits hi
  $display("a %0.2f  b %0.2f  type %0.2f", a.get_inst_coverage(), b.get_inst_coverage(), a.get_coverage());
  val_cg::type_option.merge_instances = 1;
  $display("merged type %0.2f", a.get_coverage());
end
```

```text
a 50.00  b 50.00  type 50.00
merged type 100.00
```

### Assertion coverage

Every `cover`, `assert` and `assume` is counted, with no extra code. A
`cover property` action block runs on each match:

```systemverilog
module tb;
  logic clk = 0, rst = 1, req = 0, gnt = 0;
  always #5 clk = ~clk;

  // Concurrent cover: a request answered by a grant on the next clock.
  c_req_gnt : cover property (@(posedge clk) disable iff (rst) req ##1 gnt)
    $display("%0t: request granted", $time);

  // Concurrent assert: never a grant without a request on the clock before.
  a_gnt_has_req : assert property (@(posedge clk) disable iff (rst) gnt |-> $past(req))
    else $error("grant without request");

  // Immediate cover, evaluated on every clock.
  always @(posedge clk) c_waiting : cover (req && !gnt);

  initial begin
    @(negedge clk) rst = 0;
    @(negedge clk) req = 1;
    @(negedge clk) begin req = 0; gnt = 1; end
    @(negedge clk) gnt = 0;
    @(negedge clk) req = 1;
    @(negedge clk) begin req = 0; gnt = 1; end
    @(negedge clk) gnt = 0;
    $finish;
  end
endmodule
```

```text
$ xezim handshake.sv
35: request granted
65: request granted
Simulation finished at time 70 ($finish called)
```

The counts are in the results file; see [Assertion entries](#assertion-entries).

---

## Collecting and reading results

### The results file

When the run ends, by `$finish`, `$fatal`, `--max-time` or running out of events,
xezim writes `xezim_cov.json` to the current directory, replacing any earlier one.

- It is written only when the design has at least one covergroup instance or one
  assertion that was evaluated, or when the run collects code coverage. A run
  with none of them leaves an old `xezim_cov.json` as it was.
- `XEZIM_COV_DB=<path>` writes it to `<path>` instead. The directory must exist.
  When the file can't be written, xezim prints
  `[COV] warning: could not write <path>: <reason>` and the run's result is
  unchanged.

The file for the [module example](#a-covergroup-in-a-module-sampled-on-a-clock):

```json
{
  "sim_time": 40,
  "assertion_sites": 0,
  "assertion_pass_total": 0,
  "assertion_fail_total": 0,
  "assertions": [
  ],
  "covergroups": [
    {"name": "alu_cg", "samples": 4, "coverpoints": {"cp_op": 2, "cp_a": 4}, "crosses": {}, "bins": {"cp_a.zero": 1, "cp_op.add": 2, "cp_op.sub": 1, "cp_a.low": 2, "cp_a.high": 1}}
  ]
}
```

| Field | Meaning |
|---|---|
| `sim_time` | The time the run ended: the same number as the closing `Simulation finished at time N` line |
| `assertion_sites` | Number of entries in `assertions` |
| `assertion_pass_total`, `assertion_fail_total` | The sums of the `pass` and `fail` fields of `assertions` |
| `assertions` | One entry per assertion or cover statement; see below |
| `covergroups` | One entry per covergroup instance, in creation order |
| `covergroups[].name` | The covergroup type: `alu_cg`, `packet::pkt_cg` for one declared in class `packet`, or `u0.v_cg` for one declared in the module instantiated as `u0` (the instance path below the top module). Instances of one type share the name |
| `covergroups[].samples` | How many times the instance was sampled (sampling events plus `sample()` calls) |
| `covergroups[].coverpoints` | Per coverpoint, the number of **distinct values** sampled (not bins) |
| `covergroups[].crosses` | Per cross, the number of distinct value combinations sampled |
| `covergroups[].bins` | Hit count per explicit bin, keyed `<coverpoint>.<bin>`, `<coverpoint>.<bin>[<value>]` for `name[]` array bins and `<coverpoint>.<bin>[<index>]` for `name[N]`, and per bin of a cross body, keyed `<cross>.<bin>`. Includes `default` and `illegal_bins` bins |
| `code_coverage` | Only in a run that collects code coverage; see [The code coverage results](#the-code-coverage-results) |

Things the file does not hold for functional coverage:

- **Percentages.** Print them from the testbench with the
  [query functions](#query-functions), for example in a `final` block.
- **Bins that were never hit.** `alu_cg` above has no `cp_op.logic_ops` entry.
  Compare against the covergroup's source to find the holes.
- Automatic bins, of a coverpoint or a cross.

Key order in the objects can change from run to run. Compare files with
`jq -S . xezim_cov.json`, not with a plain `diff`.

#### Assertion entries

For the [assertion example](#assertion-coverage):

```json
  "assertions": [
    {"file": "handshake.sv", "line": 6, "span_start": 95, "kind": "cover", "pass": 2, "fail": 0},
    {"file": "handshake.sv", "line": 10, "span_start": 222, "kind": "assert", "pass": 6, "fail": 0},
    {"file": "handshake.sv", "line": 14, "span_start": 372, "kind": "cover", "pass": 2, "fail": 5}
  ],
```

| Statement | `pass` | `fail` |
|---|---|---|
| `cover property`, `cover sequence` | Matches | Always 0 |
| `assert property`, `assume property` | Attempts that succeeded, including vacuous ones (`gnt` low, so `gnt \|-> ...` holds) | Attempts that failed |
| Immediate `cover`, `assert`, `assume` | Evaluations where the expression was true | Evaluations where it was false |
| Deferred `assert #0`, `assert final` (and `assume`, `cover`) | Reports that matured with the expression true | Reports that matured with it false |

A deferred assertion counts a report only when it matures in the Observed
(`#0`) or Postponed (`final`) region of its time slot (§16.4.2). A report that
a flush point discards, because its process resumed in the same time slot,
or that a later evaluation of the same `always_comb` supersedes, is no
attempt.

An immediate `cover`'s false evaluations therefore count in `assertion_fail_total`
too. In the example, all 5 fails are clock edges where `req && !gnt` was false; no
assertion failed.

`file` and `line` locate the `cover` / `assert` / `assume` statement; `span_start`
is its position in that file after preprocessing. The entries are sorted by file
and position. An entry has no name.

All instances of a module share one entry: its counts are the sum over the
instances.

### The summary lines

`--verbose` (or `XEZIM_VERBOSE=1`) adds `[COV]` lines at the end of the run, on
stderr, together with the other engine lines:

```text
$ xezim --verbose alu_cov.sv 2>&1 | grep '^\[COV\]'
[COV] assertions: 0 sites (assert=0, assume=0, cover=0) — 0 passes, 0 fails
[COV] coverage: 1 covergroup instances, 4 samples, 6 unique coverpoint values, 0 unique cross tuples
[COV] wrote coverage DB to xezim_cov.json
```

The counts are totals over the whole design, with the same meanings as the file's
fields: "unique coverpoint values" adds up the file's `coverpoints` numbers (2 + 4).

### Query functions

All return a `real` percentage from 0.0 to 100.0; print it with `%f`, for example
`%0.2f`.

| Call | Returns |
|---|---|
| `cg.get_inst_coverage()` | Coverage of the instance `cg` |
| `cg.get_coverage()` | Coverage of the covergroup type: the average over its instances, or of their hits added up with `type_option.merge_instances = 1` |
| `cg.<cp>.get_inst_coverage()`, `cg.<cp>.get_coverage()` | The same for one coverpoint or cross `<cp>` |
| `cg_type::get_coverage()`, `cg_type::<cp>::get_coverage()` | The same as `get_coverage()` on an instance of type `cg_type` |
| `$get_coverage()` | The average over every covergroup type, weighted by `type_option.weight` |
| `cg.sample()`, `cg.sample(args)` | Samples the instance now |
| `cg.stop()`, `cg.start()`, `cg.<cp>.stop()`, `cg.<cp>.start()` | Stop sampling the instance (or one coverpoint or cross of it), and start it again |

Each coverage query also takes two `int` output arguments,
`get_inst_coverage(covered, total)`: they receive the number of covered bins and the
total number of bins, summed over the coverpoints and crosses for a covergroup.
`get_coverage` sums them over the instances, or counts the bins of their added-up
hits with `type_option.merge_instances = 1`. With `merge_instances`,
`get_inst_coverage()` returns the type's numbers too.

### Several runs

There is no merge tool: every run writes its own file. Give each run its own path,
for example one per seed:

```systemverilog
module tb;
  bit [3:0] len;
  covergroup len_cg;
    cp_len : coverpoint len {
      bins short_p = {[0:3]};
      bins mid_p   = {[4:11]};
      bins long_p  = {[12:15]};
    }
  endgroup
  len_cg cg = new();
  initial begin
    repeat (2) begin
      len = $urandom_range(0, 15);
      cg.sample();
    end
    $display("len_cg = %0.2f%%", cg.get_inst_coverage());
  end
endmodule
```

```text
$ mkdir -p cov
$ for s in 1 2 3; do XEZIM_COV_DB=cov/seed$s.json xezim +seed=$s len_cov.sv; done
len_cg = 66.67%
Simulation finished at time 0
len_cg = 66.67%
Simulation finished at time 0
len_cg = 33.33%
Simulation finished at time 0
```

The files hold hit counts per bin, so the bins hit across all runs can be added up.
For example, with `jq`:

```text
$ jq -r '.covergroups[] | .name as $cg | .bins | to_entries[] | "\($cg).\(.key) \(.value)"' cov/seed*.json \
    | awk '{h[$1] += $2} END {for (b in h) print b, h[b]}' | sort
len_cg.cp_len.long_p 2
len_cg.cp_len.mid_p 4
```

`short_p` was not hit by any seed. The files don't record the bins that were never
hit, so a combined percentage can't be worked out from them alone.

---

## Code coverage

With `--code-coverage`, xezim also counts which statements ran, which way each
branch went and which bits of each signal toggled, and writes the counts to
`xezim_cov.json` next to the functional coverage.

### Enabling code coverage

| Spelling | Collects |
|---|---|
| `--code-coverage` | Statement, branch and toggle coverage |
| `--code-coverage=<kinds>` | The kinds listed, separated by commas: `stmt` (or `statement`), `branch`, `toggle`, `all` |
| `XEZIM_CODE_COVERAGE=<kinds>` | The same, for scripts that cannot add a flag. `--code-coverage` wins |
| `+cover` | All three |
| `+cover=<letters>` | `s` statement, `b` branch, `t` toggle. Other simulators' other letters (`c` condition, `e` expression, `f` FSM, `x` extended toggle) are ignored with one warning, for example `Warning: +cover=sbcf: xezim has no condition (c), FSM (f) coverage; collecting statement, branch coverage` |
| `-coverage` | All three, unless a `+cover=<letters>` names fewer |

`--code-coverage-scope=<path>[,<path>...]` (repeatable) limits every kind to the
instances at or below the given paths (`tb.dut`, or `dut` for the instance of that
name below the top) and to the packages named. Without it the whole design is
covered, except the UVM library package `uvm_pkg` and the built-in `std` package;
name one of them in the scope to include it.

Code coverage is off by default, and then costs nothing: the design compiles and
runs exactly as it does without the feature. When it is on, statements and
branches are instrumented as the design is compiled, before xezim lowers it to
bytecode, so every execution path (compiled blocks, the two-state executors and
the interpreter) counts the same way, and the simulation's results do not
change. The instrumented run is slower, most of all for RTL:

| Run | Without | `--code-coverage` | `=stmt,branch` | `=toggle` |
|---|---|---|---|---|
| C906 RISC-V CPU running one CoreMark iteration (RTL) | 112 s | 298 s; 2.36x the instructions | 2.17x the instructions | 1.23x the instructions |
| AXI4 UVM testbench (`axi4_base_test`) | 7.0 s | 7.2 s; +0.1% instructions | - | - |

The C906 run's results file is 23 MB: 47,632 statements, 23,603 branch arms and
979,190 toggle bins in 1,773 scopes.

### An example

```systemverilog
module counter (input logic clk, rst, en, output logic [2:0] q);
  always_ff @(posedge clk)
    if (rst)
      q <= 0;
    else if (en)
      q <= q + 1;
endmodule

module tb;
  logic clk = 0, rst = 1, en = 0;
  logic [2:0] q;
  counter u_cnt (.clk, .rst, .en, .q);
  always #5 clk = ~clk;
  initial begin
    @(negedge clk) rst = 0;
    en = 1;
    repeat (3) @(negedge clk);
    $display("q = %0d", q);
    $finish;
  end
endmodule
```

```text
$ xezim --code-coverage cnt_cov.sv
q = 3
Simulation finished at time 40 ($finish called)
$ xezim --code-coverage --verbose cnt_cov.sv 2>&1 | grep '^\[COV\] code\|^\[COV\]  '
[COV] code coverage: statement 12/12 (100.00%), branch 2/3 (66.67%), toggle 10/18 (55.56%)
[COV]   tb (tb): statement 9/9 (100.00%), branch 0/0, toggle 7/12 (58.33%)
[COV]   tb.u_cnt (counter): statement 3/3 (100.00%), branch 2/3 (66.67%), toggle 3/6 (50.00%)
```

The first line is the design's totals, then one line per scope with its design
unit in parentheses (at most 50 scopes; the file has them all).

The counter's entry in `xezim_cov.json`:

```json
      {
        "scope": "tb.u_cnt",
        "design_unit": "counter",
        "kind": "instance",
        "statement": {"covered": 3, "total": 3, "percent": 100.00},
        "branch": {"covered": 2, "total": 3, "percent": 66.67},
        "toggle": {"covered": 3, "total": 6, "percent": 50.00},
        "statements": [{"file": "cnt_cov.sv", "line": 2, "kind": "always_ff", "count": 4}, {"file": "cnt_cov.sv", "line": 4, "kind": "statement", "count": 1}, {"file": "cnt_cov.sv", "line": 6, "kind": "statement", "count": 3}],
        "branches": [{"file": "cnt_cov.sv", "line": 3, "kind": "if", "arms": [{"line": 4, "arm": "if", "count": 1}, {"line": 6, "arm": "else if", "count": 3}, {"line": 3, "arm": "else (implicit)", "count": 0}]}],
        "toggles": [{"signal": "q", "width": 3, "rise": [2, 1, 0], "fall": [1, 0, 0]}]
      }
```

The flop ran on four clock edges: once in reset, three times counting. No edge
came with both `rst` and `en` low, so the implicit `else` of the `if` chain was
never taken. `q` went 0, 1, 2, 3: bit 2 never toggled and bit 1 never fell, so
3 of the counter's 6 toggle bins are covered. Its inputs `clk`, `rst` and `en`
are not listed: their toggles are the testbench's.

### What counts

#### Statements

- Every procedural statement of `initial`, `always`, `always_comb`, `always_ff`,
  `always_latch` and `final` blocks, tasks, functions and class methods counts
  each time it starts: assignments, calls, system task calls, `return`,
  `break`, `continue`, `disable`, `->`, `wait fork`, immediate assertions, and
  procedural `assign`, `deassign`, `force` and `release`.
- The `always` construct is a statement of its own, of kind `always`,
  `always_comb`, `always_ff` or `always_latch`: it counts each time its body
  starts, once per event for `always @(...)` and once per evaluation for
  `always_comb`. An `always` that starts with a delay (`always #5 clk = ~clk;`)
  counts when the delay has passed.
- A timing control and the statement it guards are two statements:
  `#1 a = 1;` counts the delay (kind `delay`) when it starts waiting and the
  assignment when it runs. `@(...)` is kind `event`, `wait (...)` kind `wait`.
- A loop (`for`, `foreach`, `while`, `do`, `repeat`, `forever`) counts once each
  time it starts; the statements of its body count once per iteration.
- `if` and `case` are not statements (they are [branches](#branches)), and
  neither are `begin`/`end`, `fork`/`join` and declarations. The statements
  inside them count.
- Each continuous assignment is a statement of kind `assign`: an `assign`
  (one per target when it lists several) and a net declaration assignment
  (`wire w = a & b;`). It counts each time xezim evaluates it.
- Not counted: the initializers of variables declared outside procedural code,
  port connections and gate primitives.

#### Branches

- `if`: one arm per condition of an `if` / `else if` chain, and one for the
  final `else`. A chain without a final `else` gets an `else (implicit)` arm
  that counts the times no condition held. A `unique`, `unique0` or `priority`
  `if` has no implicit arm, since no condition holding is a violation.
- `case`, `casez`, `casex` and `case inside`: one arm per case item, the
  `default` item included. An item listing several expressions (`0, 1: ...`) is
  one arm. A `case` without a `default` gets a `default (implicit)` arm, except a
  `unique`, `unique0` or `priority` one.
- `randcase`: one arm per item.
- `?:`: a `true` and a `false` arm, counted each time the expression is
  evaluated, in the value of an assignment (procedural or continuous) or a
  `return`, in the arguments of a task or function call, and in an `if`
  condition. A condition with x or z bits counts for neither.
- An arm is covered once its count is at least 1; branch coverage is the
  covered arms over all arms.

#### Toggles

- Each bit has two bins: a rise (0 to 1) and a fall (1 to 0). Transitions to or
  from x or z do not count.
- A bit is compared with its value at the end of the previous time slot, the way
  a waveform dump sees it: a signal that goes 0, 1, 0 within one time slot, even
  across delta cycles, does not toggle. The first comparison is with the value
  the signal starts with.
- Values a VPI client writes are sampled the same way, at the end of the time
  slot they are written in. One written at time 0 or before it (cocotb writes
  its first values from `cbStartOfSimulation` and that slot's
  `cbReadWriteSynch`) is the signal's time-0 value, the baseline its first
  toggle is counted from.
- Tracked: every net and variable of an integral type declared in a module or
  interface instance: `logic`, `bit` and `reg` vectors, nets, `int`, `integer`,
  `byte`, `shortint`, `longint`, enums and packed structs (both as plain bit
  vectors), and the members of unpacked structs. Output ports are tracked; when
  one is connected straight to a parent's signal, both names show the same
  counts.
- Not tracked: input and inout ports (the signal that drives one carries its
  toggles), unpacked arrays, queues, dynamic and associative arrays, `real`,
  `time`, `string`, `event` and class-handle variables, parameters, variables
  declared in tasks, functions and procedural blocks, and class properties.
- Counts are exact: they do not stop at 1.

#### Scopes

Code in a module body (generate blocks included) belongs to its instance. The
methods of a class and the tasks and functions of a package belong to a scope
named after the package (kind `package`); a class declared outside any package
or module belongs to `$unit` (kind `unit`), and one declared inside a module to a
scope named after the module (kind `module`), shared by all of its instances.

The signals declared in a generate block are listed in its instance under their
names below the block (`u_r[0].r`, `u_if.s`). The names of generate blocks and
instances, instance arrays included, are scopes, not signals, and have no
toggle bins.

#### Compared with other simulators

The rules above follow what other simulators report for the same constructs. The
differences:

- Counts of combinational code (`always_comb`, an `always` whose event
  control lists only level changes such as `@*` or `@(a or b)`, continuous
  assignments and the `?:` in them) are the number of times xezim evaluated
  it, which can be more than another simulator's count. Whether a statement
  or an arm was hit agrees.
- A loop counts once per start. Other simulators may count its iterations or
  its condition tests, and a `for` loop's step as a statement of its own.
- A `case` item listing several expressions is one arm. Other simulators can
  give each expression its own arm.
- An `always` that starts with a delay counts when the delay has passed, not
  when it starts, which can make its count one lower.
- A continuous assignment that another simulator optimizes into a gate
  (`assign y = r;`, `wire w = a ? b : c;`) is still a statement here, and its
  `?:` still a branch. So is the code of an instance whose inputs are
  constant, which another simulator may fold away.
- Enum variables toggle as bit vectors, not by value, and unused variables stay
  in the toggle list.
- Toggle counts are not capped at 1.

### The code coverage results

The results file gains a `code_coverage` member:

| Field | Meaning |
|---|---|
| `code_coverage.kinds` | The kinds collected: `statement`, `branch`, `toggle` |
| `code_coverage.statement`, `.branch`, `.toggle` | Totals over the design: `covered`, `total` and `percent` (two decimals; `null` when `total` is 0). Toggle totals count bins, two per bit |
| `code_coverage.scopes[]` | One entry per scope that has code: the instances by path, then the packages, `$unit` and the modules that declare classes |
| `scopes[].scope` | The instance path from the top (`tb.u_cnt`), or the package, `$unit` or module name |
| `scopes[].design_unit` | The module or interface of the instance; the scope's own name otherwise |
| `scopes[].kind` | `instance`, `package`, `unit` or `module` (see [Scopes](#scopes)) |
| `scopes[].statement`, `.branch`, `.toggle` | The scope's totals, like the design totals |
| `scopes[].statements[]` | Every counted statement, count 0 included, in source order: `file`, `line`, `kind` and `count`. `kind` is `statement`, `assign`, `always`, `always_comb`, `always_ff`, `always_latch`, `for`, `foreach`, `while`, `do`, `repeat`, `forever`, `delay`, `event` or `wait` |
| `scopes[].branches[]` | Every branch: `file`, `line`, `kind` (`if`, `case`, `casez`, `casex`, `case inside`, `randcase` or `ternary`) and `arms[]`, each with `line`, `arm` (`if`, `else if`, `else`, `else (implicit)`, `item`, `default`, `default (implicit)`, `true` or `false`) and `count` |
| `scopes[].toggles[]` | Every tracked signal: `signal` (its name within the scope), `width`, and `rise[]` and `fall[]` with one count per bit, bit 0 (the rightmost) first |
| `code_coverage.design_units[]` | One entry per module, interface or package: `design_unit`, `instances`, the totals, and `statements`, `branches` and `toggles` with the counts of all its instances added up. A statement, arm or bin is covered when any instance covered it |

Only the kinds collected appear. Several statements on one line are separate
entries with the same `line`.

With `--verbose`, the end of the run prints the `[COV]` lines of the
[example](#an-example) as well.

### Adding code coverage runs together

Each run's file lists every statement, arm and bit, hit or not, in the same order
for the same design, so the files of several runs add up. For example, one
random `case` selector per run:

```systemverilog
module tb;
  bit [1:0] op;
  int acc;
  initial begin
    op = $urandom_range(0, 3);
    case (op)
      0: acc = 1;
      1: acc = 2;
      2: acc = 3;
      default: acc = 4;
    endcase
    $display("op = %0d", op);
  end
endmodule
```

```text
$ mkdir -p cov
$ for s in 5 6 7; do XEZIM_COV_DB=cov/seed$s.json xezim --code-coverage=stmt,branch +seed=$s op_cov.sv; done
op = 2
Simulation finished at time 0
op = 3
Simulation finished at time 0
op = 0
Simulation finished at time 0
```

Each run covers 3 of the 6 statements and 1 of the 4 arms. Keyed by scope and
position, the statements of all three runs add up with `jq`:

```text
$ jq -s '[.[] | .code_coverage.scopes[] | .scope as $s | .statements | to_entries[]
          | {key: "\($s)#\(.key)", count: .value.count}]
         | group_by(.key) | map(map(.count) | add)
         | {covered: map(select(. > 0)) | length, total: length}' cov/seed*.json
{
  "covered": 5,
  "total": 6
}
```

and the branch arms the same way:

```text
$ jq -s '[.[] | .code_coverage.scopes[] | .scope as $s | .branches | to_entries[]
          | .key as $b | .value.arms | to_entries[]
          | {key: "\($s)#\($b)#\(.key)", count: .value.count}]
         | group_by(.key) | map(map(.count) | add)
         | {covered: map(select(. > 0)) | length, total: length}' cov/seed*.json
{
  "covered": 3,
  "total": 4
}
```

Only `1: acc = 2;` never ran. Toggles add up per bit in the same way, from
`.toggles[] | .rise` and `.fall`.

### Code coverage limits

- No condition, expression or FSM coverage, and no exclusions: every statement,
  arm and tracked bit counts.
- Instrumented code leaves some of xezim's fast paths: the optional native code
  generators, gate fusion for instrumented continuous assignments, table
  lookups for `case` statements, skipping a flop's clock edge when its inputs
  are unchanged, clock generators such as `always #5 clk = ~clk;`, and parallel
  evaluation. The results do not change; the run is slower.
- A function evaluated during elaboration (for a parameter value) counts only its
  calls during the run.
- Assertion action blocks, `randsequence` productions and covergroups are not
  instrumented, and a `?:` elsewhere than listed under [Branches](#branches)
  (in a system task's arguments, a loop condition or a `case` selector) is not
  a branch.
- A run from a precompiled artifact (`-o`) has no source text, so its entries
  have an empty `file` and line 0.
- The file lists every statement and tracked bit of the covered scopes. For a
  large design, cover the design under test only (`--code-coverage-scope`), or
  leave toggles out (`--code-coverage=stmt,branch`).

---

## Limits and unsupported features

**Not supported at all:**

- Condition, expression and FSM coverage. Code coverage is statement, branch
  and toggle only; see [Code coverage limits](#code-coverage-limits).
- Coverage databases in other formats (such as UCIS), text or HTML coverage
  reports, and merging results across runs (the files can be added up with
  `jq`: see [Several runs](#several-runs) and
  [Adding runs together](#adding-code-coverage-runs-together)).
- The coverage system functions `$coverage_control`, `$coverage_get`,
  `$coverage_get_max`, `$coverage_merge`, `$coverage_save`, `$set_coverage_db_name`
  and `$load_coverage_db`. They print `Warning: unknown system task '<name>' ignored`
  once, and the functions return 0.
- `coverage save` and `coverage report` in a `-do` script: ignored with a warning;
  the results are in `xezim_cov.json`.

#### Where a covergroup can be declared

A covergroup can be declared in any module (the top or one instantiated below it,
and every top of a design with several), an interface, a package, at file scope,
and in a class wherever the class is declared.

A covergroup declared in a module or interface is a separate type in each instance
of that module: `get_coverage()` averages the covergroup's instances within that
module instance only, and `$get_coverage()` counts each module instance's type
once. A covergroup declared in a package is one type however many modules create
it.

**Covergroup features that are accepted but have no effect, or only a partial one:**

| Feature | What happens |
|---|---|
| `bins name[] = {...} with (expr)` on a coverpoint | The `with` filter is ignored: the bin holds every listed value |
| `bins name = default sequence` | Counted like `default`: every sampled value no other value bin holds |
| An `ignore_bins` or `illegal_bins` transition | An `illegal_bins` transition is reported; neither removes the transition from other bins |
| Automatic bins of a signed coverpoint | They cover 0 to 2^width-1: a negative value falls in no automatic bin |
| `matches` in a cross bin select | Ignored |
| A file-scope covergroup created in a module instance | A separate type per module instance, like a covergroup declared in the module |
| `option.per_instance`, `option.goal`, `option.name`, `option.comment` | Readable as `cg.option.goal` (the covergroup's setting, a value written at run time, or the default), but they change nothing else. An option written at run time (`cg.option.at_least = 2`) does not change the coverage either, as in the reference simulator. The results file always has one entry per instance, named after the type |

---

## Quick reference

| Flag / variable | Effect |
|---|---|
| (none) | Covergroups and assertions are always collected. A run with any of them writes `./xezim_cov.json` |
| `XEZIM_COV_DB=<path>` | Write the results file to `<path>`; `/dev/null` to skip it |
| `--verbose`, `XEZIM_VERBOSE=1` | Print the `[COV]` summary lines on stderr, along with the other engine lines |
| `+seed=<n>` | Seed the random generator, for reproducible random stimulus and coverage |
| `--error-exit` | Exit nonzero after any `$error`, including one from an assertion's action block, and after an `illegal_bins` hit |
| `--code-coverage[=<kinds>]`, `XEZIM_CODE_COVERAGE=<kinds>` | Collect code coverage: `stmt`, `branch`, `toggle` or `all` (the default). See [Code coverage](#code-coverage) |
| `--code-coverage-scope=<path>[,<path>...]` | Only cover these instance subtrees and packages |
| `+cover`, `+cover=<letters>`, `-coverage` | Other simulators' spellings of `--code-coverage`: `s`, `b` and `t` select the kinds; other letters are ignored with a warning |
| `+fcover` | Accepted for compatibility with other simulators; no effect |
| `-do "coverage save ..."`, `-do "coverage report ..."` | Ignored with a warning |
