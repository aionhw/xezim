//! IEEE 1800-2023 §8.3, §18.5 (#721): a class declared in a module is
//! elaborated once per instance of the module, and the copies hold the
//! constraint blocks of the one declaration instead of a copy each. A
//! `randomize()` call solves a working copy of its own, so the bounds an
//! instance's parameters and variables give a block, a `constraint_mode()`
//! switch, an in-line block and an out-of-block body (§18.5.1) stay those of
//! the one class. Expected lines are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
module worker #(parameter int LIMIT = 3, parameter int ID = 0);
  bit mode;
  int floor_v = LIMIT * 10;
  class packet;
    rand bit [7:0] value;
    rand bit [7:0] other;
    constraint permitted { value inside {[LIMIT-2:LIMIT]}; }
    constraint ordered { other > value; other < floor_v + 40; }
    constraint moded { if (mode) other[0] == 1'b1; else other[0] == 1'b0; }
    static constraint fixed { value != LIMIT - 1; }
  endclass
  class base;
    rand int lo;
    int width = 5;
    constraint low { lo inside {[LIMIT:LIMIT + width]}; }
  endclass
  class derived extends base;
    rand int hi;
    int width = 100;
    constraint high { hi > lo; hi < LIMIT + width; }
  endclass
  class outer;
    rand packet inner;
    rand bit [7:0] top_v;
    constraint link { top_v == inner.value + 1; }
    function new(); inner = new(); endfunction
  endclass
  if (ID == 0) begin : g
    class gen_c;
      rand bit [7:0] w;
      constraint wc { w inside {[LIMIT + 20:LIMIT + 22]}; }
    endclass
    gen_c gh;
    initial begin
      gh = new();
      repeat (8) begin
        if (!gh.randomize()) $display("FAIL %0d gen randomize", ID);
        if (gh.w < LIMIT + 20 || gh.w > LIMIT + 22) $display("FAIL %0d gen bound %0d", ID, gh.w);
      end
      $display("T|%0d gen done", ID);
    end
  end
  packet item;
  derived d;
  outer o;
  int bad;
  initial begin
    #(ID + 1);
    item = new(); d = new(); o = new();
    bad = 0;
    repeat (16) begin
      if (!item.randomize()) bad++;
      if (item.value < LIMIT - 2 || item.value > LIMIT || item.value == LIMIT - 1) bad++;
      if (item.other <= item.value || item.other >= floor_v + 40 || item.other[0] != mode) bad++;
    end
    $display("T|%0d packet bad=%0d", ID, bad);
    mode = 1; bad = 0;
    repeat (16) begin
      if (!item.randomize()) bad++;
      if (item.other[0] != 1'b1) bad++;
    end
    $display("T|%0d moded bad=%0d", ID, bad);
    bad = 0;
    repeat (16) begin
      if (!d.randomize()) bad++;
      if (d.lo < LIMIT || d.lo > LIMIT + 5) bad++;
      if (d.hi <= d.lo || d.hi >= LIMIT + 100) bad++;
    end
    $display("T|%0d derived bad=%0d", ID, bad);
    bad = 0;
    repeat (8) begin
      if (!o.randomize()) bad++;
      if (o.top_v != o.inner.value + 1) bad++;
      if (o.inner.value < LIMIT - 2 || o.inner.value > LIMIT) bad++;
    end
    $display("T|%0d outer bad=%0d", ID, bad);
    bad = 0;
    repeat (8) begin
      if (!item.randomize() with { value == LIMIT; }) bad++;
      if (item.value != LIMIT) bad++;
    end
    $display("T|%0d inline bad=%0d", ID, bad);
    item.value = LIMIT - 1; item.other = LIMIT + 1;
    $display("T|%0d check static-violating=%0d", ID, item.randomize(null));
    if (ID == 1) begin
      item.permitted.constraint_mode(0);
      item.fixed.constraint_mode(0);
    end
    #10;
    item.value = LIMIT - 1; item.other = LIMIT + 1;
    $display("T|%0d check after mode: %0d permitted=%0d fixed=%0d", ID, item.randomize(null),
             item.permitted.constraint_mode(), item.fixed.constraint_mode());
  end
endmodule
package p;
  class pc;
    rand bit [7:0] v;
    extern constraint body;
    constraint even { v[0] == 1'b0; }
  endclass
  constraint pc::body { v inside {[4:7]}; }
  class spec #(parameter int W = 4);
    rand bit [7:0] s;
    constraint cap { s < W; s > W - 3; }
  endclass
endpackage
module top;
  import p::*;
  class tc;
    rand int x;
    extern constraint b;
  endclass
  constraint tc::b { x inside {[70:71]}; }
  pc hp; tc ht; spec #(4) s4; spec #(200) s200;
  int bad;
  initial begin
    #50;
    hp = new(); ht = new(); s4 = new(); s200 = new();
    repeat (12) begin
      if (!hp.randomize() || !ht.randomize() || !s4.randomize() || !s200.randomize()) bad++;
      if (hp.v < 4 || hp.v > 7 || hp.v[0]) bad++;
      if (ht.x < 70 || ht.x > 71) bad++;
      if (s4.s >= 4 || s4.s <= 1) bad++;
      if (s200.s >= 200 || s200.s <= 197) bad++;
    end
    $display("T|top extern and specializations bad=%0d", bad);
  end
  worker #(.LIMIT(3), .ID(0)) first();
  worker #(.LIMIT(9), .ID(1)) second();
  worker #(.LIMIT(50), .ID(2)) third();
  initial begin #100; $display("T|done"); $finish; end
endmodule
"#;

#[test]
fn shared_constraint_blocks_solve_in_each_instance_scope() {
    let sim = simulate(SRC, 1000).expect("simulate");
    let mut lines: Vec<&str> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|l| l.starts_with("T|"))
        .collect();
    lines.sort_unstable();
    assert_eq!(
        lines,
        [
            "T|0 check after mode: 0 permitted=1 fixed=1",
            "T|0 check static-violating=0",
            "T|0 derived bad=0",
            "T|0 gen done",
            "T|0 inline bad=0",
            "T|0 moded bad=0",
            "T|0 outer bad=0",
            "T|0 packet bad=0",
            "T|1 check after mode: 0 permitted=0 fixed=0",
            "T|1 check static-violating=0",
            "T|1 derived bad=0",
            "T|1 inline bad=0",
            "T|1 moded bad=0",
            "T|1 outer bad=0",
            "T|1 packet bad=0",
            "T|2 check after mode: 0 permitted=1 fixed=1",
            "T|2 check static-violating=0",
            "T|2 derived bad=0",
            "T|2 inline bad=0",
            "T|2 moded bad=0",
            "T|2 outer bad=0",
            "T|2 packet bad=0",
            "T|done",
            "T|top extern and specializations bad=0",
        ]
    );
}

/// §18.5.3: two instances of one module draw from their own ranges call
/// after call; a value memoized in the shared block would leak across.
#[test]
fn alternating_instances_keep_their_own_bounds() {
    let sim = simulate(
        r#"
module unit #(parameter int LIMIT = 3);
  class packet;
    rand bit [7:0] value;
    constraint permitted { value inside {[LIMIT-2:LIMIT]}; }
  endclass
  packet item = new();
  function automatic int draw();
    if (!item.randomize()) return -1;
    return item.value;
  endfunction
endmodule
module top;
  unit #(.LIMIT(3)) first();
  unit #(.LIMIT(90)) second();
  int a, b, bad;
  initial begin
    repeat (40) begin
      a = first.draw();
      b = second.draw();
      if (a < 1 || a > 3) bad++;
      if (b < 88 || b > 90) bad++;
    end
    $display("T|bad=%0d", bad);
    $finish;
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    assert!(sim.output.iter().any(|o| o.message == "T|bad=0"));
}
