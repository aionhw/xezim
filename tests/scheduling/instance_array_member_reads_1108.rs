//! §9.2.2.2, §10.3.2, §23.6 (#1108): an element of an unpacked array that is
//! a member of an element of an array of instances (`lane[0].payload[2]`) is
//! an operand of the processes that read it. The read resolved to the array
//! of instances' own proxy signal, which no write changes, so an
//! `always_comb`, `always @*`, `always_latch` or a continuous assignment in a
//! nested module never ran again after time 0, and an event control naming
//! the element waited on nothing.

fn check(source: &str, expected: &[&str]) {
    let actual: Vec<_> = xezim::simulate(source, 1_000_000)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn instance_array_member_array_reads_wake_their_processes() {
    check(
        r#"
interface lane_if;
  logic [31:0] payload [6];
  logic [31:0] vec;
endinterface
interface big_if;
  logic [31:0] payload [64];
endinterface
interface kinds_if;
  real r [6];
  string s [6];
  bit [31:0] b [6];
  logic [31:0] up [1:6];
  logic [31:0] down [5:0];
endinterface
interface outer_if;
  lane_if lane [2] ();
endinterface
module hold_mod;
  logic [31:0] payload [6];
endmodule
module owner;
  lane_if lane [2] ();
  if (1) begin : g
    lane_if lane [2] ();
  end
endmodule
module nested_reader;
  lane_if lane [3] ();
  wire [31:0] w = lane[0].payload[2];
  logic [31:0] c;
  always_comb c = lane[0].payload[2];
  initial begin
    for (int j = 0; j < 6; j++) lane[0].payload[j] = 32'h10 + j;
    #7 $display("T|nested %h %h", w, c);
    for (int j = 0; j < 6; j++) lane[0].payload[j] = 32'hA0 + j;
    #7 $display("T|nested %h %h", w, c);
  end
endmodule
module top;
  lane_if lane [3] ();
  hold_mod hm [1] ();
  big_if big [2] ();
  kinds_if k [2] ();
  outer_if outer ();
  owner u ();
  nested_reader nr ();
  logic [31:0] c_comb, c_star, c_latch, c_list, c_mod, c_big, c_var, c_cond, c_u, c_ug, c_outer, c_bit, c_up, c_down;
  real c_real;
  string c_str;
  int sel = 2;
  logic take = 1;
  always_comb c_comb = lane[0].payload[2];
  always @* c_star = lane[0].payload[2];
  always_latch if (take) c_latch = lane[0].payload[2];
  always @(lane[0].payload[2]) c_list = lane[0].payload[2];
  always_comb c_mod = hm[0].payload[2];
  always_comb c_big = big[1].payload[40];
  always_comb c_var = lane[0].payload[sel];
  always_comb begin
    if (take) c_cond = lane[0].payload[2];
    else c_cond = 0;
  end
  always_comb c_u = u.lane[0].payload[2];
  always_comb c_ug = u.g.lane[1].payload[2];
  always_comb c_outer = outer.lane[0].payload[2];
  always_comb c_real = k[0].r[2];
  always_comb c_str = k[0].s[2];
  always_comb c_bit = k[0].b[2];
  always_comb c_up = k[0].up[2];
  always_comb c_down = k[0].down[2];
  task automatic fill(input logic [31:0] base);
    for (int j = 0; j < 6; j++) begin
      lane[0].payload[j] = base + j;
      hm[0].payload[j] = base + j;
      u.lane[0].payload[j] = base + j;
      u.g.lane[1].payload[j] = base + j;
      outer.lane[0].payload[j] = base + j;
      k[0].r[j] = base + j;
      k[0].s[j] = $sformatf("s%0h", base + j);
      k[0].b[j] = base + j;
      k[0].up[j + 1] = base + j;
      k[0].down[j] = base + j;
    end
    for (int j = 0; j < 64; j++) big[1].payload[j] = base + j;
  endtask
  task show(input string tag);
    $display("T|%s comb=%h star=%h latch=%h list=%h mod=%h big=%h var=%h cond=%h", tag,
             c_comb, c_star, c_latch, c_list, c_mod, c_big, c_var, c_cond);
    $display("T|%s u=%h ug=%h outer=%h real=%0.1f str=%s bit=%h up=%h down=%h", tag,
             c_u, c_ug, c_outer, c_real, c_str, c_bit, c_up, c_down);
  endtask
  initial begin
    fill(32'h10);
    #7 show("r0");
    fill(32'hA0);
    #7 show("r1");
    sel = 4;
    #1 $display("T|sel var=%h", c_var);
    $finish;
  end
endmodule
"#,
        &[
            "T|nested 00000012 00000012",
            "T|r0 comb=00000012 star=00000012 latch=00000012 list=00000012 mod=00000012 big=00000038 var=00000012 cond=00000012",
            "T|r0 u=00000012 ug=00000012 outer=00000012 real=18.0 str=s12 bit=00000012 up=00000011 down=00000012",
            "T|nested 000000a2 000000a2",
            "T|r1 comb=000000a2 star=000000a2 latch=000000a2 list=000000a2 mod=000000a2 big=000000c8 var=000000a2 cond=000000a2",
            "T|r1 u=000000a2 ug=000000a2 outer=000000a2 real=162.0 str=sa2 bit=000000a2 up=000000a1 down=000000a2",
            "T|sel var=000000a4",
        ],
    );
}
