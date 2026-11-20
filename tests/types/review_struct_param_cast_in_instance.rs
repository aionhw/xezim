//! §6.24.1: a size cast by a field of a struct parameter (`pt.W'(x)`)
//! evaluates its operand at the field's value in an instance's body too, with
//! that instance's own parameter values. An instance's names reach the
//! simulator with the instance folded into the first segment (`u.pt` .
//! `W`): the interpreter's lookup of the bare root missed it, and the
//! compiled lookup found a layout only through a same-named top-level
//! parameter, so with none in scope the operand passed through at its own
//! width. Every expected value matches the reference simulator.

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn struct_parameter_field_cast_in_instances_uses_each_instance_value() {
    let src = r#"
package cfg_pkg;
  typedef struct packed { int unsigned D; int unsigned W; } cfg_t;
endpackage
module sub #(parameter cfg_pkg::cfg_t pt = '{D: 9, W: 3}) ();
  logic [15:0] o1, o2, o3, o4, o5;
  logic en = 1;
  initial #1 o1 = pt.W'(4'h7 + 4'h1);
  initial begin #1 o2 = pt.W'(4'h7 + 4'h1); end
  initial begin #1 o3 = (pt.D)'(en << 9); end
  assign o4 = pt.W'(4'h7 + 4'h1);
  always_comb o5 = pt.W'(4'h7 + 4'h1);
  initial begin #1 $display("T|%m disp=%h", pt.W'(4'h7 + 4'h1)); end
  initial #2 $display("T|%m o1=%h o2=%h o3=%h o4=%h o5=%h", o1, o2, o3, o4, o5);
endmodule
module t;
  sub #(.pt('{D: 11, W: 3})) u();
  sub v();
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "T|t.u disp=0",
            "T|t.v disp=0",
            "T|t.u o1=0000 o2=0000 o3=0200 o4=0000 o5=0000",
            "T|t.v o1=0000 o2=0000 o3=0000 o4=0000 o5=0000",
        ]
    );
}

#[test]
fn nested_struct_parameter_field_and_value_parameter_casts_in_instances() {
    let src = r#"
package cfg_pkg;
  typedef struct packed { int unsigned D; int unsigned W; } cfg_t;
  typedef struct packed { cfg_t in; int unsigned K; } outer_t;
endpackage
module sub #(parameter cfg_pkg::outer_t po = '{in: '{D: 9, W: 3}, K: 5}, parameter int N = 3) ();
  logic [15:0] o1, o2, o3, o4, o5;
  initial begin
    #1 o1 = po.in.W'(4'h7 + 4'h1);
    o2 = po.K'(6'h3f + 6'h1);
    o3 = N'(4'h7 + 4'h1);
  end
  assign o4 = po.in.W'(4'h7 + 4'h1);
  assign o5 = N'(4'h7 + 4'h1);
  initial #2 $display("T|%m o1=%h o2=%h o3=%h o4=%h o5=%h", o1, o2, o3, o4, o5);
endmodule
module t;
  localparam int N = 8;
  sub #(.po('{in: '{D: 11, W: 4}, K: 7}), .N(4)) u();
  sub v();
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "T|t.u o1=0008 o2=0040 o3=0008 o4=0008 o5=0008",
            "T|t.v o1=0000 o2=0000 o3=0000 o4=0000 o5=0000",
        ]
    );
}
