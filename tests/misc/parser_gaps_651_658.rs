//! Parser gaps reported by amal-khailtash (#651-#658), each now read as
//! IEEE 1800-2023 allows.
use xezim::simulate;

fn shown(src: &str) -> Vec<String> {
    simulate(src, 100_000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with('R') || m.starts_with("PASS") || m.starts_with("FAIL"))
        .collect()
}

/// §19.6.1.4: a cross bin given as a list of value tuples (#651).
#[test]
fn cross_bin_tuple_list_651() {
    assert_eq!(
        shown(
            r#"module top;
  int a, b;
  covergroup cg;
    ca: coverpoint a { bins lo[] = {[0:4]}; }
    cb: coverpoint b { bins lo[] = {[0:4]}; }
    ab: cross ca, cb { bins tuples = '{'{1,2}, '{3,4}}; bins one = '{'{0,0}}; }
  endgroup
  cg c = new;
  initial begin
    a = 1; b = 2; c.sample();
    a = 3; b = 3; c.sample();
    $display("R %0.2f %0.2f", c.ab.get_coverage(), c.get_coverage());
  end
endmodule
"#
        ),
        ["R 8.33 29.44"]
    );
}

/// §16.9.3: `$changed(x, @clk)`, the bare clocking-event form (#652).
#[test]
fn bare_clocking_arg_652() {
    assert_eq!(
        shown(
            r#"module top;
  bit clk, s, r1, r2;
  always #5 clk = ~clk;
  initial begin
    #12 s = 1;
    #4 r1 = $changed(s, @clk); r2 = $changed(s, @(posedge clk));
    $display("R %0d %0d", r1, r2);
    #5 r1 = $changed(s, @clk); r2 = $changed(s, @(posedge clk));
    $display("R %0d %0d", r1, r2);
    $finish;
  end
endmodule
"#
        ),
        ["R 0 0", "R 0 0"]
    );
}

/// §3.12.1, A.1.11: a property and a sequence declared at compilation-unit scope (#653).
#[test]
fn unit_property_653() {
    assert_eq!(
        shown(
            r#"property implies_value(a, b);
  a |-> b;
endproperty
sequence two_high(x);
  x ##1 x;
endsequence
module top;
  bit clk, a, b; int fails = 0, hits = 0;
  always #5 clk = ~clk;
  assert property (@(posedge clk) implies_value(a, b)) else fails++;
  cover property (@(posedge clk) two_high(a)) hits++;
  initial begin
    @(negedge clk) a = 1; b = 1;
    @(negedge clk) a = 1; b = 0;
    @(negedge clk) a = 0;
    @(negedge clk);
    $display("R %0d %0d", fails, hits); $finish;
  end
endmodule
"#
        ),
        ["R 1 1"]
    );
}

/// §18.5.1: a class's extern constraint defined inside its package (#654).
#[test]
fn package_ooc_constraint_654() {
    assert_eq!(
        shown(
            r#"package p;
  class item;
    rand int value;
    extern constraint bounded;
  endclass
  constraint item::bounded { value inside {[1:4]}; }
endpackage
module top;
  import p::*;
  initial begin
    item it = new; int bad = 0;
    repeat (50) begin void'(it.randomize()); if (!(it.value inside {[1:4]})) bad++; end
    $display("R %0d", bad); $finish;
  end
endmodule
"#
        ),
        ["R 0"]
    );
}

/// A.2.7: unnamed formals in `extern` prototypes (#655).
#[test]
fn unnamed_prototype_formal_655() {
    assert_eq!(
        shown(
            r#"class c;
  extern function int f(int, bit [3:0]);
  extern task t(output int, input int);
endclass
function int c::f(int v, bit [3:0] w); return v + w; endfunction
task c::t(output int o, input int i); o = i * 2; endtask
module top;
  initial begin c x = new; int r; x.t(r, 5); $display("R %0d %0d", x.f(3, 4), r); end
endmodule
"#
        ),
        ["R 7 10"]
    );
}

/// §22.5.1: a macro with a multiline formal list and `ifdef in its body (#656).
#[test]
fn multiline_macro_formals_656() {
    assert_eq!(
        shown(
            r#"`define ENABLE_PROBE
`define DECLARE(tag,
               unused_arg) \
`ifdef ENABLE_PROBE \
  probe probe_``tag``(); \
`endif
module probe; initial $display("R %m"); endmodule
module top;
 `DECLARE(one, 0)
 `DECLARE(two,
          1)
endmodule
"#
        ),
        ["R top.probe_one", "R top.probe_two"]
    );
}

/// §7.4.2, A.2.3: `input a[4];` as a non-ANSI port with an unpacked dimension (#657).
#[test]
fn nonansi_unpacked_port_657() {
    assert_eq!(
        shown(
            r#"typedef logic [1:0] pair_t;
module sub(a, b, c, d);
  input a[4];
  input pair_t [2:0] b;
  input [3:0] c [2];
  output logic d;
  always_comb d = a[3] ^ b[2][1] ^ c[1][2];
endmodule
module top;
  logic a[4] = '{0, 0, 0, 1};
  pair_t [2:0] b = '{2'b10, 2'b00, 2'b00};
  logic [3:0] c [2] = '{4'h0, 4'h4};
  logic d;
  sub u(.a(a), .b(b), .c(c), .d(d));
  initial #1 $display("R %0d %0d %0d %0d", $bits(u.b), $size(u.a), u.d, $size(u.c));
endmodule
"#
        ),
        ["R 6 4 1 2"]
    );
}

/// A.1.11: an empty `;` package item, also before `endpackage` (#658).
#[test]
fn empty_package_item_658() {
    assert_eq!(
        shown(
            r#"package p;
 ;
 function void f(); endfunction
 ;
endpackage
module top; initial $display("R ok"); endmodule
"#
        ),
        ["R ok"]
    );
}

/// A.2.7: an unnamed formal in a DPI import prototype parses (#655).
#[test]
fn unnamed_dpi_formal_655() {
    simulate(
        r#"package p;
 import "DPI-C" function int external_fn(int);
endpackage
module top;
 initial $finish;
endmodule
"#,
        10,
    )
    .expect("simulate");
}
