//! IEEE 1800-2017 §7.4.6, §11.5.1: an unpacked-array element read through an
//! x index is x, including an element wider than 64 bits. The compiled read
//! built the x result with 64-bit inline storage but the element's width, so
//! a later part-select past bit 63 read wrapped bit patterns instead of x
//! (CVA6's 140-bit store-buffer entry indexed by an uninitialized pointer).

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.trim().to_string()).collect()
}

#[test]
fn wide_element_through_x_index_reads_x() {
    let sim = simulate(
        r#"
module top;
  typedef struct packed { logic [55:0] hi; logic [83:0] lo; } e_t;   // 140 bits
  e_t q [3:0];
  logic [1:0] ptr;
  logic [55:0] hi_sel; logic [83:0] lo_sel; logic valid;
  always_comb begin
    hi_sel = q[ptr].hi;
    lo_sel = q[ptr].lo;
    valid  = q[ptr][139];
  end
  initial begin
    q[0] = '{hi: 56'hABCDEF, lo: 84'h123}; q[1] = '0; q[2] = '0; q[3] = '0;
    #1 $display("x: hi=%h lo=%h v=%b", hi_sel, lo_sel, valid);
    ptr = 0;
    #1 $display("0: hi=%h lo=%h v=%b", hi_sel, lo_sel, valid);
  end
endmodule
"#,
        10,
    )
    .expect("sim");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "x: hi=xxxxxxxxxxxxxx lo=xxxxxxxxxxxxxxxxxxxxx v=x"),
        "{msgs:?}"
    );
    assert!(
        msgs.iter().any(|m| m == "0: hi=00000000abcdef lo=000000000000000000123 v=0"),
        "{msgs:?}"
    );
}
