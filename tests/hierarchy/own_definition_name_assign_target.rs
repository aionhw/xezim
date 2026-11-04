//! §23.8, §10.3: a continuous-assign target spelled through the enclosing
//! module's or interface's own definition name (`assign parent_if.kids[0].clk
//! = clk;` inside `parent_if`) names this instance's own object. The target
//! bound to a signal literally named `parent_if.…`, so the assign drove
//! nothing and the net stayed `x`; an operand spelled the same way never
//! woke the assign. Procedural reads and writes through the
//! same spelling already resolved. Each instance drives its own copy.
//! Expected values are the reference simulator's.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn own_definition_name_assign_target_drives_this_instance() {
    const SRC: &str = r#"
interface child_if;
  logic clk;
endinterface
interface parent_if(input bit clk, input bit [1:0] id);
  child_if kids[2]();
  logic a;
  logic [1:0] b, c;
  assign parent_if.kids[0].clk = clk;
  assign parent_if.a = ~clk;
  assign {parent_if.b[0], parent_if.c} = {clk, id};
  for (genvar i = 1; i < 2; i++) begin : g
    assign parent_if.kids[i].clk = id[0];
  end
endinterface
module m(input bit clk);
  logic a;
  assign m.a = clk;
endmodule
module top;
  bit clk = 0;
  always #5 clk = ~clk;
  parent_if p0(clk, 2'd2);
  parent_if p1(~clk, 2'd1);
  m u(clk);
  initial begin
    #6;
    $display("T|p0 kid0=%b kid1=%b a=%b b0=%b c=%0d", p0.kids[0].clk, p0.kids[1].clk, p0.a, p0.b[0], p0.c);
    $display("T|p1 kid0=%b kid1=%b a=%b b0=%b c=%0d", p1.kids[0].clk, p1.kids[1].clk, p1.a, p1.b[0], p1.c);
    $display("T|m a=%b", u.a);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        t_lines(SRC),
        [
            "T|p0 kid0=1 kid1=0 a=0 b0=1 c=2",
            "T|p1 kid0=0 kid1=1 a=1 b0=0 c=1",
            "T|m a=1",
        ]
    );
}

/// The read side: an operand spelled through the own definition name
/// (`assign ready = parent_if.kids[0].ready;`) must feed the assign and
/// wake it when the operand changes.
#[test]
fn own_definition_name_assign_operand_reads_this_instance() {
    const SRC: &str = r#"
interface child_if;
  logic ready;
endinterface
interface parent_if(input bit en);
  child_if kids[2]();
  wire ready, both;
  wire [1:0] pair;
  assign ready = parent_if.kids[0].ready;
  assign both = parent_if.kids[0].ready & parent_if.kids[1].ready & parent_if.en;
  assign pair = {parent_if.kids[1].ready, kids[0].ready};
endinterface
module top;
  parent_if p0(1'b1);
  parent_if p1(1'b0);
  initial begin
    p0.kids[0].ready = 1; p0.kids[1].ready = 1;
    p1.kids[0].ready = 0; p1.kids[1].ready = 1;
    #1 $display("T|p0 ready=%b both=%b pair=%b", p0.ready, p0.both, p0.pair);
    $display("T|p1 ready=%b both=%b pair=%b", p1.ready, p1.both, p1.pair);
    p0.kids[1].ready = 0;
    p1.kids[0].ready = 1;
    #1 $display("T|p0 ready=%b both=%b pair=%b", p0.ready, p0.both, p0.pair);
    $display("T|p1 ready=%b both=%b pair=%b", p1.ready, p1.both, p1.pair);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        t_lines(SRC),
        [
            "T|p0 ready=1 both=1 pair=11",
            "T|p1 ready=0 both=0 pair=10",
            "T|p0 ready=1 both=0 pair=01",
            "T|p1 ready=1 both=0 pair=11",
        ]
    );
}
