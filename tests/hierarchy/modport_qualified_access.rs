//! §25.5: a reference through a modport of an interface instance names the
//! instance's own variable: whole and member reads and writes at module
//! level, a compiled read and partial nonblocking write in `always` blocks, a
//! task, a class method and a module that took the modport as its port
//! (#1050). Expected values are the reference simulator's.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).unwrap_or_else(|e| panic!("rejected: {e}"));
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[test]
fn modport_qualified_references_reach_the_interface_variable() {
    assert_eq!(
        lines(
            r#"typedef struct packed { logic vld; logic [6:0] d; } p_t;
interface ifc (input logic clk);
  p_t b; logic [7:0] w;
  modport mp (output b, w, input clk);
  modport rd (input b, w);
endinterface
module sub (ifc.mp port);
  task automatic put(input p_t v); port.b <= v; endtask
endmodule
module t;
  logic clk = 0;
  ifc i (clk);
  sub s (.port(i.mp));
  logic [7:0] q1, q3;
  always @(posedge clk) q1 <= i.rd.b;
  always @(posedge clk) i.mp.w[3:0] <= 4'h9;
  task automatic tk; i.mp.b.d = 7'h11; q3 = i.rd.b.d; endtask
  class C; task run; i.mp.w = 8'h77; endtask endclass
  initial begin
    C c = new;
    i.b = 0; i.w = 0;
    i.mp.b = 8'hA5;
    $display("A b=%h mpb=%h rdb=%h", i.b, i.mp.b, i.rd.b);
    i.mp.b.vld = 1'b0;
    $display("B b=%h vld=%b", i.b, i.rd.b.vld);
    #1 clk = 1; #1;
    $display("D q1=%h w=%h", q1, i.w);
    tk();
    $display("E b=%h q3=%h", i.b, q3);
    c.run();
    #1 $display("F w=%h", i.w);
    s.put(8'h42);
    #1 $display("G b=%h", i.b);
    $finish;
  end
endmodule
"#
        ),
        [
            r#"A b=a5 mpb=a5 rdb=a5"#,
            r#"B b=25 vld=0"#,
            r#"D q1=25 w=09"#,
            r#"E b=11 q3=11"#,
            r#"F w=77"#,
            r#"G b=42"#,
        ]
    );
}
