//! IEEE 1800-2023 §23.6 / §7.2: the named-block check reads a dotted name
//! segment by segment, but once a segment names a data object the rest is
//! a member select. `resp.exception.valid`, where `exception` is also the
//! label of a `begin : exception` block, was rejected as "'valid' is not
//! declared in block 'exception'" (#527, reported by davekeeshan).

use xezim::simulate;

const SRC: &str = r#"
package p;
  typedef struct packed { logic valid; logic [3:0] cause; } exc_t;
  typedef struct packed { logic ok; exc_t exception; } resp_t;
endpackage
module dut (input p::resp_t ara_resp_i, output logic exc_o);
  import p::*;
  resp_t resp_q;
  localparam resp_t RESET = '0;
  always_comb begin : exception
    exc_o = ara_resp_i.exception.valid | resp_q.exception.valid | RESET.exception.valid;
  end
  task show(resp_t r);
    $display("T|%b", r.exception.valid);
  endtask
endmodule
module top;
  p::resp_t r;
  logic e;
  dut u(.ara_resp_i(r), .exc_o(e));
  initial begin
    r = '0; r.exception.valid = 1'b1;
    #1 $display("E|%b", e);
    u.show(r);
  end
endmodule
"#;

#[test]
fn struct_member_named_like_a_block_is_accepted() {
    let sim = simulate(SRC, 10).expect("simulate");
    let msgs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(msgs.contains(&"E|1"), "{msgs:?}");
    assert!(msgs.contains(&"T|1"), "{msgs:?}");
}

#[test]
fn a_real_block_reference_is_still_checked() {
    let src = r#"
module top;
  int x;
  initial begin : blk
    int own;
    own = 1;
  end
  initial #1 $display("%0d", blk.nothere);
endmodule
"#;
    assert!(simulate(src, 10).is_err());
}
