//! IEEE 1800-2017 §26.3: a name declared by two packages resolves in a
//! module through THAT module's imports. The shared parameter table keeps
//! one bare entry per name (last package registered wins), and compiled
//! blocks read bare constants from it: CVA6's `alu` imports `ariane_pkg`,
//! but `fu_data_i.operation == ADD` compared against `fpnew_pkg::ADD` (2) and
//! the ADD arm never fired, so every ALU result was 0.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.trim().to_string()).collect()
}

#[test]
fn each_module_reads_the_enum_constant_it_imports() {
    let sim = simulate(
        r#"
package a_pkg;
  typedef enum logic [7:0] { ADD, SUB, MUL } fu_op;
endpackage
package f_pkg;
  typedef enum logic [1:0] { FMADD, FNMSUB, ADD, MUL } operation_e;
endpackage
module alu import a_pkg::*; (input fu_op op, input logic [7:0] a, b, output logic [7:0] r, k);
  always_comb begin
    r = '0;
    unique case (op)
      ADD: r = a + b;
      SUB: r = a - b;
      default: ;
    endcase
  end
  assign k = ADD;
endmodule
module fpu import f_pkg::*; (input operation_e op, output logic is_add, output logic [7:0] fk, fk2);
  always_comb is_add = (op == ADD);
  assign fk = ADD;
  always_comb fk2 = ADD + 8'd0;
endmodule
module top;
  a_pkg::fu_op op = a_pkg::ADD; logic [7:0] a = 3, b = 4, r, k, fk, fk2; logic f_is_add;
  f_pkg::operation_e fop = f_pkg::ADD;
  alu u (.op, .a, .b, .r, .k);
  fpu f (.op(fop), .is_add(f_is_add), .fk, .fk2);
  initial #1 $display("r=%0d k=%0d f_is_add=%b fk=%0d fk2=%0d", r, k, f_is_add, fk, fk2);
endmodule
"#,
        10,
    )
    .expect("sim");
    let msgs = messages(&sim);
    assert!(msgs.iter().any(|m| m == "r=7 k=0 f_is_add=1 fk=2 fk2=2"), "{msgs:?}");
}

#[test]
fn qualified_enum_member_in_constant_context() {
    // §26.3, §6.20: `pkg::MEMBER` names that package's member in every
    // context, including a declaration initializer and a localparam.
    let sim = simulate(
        r#"
package a_pkg;
  typedef enum logic [7:0] { ADD, SUB, MUL } fu_op;
endpackage
package f_pkg;
  typedef enum logic [1:0] { FMADD, FNMSUB, ADD, MUL } operation_e;
endpackage
module top;
  f_pkg::operation_e fop = f_pkg::ADD;
  a_pkg::fu_op aop = a_pkg::ADD;
  f_pkg::operation_e fop2;
  localparam f_pkg::operation_e FP = f_pkg::ADD;
  initial begin
    fop2 = f_pkg::ADD;
    #1 $display("f=%0d a=%0d fop=%0d aop=%0d fop2=%0d FP=%0d", f_pkg::ADD, a_pkg::ADD, fop, aop, fop2, FP);
  end
endmodule
"#,
        10,
    )
    .expect("sim");
    let msgs = messages(&sim);
    assert!(msgs.iter().any(|m| m == "f=2 a=0 fop=2 aop=0 fop2=2 FP=2"), "{msgs:?}");
}
