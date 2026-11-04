//! IEEE 1800-2017 §6.20.2, §7.2.1, §11.5.1: a part-select whose bounds are
//! members of a struct-typed parameter (`q[Cfg.LW/Cfg.AW-1:1]`) is a constant
//! select, so on a `[1:0][63:0]` array it picks ELEMENT 1 (64 bits). The
//! bytecode constant folders had no member-access arm, so the bound counted as
//! dynamic and the select read flat BIT 1: in CVA6 the AXI adapter's shift
//! register returned the fetched cache line shifted by 63 bits and the core
//! stalled after its first instruction fetch.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.trim().to_string()).collect()
}

#[test]
fn struct_param_member_bounds_select_whole_elements() {
    let sim = simulate(
        r#"
package cfgp;
  typedef struct packed { int unsigned LW; int unsigned AW; } cfg_t;
  localparam cfg_t C = '{LW: 128, AW: 64};
endpackage
module adp #(parameter cfgp::cfg_t Cfg = cfgp::C) ();
  localparam int N = Cfg.LW/Cfg.AW;
  logic [Cfg.LW/Cfg.AW-1:0][Cfg.AW-1:0] q;
  logic [127:0] r1, r2, r3, r4, r5, r6;
  logic [63:0] d = 64'hdddddddddddddddd;
  assign r6 = {d, q[Cfg.LW/Cfg.AW-1:1]};
  always_comb begin
    r1 = {d, q[1:1]};
    r2 = {d, q[N-1:1]};
    r3 = {d, q[Cfg.LW/Cfg.AW-1:1]};
    r4 = {d, q[(Cfg.LW/Cfg.AW-1):1]};
    r5 = {d, q[Cfg.LW/Cfg.AW-1]};
  end
  initial begin
    q = {64'h1111111111111111, 64'h2222222222222222};
    #1 $display("lit=%h N=%h cfg=%h par=%h idx=%h ca=%h", r1, r2, r3, r4, r5, r6);
  end
endmodule
module top;
  adp u1 ();
endmodule
"#,
        10,
    )
    .expect("sim");
    let msgs = messages(&sim);
    let want = "dddddddddddddddd1111111111111111";
    assert!(
        msgs.iter().any(|m| m
            == &format!("lit={want} N={want} cfg={want} par={want} idx={want} ca={want}")),
        "{msgs:?}"
    );
}

#[test]
fn struct_param_member_bounds_shift_register() {
    // The CVA6 adapter shape: a two-beat line assembled by shifting elements.
    let sim = simulate(
        r#"
package cfgp;
  typedef struct packed { int unsigned LW; int unsigned AW; } cfg_t;
  localparam cfg_t C = '{LW: 128, AW: 64};
endpackage
module adp #(parameter cfgp::cfg_t Cfg = cfgp::C) (input logic clk, input logic [Cfg.AW-1:0] d,
    input logic first, output logic [Cfg.LW-1:0] line);
  logic [Cfg.LW/Cfg.AW-1:0][Cfg.AW-1:0] sh_d, sh_q;
  always_comb begin
    sh_d = {d, sh_q[Cfg.LW/Cfg.AW-1:1]};
    if (first) sh_d[0] = d;
  end
  always_ff @(posedge clk) sh_q <= sh_d;
  assign line = sh_q;
endmodule
module top;
  logic clk = 0, first;
  logic [63:0] d;
  logic [127:0] line;
  adp u (.clk, .d, .first, .line);
  initial begin
    first = 1; d = 64'h0010031300000293; #1 clk = 1; #1 clk = 0;
    first = 0; d = 64'h006282b306500393; #1 clk = 1; #1 clk = 0;
    $display("line=%h", line);
  end
endmodule
"#,
        10,
    )
    .expect("sim");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "line=006282b3065003930010031300000293"),
        "{msgs:?}"
    );
}
