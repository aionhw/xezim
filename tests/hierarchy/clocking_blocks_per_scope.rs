//! §14.3, §23.9, §27.4: a clocking block is a declaration of the scope that
//! declares it. Each copy of a generate block, and each instance of a module,
//! has its own block, which samples its own expressions and the signals of its
//! own scope, and `@(cb)` and `cb.v` in that scope name it, not a same-named
//! block of an enclosing scope. Every copy registered one block under the bare
//! name, so the last copy's expression answered all copies, and a block of the
//! top module answered `cb.v` in every instance (x when it had no such
//! clockvar). The expected values are those of the reference simulator.
use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

/// Two copies of a generate-loop block with an expression-bound clockvar.
#[test]
fn generate_copies_sample_their_own_expressions() {
    let out = lines(
        r#"
`timescale 1ns/1ns
module gen_child #(int N = 0) (input bit clk);
  int src [3] = '{10, 20, 30};
  int one = 1;
  for (genvar g = 0; g < 2; g++) begin : gb
    clocking cb @(posedge clk);
      input e = src[g] + g + N;
      input f = one + g;
      input h = one;
    endclocking
    initial begin
      @(cb); #1;
      $display("T|gen|g=%0d e=%0d f=%0d h=%0d", g, cb.e, cb.f, cb.h);
    end
  end
endmodule
module top;
  bit clk = 0;
  always #5 clk = ~clk;
  gen_child #(.N(100)) gc(.clk(clk));
  initial #20 $finish;
endmodule
"#,
    );
    assert_eq!(
        out,
        ["T|gen|g=0 e=110 f=1 h=1", "T|gen|g=1 e=121 f=2 h=1"],
        "{out:?}"
    );
}

/// Same-named blocks in the top module, an interface, two instances and three
/// generate copies, each reading the signal `a` of its own scope.
#[test]
fn same_named_blocks_of_instances_and_generate_copies() {
    let mut out = lines(
        r#"
`timescale 1ns/1ns
interface ifc(input bit clk);
  int a = 1;
  clocking cb @(posedge clk);
    input a;
  endclocking
endinterface
module leaf(input bit clk, input int base);
  int a;
  initial a = base;
  clocking cb @(posedge clk);
    input a;
  endclocking
  initial begin
    @(cb); #1 $display("T|%m s=%0d", cb.a);
  end
endmodule
module top;
  bit clk = 0;
  always #5 clk = ~clk;
  initial #100 $finish;
  int a = 50;
  ifc i(clk);
  leaf l1(clk, 10);
  leaf l2(clk, 20);
  clocking cb @(posedge clk);
    input a;
  endclocking
  for (genvar g = 1; g <= 3; g++) begin : gen
    int a = 100 + g;
    clocking cb @(posedge clk);
      input a;
    endclocking
    initial begin @(cb); #1 $display("T|gen%0d s=%0d", g, cb.a); end
  end
  initial begin
    @(cb); #1;
    $display("T|top s=%0d ifc s=%0d", cb.a, i.cb.a);
    #20 $finish;
  end
endmodule
"#,
    );
    out.sort();
    assert_eq!(
        out,
        [
            "T|gen1 s=101",
            "T|gen2 s=102",
            "T|gen3 s=103",
            "T|top s=50 ifc s=1",
            "T|top.l1 s=10",
            "T|top.l2 s=20",
        ],
        "{out:?}"
    );
}

