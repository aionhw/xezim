//! IEEE 1800-2017 §25.9, §23.6, §25.5: a hierarchical name of an interface
//! instance nested in another (`o.leaf`, `o.arr[1]`), or a modport view of
//! one (`o.leaf.mp`), binds a virtual interface and compares as the instance
//! it names, as a top-level instance's name does: assigned, passed as an
//! argument, returned, or stored in a static (#612, reported by
//! git-hevangel).
use xezim::simulate;

fn shown(src: &str, f: impl Fn(&str) -> bool) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| f(m))
        .collect()
}

#[test]
fn reported_testbench() {
    let out = shown(
        r#"// IEEE 1800-2017 25.9 / 23.6 / 25.5: a hierarchical name that names an
// interface instance nested in another instance (`o.leaf`, `o.arr[1]`), or a
// modport of one (`o.leaf.mp`), must bind a virtual interface and take part in
// a comparison exactly as the name of a top-level instance does. Every line
// should print PASS.
`timescale 1ns/1ns

interface leaf_if;
  bit         clk;
  logic       en;
  logic [7:0] d;
  logic [7:0] q;
  modport mp (input en, input q, output d);
  always #2 clk <= ~clk;       // negedges at 4, 8, 12, ...
endinterface

interface outer_if;
  leaf_if leaf();
  leaf_if arr[2]();
endinterface

class watcher;
  virtual leaf_if vif;
  time t_en = 0, t_clk = 0;
  task run();
    @(posedge vif.en);             t_en  = $time;
    repeat (2) @(negedge vif.clk); t_clk = $time;
  endtask
endclass

module t;
  outer_if o();                    // nested: o.leaf, o.arr[0], o.arr[1]
  outer_if o2();
  leaf_if  flat();                 // top level, for comparison
  leaf_if  flat_arr[2]();
  virtual leaf_if    vl, vf, vfa;
  virtual leaf_if.mp vm, vmf;
  watcher w_arr = new, w_flat = new;
  int fails = 0;

  task automatic chk(string what, logic [31:0] got, logic [31:0] exp);
    if (got === exp) $display("PASS %s", what);
    else begin $display("FAIL %s: got %0h, expected %0h", what, got, exp); fails++; end
  endtask
  task automatic chkt(string what, time got, time exp);
    if (got === exp) $display("PASS %s", what);
    else begin $display("FAIL %s: got %0t, expected %0t", what, got, exp); fails++; end
  endtask

  initial begin
    o.leaf.q = 8'h11;  o2.leaf.q = 8'h22;  flat.q = 8'h33;
    o.arr[1].d = 8'h44;  flat_arr[1].d = 8'h44;
    vl  = o.leaf;                  // bound to a nested instance
    vf  = flat;                    // bound to a top-level instance
    vm  = o.leaf.mp;               // a modport view of a nested instance
    vmf = flat.mp;                 // a modport view of a top-level instance
    vfa = flat_arr[1];             // an element of a top-level array

    // 1: the identity comparison names the instance on the right
    chk("flat:   vif == flat",            vf == flat,        1);
    chk("flat:   vif != o.leaf",          vf == o.leaf,      0);
    chk("nested: vif == o.leaf",          vl == o.leaf,      1);
    chk("nested: vif != o2.leaf",         vl == o2.leaf,     0);
    chk("nested: vif != flat",            vl == flat,        0);

    // 2: a modport view of a nested instance binds that instance
    chk("flat:   modport view reads q",   vmf.q,             8'h33);
    chk("nested: modport view reads q",   vm.q,              8'h11);
    vm.d = 8'h5a;
    chk("nested: modport view writes d",  o.leaf.d,          8'h5a);

    // 3: an element of a nested interface-instance array binds that element
    w_arr.vif  = o.arr[1];
    w_flat.vif = flat_arr[1];
    fork w_arr.run(); w_flat.run(); join_none
    #13 o.arr[1].en = 1; flat_arr[1].en = 1;          // posedge en at 13
    #20;
    chkt("flat:   element @(posedge vif.en) at 13",   w_flat.t_en,  13);
    chkt("flat:   element 2 x @(negedge clk) at 20",  w_flat.t_clk, 20);
    chkt("nested: element @(posedge vif.en) at 13",   w_arr.t_en,   13);
    chkt("nested: element 2 x @(negedge clk) at 20",  w_arr.t_clk,  20);
    chk("nested: element read through vif",          w_arr.vif.d,  8'h44);
    chk("nested: element == o.arr[1]",               w_arr.vif == o.arr[1], 1);
    chk("nested: element != o.arr[0]",               w_arr.vif == o.arr[0], 0);

    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#,
        |m| m.contains("PASS") || m.contains("FAIL"),
    );
    assert!(out.iter().any(|m| m == "ALL PASS"), "{out:?}");
    assert!(!out.iter().any(|m| m.contains("FAIL")), "{out:?}");
}

#[test]
fn argument_return_and_static_forms() {
    assert_eq!(
        shown(
            r#"interface leaf_if; logic [7:0] q; modport mp (input q); endinterface
interface outer_if; leaf_if leaf(); leaf_if arr[2](); endinterface
class holder;
  virtual leaf_if v;
  static virtual leaf_if sv;
  function void set(virtual leaf_if x); v = x; endfunction
endclass
module t;
  outer_if o();
  holder h = new, h2 = new;
  function automatic virtual leaf_if pick(); return o.arr[1]; endfunction
  function automatic virtual leaf_if pickmp(); return o.leaf.mp; endfunction
  initial begin
    virtual leaf_if r;
    o.leaf.q = 8'h11; o.arr[1].q = 8'h44;
    h.set(o.arr[1]);       h2.set(o.leaf.mp);
    holder::sv = o.arr[1];
    r = pick();
    $display("R %h %h %h %h %h", h.v.q, h2.v.q, holder::sv.q, r.q, pickmp().q);
    $display("E %0d %0d", h.v == o.arr[1], r == o.arr[0]);
  end
endmodule
"#,
            |m| m.starts_with("R ") || m.starts_with("E ")
        ),
        ["R 44 11 44 44 11", "E 1 0"]
    );
}
