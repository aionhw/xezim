//! IEEE 1800-2017 §11.5.1, §7.4.1: a constant element index outside a packed
//! array's declared range reads x. The compiled read folded the index in u32,
//! so `arr[i-1]` at genvar 0 became element 4294967295, whose bit offset
//! overflowed into a bogus slice (CVA6 PMP `conf_addr_i[i-1]`).

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.trim().to_string()).collect()
}

#[test]
fn negative_and_past_end_constant_element_index_read_x() {
    let sim = simulate(
        r#"
module top;
  logic [7:0][53:0] conf; logic [53:0] neg, past, ok;
  for (genvar i = 0; i < 1; i++) begin : g
    assign neg  = conf[i-1];
    assign past = conf[i+8];
    assign ok   = conf[i+7];
  end
  initial begin
    for (int k = 0; k < 8; k++) conf[k] = 54'h10 + k;
    #1 $display("neg=%h past=%h ok=%h", neg, past, ok);
  end
endmodule
"#,
        10,
    )
    .expect("sim");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "neg=xxxxxxxxxxxxxx past=xxxxxxxxxxxxxx ok=00000000000017"),
        "{msgs:?}"
    );
}
