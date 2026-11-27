//! IEEE 1800-2017 §6.5, §23.3.3: a variable connected to the output port of
//! an instance has that port as a continuous driver, so two such ports on
//! one variable are an error. §27.4, §27.5: an instance in a generate block
//! drives its actual once per loop iteration or taken branch. Two explicit
//! instances were rejected, but the same connection made by a generate loop,
//! or by a generate branch beside a module-level instance, was accepted and
//! simulated with one driver overwriting the other.

use xezim::simulate;

const DRV: &str = r#"
module drv (input logic i, output logic o);
  assign o = i;
endmodule
"#;

fn rejected(design: &str) -> String {
    let src = format!("{DRV}{design}");
    match simulate(&src, 100) {
        Ok(_) => panic!("accepted:\n{src}"),
        Err(e) => e,
    }
}

fn lines(design: &str) -> Vec<String> {
    let src = format!("{DRV}{design}");
    let sim = simulate(&src, 100).unwrap_or_else(|e| panic!("{e}\n{src}"));
    sim.output
        .iter()
        .map(|l| l.message.trim().to_string())
        .collect()
}

/// The issue's case: two iterations of a generate loop drive `x`.
#[test]
fn a_generate_loop_driving_one_variable_through_output_ports_is_rejected() {
    let e = rejected(
        r#"
module tb;
  logic [1:0] a = 2'b10;
  logic x;
  for (genvar g = 0; g < 2; g++) begin : g_d
    drv u (.i(a[g]), .o(x));
  end
endmodule
"#,
    );
    assert!(
        e.contains("Variable 'x' has multiple continuous drivers"),
        "{e}"
    );
}

/// A generate branch, a branch nested in a loop, and a case arm, each
/// beside a module-level instance on the same variable.
#[test]
fn generate_branches_beside_a_module_instance_are_rejected() {
    for body in [
        "if (1) begin : g drv u (.i(a), .o(x)); end",
        "for (genvar g = 0; g < 1; g++) begin : f if (1) begin : h drv u (.i(a), .o(x)); end end",
        "case (2) 1: begin : b end 2: begin : c drv u (.i(a), .o(x)); end endcase",
        "function automatic int pick(); return 1; endfunction
         if (pick()) begin : g drv u (.i(a), .o(x)); end",
        "function automatic int pick(); return 1; endfunction
         case (1) pick(): begin : g drv u (.i(a), .o(x)); end endcase",
    ] {
        let e = rejected(&format!(
            "module tb; logic a = 1'b1; logic x; drv t (.i(a), .o(x)); {body} endmodule\n"
        ));
        assert!(
            e.contains("Variable 'x' has multiple continuous drivers"),
            "{body}: {e}"
        );
    }
}

/// Guard: per-iteration bits, a variable declared in each iteration, a
/// branch that is not taken, a one-iteration loop, and a net (which takes
/// several drivers) are all accepted.
#[test]
fn distinct_drivers_are_accepted() {
    let l = lines(
        r#"
module tb;
  logic [1:0] a = 2'b10;
  logic [1:0] y;
  logic z, w;
  wire n;
  for (genvar g = 0; g < 2; g++) begin : g_d
    logic loc;
    drv u (.i(a[g]), .o(y[g]));
    drv v (.i(a[g]), .o(loc));
    drv m (.i(1'b1), .o(n));
  end
  if (0) begin : off drv u (.i(a[0]), .o(z)); end
  drv t (.i(a[0]), .o(z));
  for (genvar g = 0; g < 1; g++) begin : one drv u (.i(a[1]), .o(w)); end
  initial #1 $display("y=%b z=%b w=%b n=%b loc=%b", y, z, w, n, g_d[1].loc);
endmodule
"#,
    );
    assert!(l.contains(&"y=10 z=0 w=1 n=1 loc=1".to_string()), "{l:?}");
}

/// A constant-function case match must prevent the default arm from contributing a driver.
#[test]
fn generate_case_does_not_count_an_inactive_default_driver() {
    let output = lines(
        r#"
module tb;
  logic x;
  drv t (.i(1'b1), .o(x));
  function automatic int pick(); return 1; endfunction
  case (1)
    pick(): begin : selected end
    default: begin : skipped drv u (.i(1'b0), .o(x)); end
  endcase
  initial #1 $display("x=%b", x);
endmodule
"#,
    );
    assert!(output.contains(&"x=1".to_owned()), "{output:?}");
}
