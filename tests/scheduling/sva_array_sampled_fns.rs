//! IEEE 1800-2023 §16.9.3: the operand of `$past`, `$stable` and `$changed`
//! may be a whole unpacked array, whose sampled value is all its cells. The
//! sampled-value watch read the array's name, which is no signal, so its
//! history never changed: `$stable(m)` always held, `$changed(m)` never
//! did, and `m == $past(m)` compared the array with that constant.
//!
//! `m` and `d` are written on the falling edge every other cycle, so at a
//! rising edge they changed since the last one exactly when `we` was set at
//! that one. `p` is written on the rising edge, so the edge that writes it
//! reads its sampled cells (§16.5.1). `$sampled(m)` is all of `m`'s cells
//! too, and so is the current value an off-tick `$stable(q, @(posedge clk))`
//! compares, for a `q` that never changes. Verilator 5.036 does not accept a
//! whole unpacked array as the operand of these functions.

use xezim::simulate;

const SRC: &str = r#"
module top;
  logic clk = 0;
  always #1 clk = ~clk;
  logic [3:0] m [2];
  logic [3:0] d [3:0];
  logic [3:0] p [2];
  logic [3:0] q [2];
  logic [3:0] din = 1;
  bit we = 0, wep = 0;
  int n = 0, f_chg = 0, f_stab = 0, f_past = 0, f_desc = 0, f_pos = 0, f_smp = 0, f_off = 0, f_wild = 0;
  // A cell that turns x at 12 and stays x: `$past(w) ==? w` treats the x
  // on the right as a wildcard, so it holds at every edge.
  logic [3:0] w [2];
  initial begin
    m[0] = 0; m[1] = 0;
    p[0] = 0; p[1] = 0;
    q[0] = 4'd3; q[1] = 4'd5;
    w[0] = 4'd1; w[1] = 4'd2;
    foreach (d[i]) d[i] = 0;
  end
  always @(posedge clk) begin
    if (wep) p[din[0]] <= din;
    wep <= !wep;
  end
  always @(negedge clk) if ($time > 4 && !$stable(q, @(posedge clk))) f_off = f_off + 1;
  initial #12 w[1] = 4'bx;
  always @(negedge clk) begin
    if (we) begin
      m[din[0]] <= din;
      d[din[1:0]] <= din;
    end
    din <= din + 1;
    we <= !we;
  end
  a_chg:  assert property (@(posedge clk) $time < 4 || $changed(m) == $past(we)) else f_chg = f_chg + 1;
  a_stab: assert property (@(posedge clk) $time < 4 || $stable(m) == !$past(we)) else f_stab = f_stab + 1;
  a_past: assert property (@(posedge clk) $time < 4 || (m == $past(m)) == !$past(we)) else f_past = f_past + 1;
  a_desc: assert property (@(posedge clk) $time < 4 || (d == $past(d)) == !$past(we)) else f_desc = f_desc + 1;
  a_pos:  assert property (@(posedge clk) $time < 4 || (p == $past(p)) == !$past(wep)) else f_pos = f_pos + 1;
  a_smp:  assert property (@(posedge clk) m == $sampled(m)) else f_smp = f_smp + 1;
  a_wild: assert property (@(posedge clk) $time < 4 || ($past(w) ==? w) === 1'b1) else f_wild = f_wild + 1;
  always @(posedge clk) n = n + 1;
  initial begin
    #20 $display("RESULT edges=%0d changed=%0d stable=%0d past=%0d desc=%0d pos=%0d sampled=%0d offtick=%0d wild=%0d",
                 n, f_chg, f_stab, f_past, f_desc, f_pos, f_smp, f_off, f_wild);
    $finish;
  end
endmodule
"#;

#[test]
fn sampled_value_functions_of_a_whole_unpacked_array() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    let line = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .find(|l| l.starts_with("RESULT"))
        .unwrap_or_default();
    assert_eq!(
        line,
        "RESULT edges=10 changed=0 stable=0 past=0 desc=0 pos=0 sampled=0 offtick=0 wild=0"
    );
}