/// The parent's block of the same name has none of the instance's clockvars:
/// `cb.s1` in the instance read x.
#[test]
fn instance_block_is_not_the_parents_block() {
    let mut out = lines(
        r#"
`timescale 1ns/1ns
module child(input bit clk, input int k, input int pk);
  int v = 5;
  int s6 = 66;
  clocking cb @(posedge clk);
    input s1 = v + 1;
    input s2 = k + 1;
    input s3 = pk + 1;
    input s4 = v;
    input s5 = pk;
    input s6;
  endclocking
  initial begin
    @(cb); #1;
    $display("T|%m s1=%0d s2=%0d s3=%0d s4=%0d s5=%0d s6=%0d", cb.s1, cb.s2, cb.s3, cb.s4, cb.s5, cb.s6);
  end
endmodule
module top;
  bit clk = 0;
  int x = 30;
  always #5 clk = ~clk;
  child c0(.clk(clk), .k(7), .pk(x));
  child c1(.clk(clk), .k(-3), .pk(x));
  clocking cb @(posedge clk);
    input x;
  endclocking
  initial #20 $finish;
endmodule
"#,
    );
    out.sort();
    assert_eq!(
        out,
        [
            "T|top.c0 s1=6 s2=8 s3=31 s4=5 s5=30 s6=66",
            "T|top.c1 s1=6 s2=-2 s3=31 s4=5 s5=30 s6=66",
        ],
        "{out:?}"
    );
}

/// Blocks of a generate-if block, of nested generate loops (with an
/// expression bound to the copy's own data), of a generate loop inside an
/// instance, output drives through instance and generate blocks, and
/// hierarchical references and waits on the blocks from the top.
#[test]
fn blocks_of_nested_scopes_and_hierarchical_references() {
    let mut out = lines(
        r#"
`timescale 1ns/1ns
module leaf(input bit clk, input int base);
  int a;
  logic [3:0] q = 0;
  initial a = base;
  clocking cb @(posedge clk);
    input a;
    output q;
  endclocking
  for (genvar k = 0; k < 2; k++) begin : lg
    int a = base + 1000 + k;
    clocking cb @(posedge clk);
      input a;
    endclocking
    initial begin @(cb); #1 $display("T|%m lg%0d s=%0d", k, cb.a); end
  end
  initial begin
    @(cb); cb.q <= base[3:0];
    #1 $display("T|%m s=%0d", cb.a);
    @(cb); #1 $display("T|%m q=%0d", q);
  end
endmodule
module top;
  bit clk = 0;
  always #5 clk = ~clk;
  int a = 50;
  leaf l1(clk, 10);
  leaf l2(clk, 20);
  clocking cb @(posedge clk);
    input a;
  endclocking
  if (1) begin : gi
    int a = 77;
    logic [7:0] w = 0;
    clocking cb @(posedge clk);
      input a;
      output w;
    endclocking
    initial begin @(cb); cb.w <= 8'h5a; #1 $display("T|gi s=%0d", cb.a); @(cb); #1 $display("T|gi w=%h", w); end
  end
  for (genvar g = 1; g <= 2; g++) begin : gen
    int a = 100 + g;
    for (genvar h = 0; h < 2; h++) begin : in
      int a = 200 + 10 * g + h;
      int bb = 7 * h + g;
      clocking cb @(posedge clk);
        input a;
        input b = bb + h;
      endclocking
      initial begin @(cb); #1 $display("T|gen%0d.in%0d s=%0d b=%0d", g, h, cb.a, cb.b); end
    end
    clocking cb @(posedge clk);
      input a;
    endclocking
    initial begin @(cb); #1 $display("T|gen%0d s=%0d", g, cb.a); end
  end
  initial begin
    @(cb); #1;
    $display("T|top s=%0d", cb.a);
    $display("T|hier l1=%0d l2=%0d gen1=%0d gen2in1=%0d gi=%0d l2lg1=%0d", l1.cb.a, l2.cb.a, gen[1].cb.a, gen[2].in[1].cb.a, gi.cb.a, l2.lg[1].cb.a);
    @(gen[2].cb); #1 $display("T|waited gen2 t=%0t", $time);
    #30 $finish;
  end
endmodule
"#,
    );
    out.sort();
    assert_eq!(
        out,
        [
            "T|gen1 s=101",
            "T|gen1.in0 s=210 b=1",
            "T|gen1.in1 s=211 b=9",
            "T|gen2 s=102",
            "T|gen2.in0 s=220 b=2",
            "T|gen2.in1 s=221 b=10",
            "T|gi s=77",
            "T|gi w=5a",
            "T|hier l1=10 l2=20 gen1=101 gen2in1=221 gi=77 l2lg1=1021",
            "T|top s=50",
            "T|top.l1 q=10",
            "T|top.l1 s=10",
            "T|top.l1.lg[0] lg0 s=1010",
            "T|top.l1.lg[1] lg1 s=1011",
            "T|top.l2 q=4",
            "T|top.l2 s=20",
            "T|top.l2.lg[0] lg0 s=1020",
            "T|top.l2.lg[1] lg1 s=1021",
            "T|waited gen2 t=16",
        ],
        "{out:?}"
    );
}

/// A default clocking block of each generate copy (`##2` and `dcb.a`), and a
/// task of the copy that waits on the copy's block, next to a same-named
/// block of the top module clocked on the other edge.
#[test]
fn default_blocks_and_tasks_of_generate_copies() {
    let mut out = lines(
        r#"
`timescale 1ns/1ns
module top;
  bit clk = 0;
  always #5 clk = ~clk;
  int a = 7;
  clocking cb @(posedge clk);
    input a;
  endclocking
  for (genvar g = 0; g < 2; g++) begin : gb
    int a = 10 * g + 1;
    default clocking dcb @(posedge clk);
      input a;
    endclocking
    clocking cb @(negedge clk);
      input a;
    endclocking
    task automatic show();
      @(cb);
      $display("T|g%0d task t=%0t a=%0d", g, $time, cb.a);
    endtask
    initial begin
      ##2;
      $display("T|g%0d cyc t=%0t a=%0d", g, $time, dcb.a);
      show();
    end
  end
  initial begin @(cb); #1 $display("T|top t=%0t a=%0d", $time, cb.a); end
  initial #60 $finish;
endmodule
"#,
    );
    out.sort();
    assert_eq!(
        out,
        [
            "T|g0 cyc t=15 a=1",
            "T|g0 task t=20 a=1",
            "T|g1 cyc t=15 a=11",
            "T|g1 task t=20 a=11",
            "T|top t=6 a=7",
        ],
        "{out:?}"
    );
}

/// Waits on and reads of the clocking blocks of generate copies and of an
/// instance-array element from outside, through the index (`@(gen[2].cb)`,
/// `gen[1].cb.a`, `u[0].cb.a`, `@(u[1].cb)`).
#[test]
fn blocks_reached_through_an_indexed_scope() {
    let out = lines(
        r#"
`timescale 1ns/1ns
module leaf(input bit clk);
  int a = 5;
  clocking cb @(posedge clk); input a; endclocking
endmodule
module top;
  bit clk = 0;
  always #5 clk = ~clk;
  leaf u [2] (clk);
  for (genvar g = 1; g <= 2; g++) begin : gen
    int a = 100 + g;
    clocking cb @(posedge clk); input a; endclocking
  end
  initial begin
    @(gen[2].cb); #1;
    $display("T|t=%0t gen1=%0d gen2=%0d u0=%0d", $time, gen[1].cb.a, gen[2].cb.a, u[0].cb.a);
    @(u[1].cb); #1 $display("T|t=%0t", $time);
    $finish;
  end
  initial #100 $finish;
endmodule
"#,
    );
    assert_eq!(out, ["T|t=6 gen1=101 gen2=102 u0=5", "T|t=16"], "{out:?}");
}
