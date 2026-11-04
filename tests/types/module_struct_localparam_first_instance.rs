//! IEEE 1800-2017 §6.20.2, §7.2.1: a localparam of a struct type declared in
//! the module body takes its assignment-pattern value in every instance. The
//! first instance read 0 and later instances read the right value.

use xezim::simulate;

const SRC: &str = r#"
module leaf #(parameter logic [5:0] RV = '0) (output logic [5:0] q);
  assign q = RV;
endmodule
module core (output logic [5:0] q, output logic [5:0] r);
  typedef struct packed { logic mie; logic mpie; logic [1:0] mpp; logic mprv; logic tw; } st_t;
  localparam st_t RST = '{mie: 1'b0, mpie: 1'b1, mpp: 2'b00, mprv: 1'b0, tw: 1'b0};
  leaf #(.RV(RST)) u (.q(q));
  assign r = RST;
endmodule
module top;
  logic [5:0] qa, ra, qb, rb;
  core a (.q(qa), .r(ra));
  core b (.q(qb), .r(rb));
  initial #1 $display("%b %b %b %b", qa, ra, qb, rb);
endmodule
"#;

#[test]
fn module_struct_localparam_first_instance() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_eq!(msgs, ["010000 010000 010000 010000"]);
}
