//! §31.7: the `&&&` conditions of library-cell timing checks are compiled
//! when first evaluated and never walked by the interpreter again: a gated
//! clock that fans out to many cells dispatches every check of every cell on
//! each edge, and each dispatch used to resolve the condition's names by
//! string (#1246, 9x the instructions of the whole run). The guard is the
//! profile's condition count; the violations are the reference simulator's.

fn run_profiled(src: &str) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_timing_conds_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, src).expect("write temporary design");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "tb",
            path.to_str().unwrap(),
            "--no-cache",
            "--profile",
        ])
        .output()
        .expect("run profiled design");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "profiled design failed:\n{text}");
    text
}

#[test]
fn gated_clock_conditions_compile() {
    let text = run_profiled(
        r#"

`timescale 1ns/1ps

`ifndef NBANKS
  `define NBANKS 2
`endif
`ifndef NSTAGES
  `define NSTAGES 3
`endif
`ifndef NFRAMES
  `define NFRAMES 10
`endif
`ifndef CAPTURE_CYCLES
  `define CAPTURE_CYCLES 4
`endif

module cg_cell (output wire eck, input wire ck, input wire e);
  reg e_q;
  always @(ck or e)
    if (!ck) e_q = e;
  assign eck = ck & e_q;
endmodule

module sync_cell (output wire q, input wire ck, input wire da,
                  input wire se, input wire si);

  reg  q_r;
  reg  NOTIFIER;

  wire en_d_nse_nsi, en_d_nse_si, en_d_nsi, en_d_se;
  wire en_d_se_nsi, en_d_se_si, en_nd_nse_nsi, en_nd_nse_si;
  wire en_nd_se, en_nd_se_nsi, en_nd_se_si, en_nd_si;
  wire en_nse_nsi, en_nse_si;

  assign en_d_nse_nsi  = (da & ~se & ~si) ? 1'b1 : 1'b0;
  assign en_d_nse_si   = (da & ~se &  si) ? 1'b1 : 1'b0;
  assign en_d_nsi      = (da &          ~si) ? 1'b1 : 1'b0;
  assign en_d_se       = (da & se         ) ? 1'b1 : 1'b0;
  assign en_d_se_nsi   = (da & se & ~si) ? 1'b1 : 1'b0;
  assign en_d_se_si    = (da & se &  si) ? 1'b1 : 1'b0;
  assign en_nd_nse_nsi = (~da & ~se & ~si) ? 1'b1 : 1'b0;
  assign en_nd_nse_si  = (~da & ~se &  si) ? 1'b1 : 1'b0;
  assign en_nd_se      = (~da & se         ) ? 1'b1 : 1'b0;
  assign en_nd_se_nsi  = (~da & se & ~si) ? 1'b1 : 1'b0;
  assign en_nd_se_si   = (~da & se &  si) ? 1'b1 : 1'b0;
  assign en_nd_si      = (~da &          si) ? 1'b1 : 1'b0;
  assign en_nse_nsi    = (     ~se & ~si) ? 1'b1 : 1'b0;
  assign en_nse_si     = (     ~se &  si) ? 1'b1 : 1'b0;

  always @(posedge ck)
    q_r <= se ? si : da;
  assign q = q_r;

  specify
    $width(negedge ck &&& en_nd_nse_nsi == 1'b1, 0.02, 0.02, NOTIFIER);
    $width(negedge ck &&& en_d_nse_nsi  == 1'b1, 0.02, 0.02, NOTIFIER);
    $width(negedge ck &&& en_nd_nse_si  == 1'b1, 0.02, 0.02, NOTIFIER);
    $width(negedge ck &&& en_d_nse_si   == 1'b1, 0.02, 0.02, NOTIFIER);
    $width(negedge ck &&& en_nd_se_nsi  == 1'b1, 0.02, 0.02, NOTIFIER);
    $width(negedge ck &&& en_d_se_nsi   == 1'b1, 0.02, 0.02, NOTIFIER);
    $width(negedge ck &&& en_nd_se_si   == 1'b1, 0.02, 0.02, NOTIFIER);
    $width(negedge ck &&& en_d_se_si    == 1'b1, 0.02, 0.02, NOTIFIER);
    $width(posedge ck &&& en_nd_nse_nsi == 1'b1, 0.02, 0.02, NOTIFIER);
    $width(posedge ck &&& en_d_nse_nsi  == 1'b1, 0.02, 0.02, NOTIFIER);
    $width(posedge ck &&& en_nd_nse_si  == 1'b1, 0.02, 0.02, NOTIFIER);
    $width(posedge ck &&& en_d_nse_si   == 1'b1, 0.02, 0.02, NOTIFIER);
    $width(posedge ck &&& en_nd_se_nsi  == 1'b1, 0.02, 0.02, NOTIFIER);
    $width(posedge ck &&& en_d_se_nsi   == 1'b1, 0.02, 0.02, NOTIFIER);
    $width(posedge ck &&& en_nd_se_si   == 1'b1, 0.02, 0.02, NOTIFIER);
    $width(posedge ck &&& en_d_se_si    == 1'b1, 0.02, 0.02, NOTIFIER);
    $setuphold(posedge ck &&& (en_nd_si     == 1'b1), negedge se, 0.02, 0.02, NOTIFIER);
    $setuphold(posedge ck &&& (en_d_nsi     == 1'b1), negedge se, 0.02, 0.02, NOTIFIER);
    $setuphold(posedge ck &&& (en_nd_se     == 1'b1), negedge si, 0.02, 0.02, NOTIFIER);
    $setuphold(posedge ck &&& (en_d_se      == 1'b1), negedge si, 0.02, 0.02, NOTIFIER);
    $setuphold(posedge ck &&& (en_nd_se     == 1'b1), posedge si, 0.02, 0.02, NOTIFIER);
    $setuphold(posedge ck &&& (en_d_se      == 1'b1), posedge si, 0.02, 0.02, NOTIFIER);
    $setuphold(posedge ck &&& (en_nse_nsi   == 1'b1), posedge da,  0.02, 0.02, NOTIFIER);
    $setuphold(posedge ck &&& (en_nse_si    == 1'b1), posedge da,  0.02, 0.02, NOTIFIER);
    $setuphold(posedge ck &&& (en_nd_si     == 1'b1), posedge se, 0.02, 0.02, NOTIFIER);
    $setuphold(posedge ck &&& (en_d_nsi     == 1'b1), posedge se, 0.02, 0.02, NOTIFIER);
    $setuphold(posedge ck &&& (en_nse_nsi   == 1'b1), negedge da,  0.02, 0.02, NOTIFIER);
    $setuphold(posedge ck &&& (en_nse_si    == 1'b1), negedge da,  0.02, 0.02, NOTIFIER);
  endspecify

endmodule

module sync_bank #(parameter integer NSTAGES = 24) (
  output wire [NSTAGES-1:0] q,
  input  wire               ck,
  input  wire               ce,
  input  wire               se,
  input  wire [NSTAGES-1:0] d,
  input  wire [NSTAGES-1:0] si
);
  wire gck;
  cg_cell u_gate (.eck(gck), .ck(ck), .e(ce));

  genvar i;
  generate
    for (i = 0; i < NSTAGES; i = i + 1) begin : g_stage
      wire dbit;
      wire sibit;
      assign dbit  = d[i];
      assign sibit = si[i];
      sync_cell u_cell (.q(q[i]), .ck(gck), .da(dbit),
                        .se(se), .si(sibit));
    end
  endgenerate
endmodule

module tb;

  localparam integer NB = `NBANKS;
  localparam integer NS = `NSTAGES;
  localparam integer NF = `NFRAMES;
  localparam integer CC = `CAPTURE_CYCLES;

  localparam integer F_VIOL_10 = (NF * 1) / 10;
  localparam integer F_VIOL_50 = (NF * 5) / 10;
  localparam integer F_VIOL_90 = (NF * 9) / 10;

  logic        clk = 1'b0;
  logic [NB-1:0]    ce_v;
  logic [NB-1:0]    se_v;
  logic [NB*NS-1:0] din;
  logic [NB*NS-1:0] si_v;
  wire  [NB*NS-1:0] q;

  integer       errors;
  logic [31:0]  lfsr;

  always #5 clk = ~clk;                    // 100 MHz

  genvar b;
  generate
    for (b = 0; b < NB; b = b + 1) begin : g_bank
      sync_bank #(.NSTAGES(NS)) u_bank (
        .q (q  [b*NS +: NS]),
        .ck(clk),
        .ce(ce_v[b]),
        .se(se_v[b]),
        .d (din[b*NS +: NS]),
        .si(si_v[b*NS +: NS])
      );
    end
  endgenerate

  task automatic frame_step(input integer f);
    logic [NB*NS-1:0] expd;
    logic [NB*NS-1:0] sivec;
    logic [NB-1:0]    sebit;
    begin
      lfsr = (lfsr << 13) ^ (lfsr >> 17) ^ (lfsr << 5);
      for (int k = 0; k < NB; k = k + 1) begin
        expd [k*NS +: NS] = lfsr ^ (32'h9E3779B9 * (k + 1));
        sivec[k*NS +: NS] = ~lfsr ^ (32'h6C078965 * (k + 1));
        sebit[k]          = ((lfsr >> ((k * 5 + 3) % 16)) >> 4) & 1'b1;
      end
      @(negedge clk);                      // safe zone: clock low
      din  = expd;                         // new word settles mid-low-phase
      si_v = sivec;
      se_v = sebit;
      ce_v = '1;                           // open the bank gates
      repeat (CC) @(posedge clk);          // capture window
      @(negedge clk);
      ce_v = '0;                           // close the bank gates
      for (int k = 0; k < NB; k = k + 1)
        if (se_v[k]) expd[k*NS +: NS] = sivec[k*NS +: NS];
      if (q !== expd) begin
        errors = errors + 1;
        $display("  capture mismatch at frame %0d: q=%h expected=%h",
                 f, q, expd);
      end
    end
  endtask

  task automatic violation_frame(input integer f, input integer pct);
    logic [NB*NS-1:0] expd;
    begin
      @(negedge clk);                      // safe zone: clock low
      din  = '0;                           // bank 0 data low, scan pins low
      si_v = '0;
      se_v = '0;
      ce_v = '1;                           // open the bank gates
      #(5.000 - 0.010);                    // 10 ps before the posedge
      din[0] = 1'b1;                       // -> setup violation (< 20 ps)
      #0.020;                              // 10 ps after the posedge
      din[1] = 1'b1;                       // -> hold violation  (< 20 ps)
      repeat (CC - 1) @(posedge clk);      // finish the capture window
      @(negedge clk);
      ce_v = '0;                           // close the bank gates
      expd = '0;
      expd[1:0] = 2'b11;                   // both skewed bits captured
      if (q !== expd) begin
        errors = errors + 1;
        $display("  capture mismatch in violation frame %0d: q=%h expected=%h",
                 f, q, expd);
      end
      $display("  [viol] set injected at frame %0d of %0d (%0d%% into the simulation)",
               f, NF, pct);
    end
  endtask

  initial begin
    errors = 0;
    lfsr   = 32'h12345678;
    ce_v   = '0;
    se_v   = '0;
    din    = '0;
    si_v   = '0;
    repeat (4) @(negedge clk);
    for (int f = 0; f < NF; f = f + 1) begin
      if (f == F_VIOL_10)
        violation_frame(f, 10);            // set 1: 10% into the simulation
      else if (f == F_VIOL_50)
        violation_frame(f, 50);            // set 2: 50% into the simulation
      else if (f == F_VIOL_90)
        violation_frame(f, 90);            // set 3: 90% into the simulation
      else
        frame_step(f);
    end
    repeat (4) @(negedge clk);
    if (errors == 0)
      $display("TEST_PASS: %0d frames through %0d bank(s) of %0d stage(s), all captures matched",
               NF, NB, NS);
    else
      $display("TEST_FAIL count=%0d", errors);
    $finish;
  end

endmodule
"#,
    );
    let violations: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with("** Error:"))
        .map(|l| l.split(" violation in ").next().unwrap())
        .collect();
    let want: Vec<String> = [105, 305, 505]
        .iter()
        .flat_map(|t| {
            [
                format!(
                    "** Error: $setup( posedge da:{}990 ps, posedge ck &&& (en_nse_nsi == 1'b1):{} ns, 20 ps )",
                    t - 1,
                    t
                ),
                format!(
                    "** Error: $hold( posedge ck &&& (en_nse_nsi == 1'b1):{} ns, posedge da:{}010 ps, 20 ps )",
                    t, t
                ),
            ]
        })
        .collect();
    assert_eq!(violations, want, "{text}");
    assert!(text.contains("TEST_PASS: 10 frames"), "{text}");
    let counts = text
        .lines()
        .find_map(|l| l.strip_prefix("[PROF] timing_conds "))
        .unwrap_or_else(|| panic!("missing condition counts:\n{text}"));
    let compiled: u64 = counts
        .split_whitespace()
        .find_map(|t| t.strip_prefix("compiled="))
        .and_then(|n| n.parse().ok())
        .unwrap();
    assert!(compiled > 1000, "{counts}");
    assert!(counts.ends_with("interpreted=0"), "{counts}");
}
