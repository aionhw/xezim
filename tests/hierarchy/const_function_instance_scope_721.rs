//! IEEE 1800-2023 §13.4.3 (#721): a constant function called from an
//! instance sizes its formals, locals and return value against the caller's
//! parameters and the module parameter table, read in place. Each call used
//! to copy the whole table first. Expected lines are the reference
//! simulator's.

use xezim::simulate;

#[test]
fn constant_functions_size_by_each_instance_parameters() {
    let sim = simulate(
        r#"
package p;
  parameter int BASE = 3;
  function automatic int unsigned clog2u(input int unsigned v);
    int unsigned r;
    r = 0;
    while ((32'd1 << r) < v) r++;
    return r;
  endfunction
  function automatic logic [BASE:0] widen(input logic [BASE:0] v);
    return v + 1'b1;
  endfunction
endpackage
module leaf #(parameter int D = 4, parameter int W = 8) ();
  import p::*;
  typedef struct packed { logic [W-1:0] hi; logic [3:0] lo; } pair_t;
  function automatic pair_t mk(input logic [W-1:0] v);
    pair_t r;
    r.hi = v + 1'b1;
    r.lo = 4'd5;
    return r;
  endfunction
  function automatic logic [W-1:0] top_bit(input logic [W-1:0] v);
    logic [W-1:0] t;
    t = '0;
    t[W-1] = v[0];
    return t;
  endfunction
  localparam int AW = clog2u(D);
  localparam logic [BASE:0] WN = widen(4'hf);
  localparam logic [BASE:0] WM = widen(4'h6);
  localparam pair_t PR = mk('1);
  localparam logic [W-1:0] TB = top_bit(8'd1);
  logic [AW:0] cnt;
  initial $display("L|%m D=%0d W=%0d AW=%0d WN=%0d WM=%0d PR=%h bits=%0d TB=%h cnt=%0d",
                   D, W, AW, WN, WM, PR, $bits(PR), TB, $bits(cnt));
endmodule
module top;
  leaf #(.D(4), .W(8)) a ();
  leaf #(.D(33), .W(12)) b ();
  for (genvar i = 1; i < 4; i++) begin : g
    leaf #(.D(i * 100), .W(4 + i)) u ();
  end
  initial #1 $finish;
endmodule
"#,
        10,
    )
    .expect("simulate");
    let mut lines: Vec<&str> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|l| l.starts_with("L|"))
        .collect();
    lines.sort_unstable();
    assert_eq!(
        lines,
        [
            "L|top.a D=4 W=8 AW=2 WN=0 WM=7 PR=005 bits=12 TB=80 cnt=3",
            "L|top.b D=33 W=12 AW=6 WN=0 WM=7 PR=0005 bits=16 TB=800 cnt=7",
            "L|top.g[1].u D=100 W=5 AW=7 WN=0 WM=7 PR=005 bits=9 TB=10 cnt=8",
            "L|top.g[2].u D=200 W=6 AW=8 WN=0 WM=7 PR=005 bits=10 TB=20 cnt=9",
            "L|top.g[3].u D=300 W=7 AW=9 WN=0 WM=7 PR=005 bits=11 TB=40 cnt=10",
        ]
    );
}
