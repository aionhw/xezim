//! §6.24.1: in `casting_type'(expression)` a constant primary is a size
//! cast, and its value is the width of the assignment-like context the
//! operand is evaluated in. A field of a struct parameter is a constant
//! primary: `pt.DEPTH'(en << addr)` is `en << addr` evaluated at
//! `pt.DEPTH` bits (§11.6.1), so the shifted bit survives.
//!
//! The parser cannot tell `pt.DEPTH'` from a package-scoped type name, so it
//! lowers it to a named cast. Both the bytecode compiler and the interpreter
//! looked the target up by its leaf (`DEPTH`), found no type or parameter,
//! and passed the operand through at its self-determined width: a 1-bit
//! shift that read 0, or the wrong width when a same-named parameter was in
//! scope. The VeeR EL2 branch predictor builds its BHT write enables as
//! `pt.BHT_ARRAY_DEPTH'(bht_wr_en0[i] << bht_wr_addr0)`, so the BHT never
//! updated and every taken loop branch cost an extra cycle; the VeeR
//! EH2 instruction fetch sizes its bus tags and beat counts with
//! `(pt.IFU_BUS_TAG)'(...)`. Expected values follow the LRM; Verilator
//! agrees on every line.

fn lines(src: &str) -> Vec<String> {
    let sim = xezim::simulate(src, 100).expect("simulate");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("PASS") || m.starts_with("FAIL"))
        .collect()
}

const SRC: &str = r#"
package cfg_pkg;
  typedef struct packed {
    int unsigned DEPTH;
    int unsigned W;
    int unsigned HI;
    int unsigned LO;
  } cfg_t;
  typedef logic [11:0] word_t;
endpackage

module bp import cfg_pkg::*; #(parameter cfg_t pt = '{DEPTH: 256, W: 5, HI: 9, LO: 2}) (
    input  logic                 clk,
    input  logic [1:0]           en,
    input  logic [pt.HI:pt.LO]   addr,
    output logic [pt.DEPTH-1:0]  bank0,
    output logic [pt.DEPTH-1:0]  bank1,
    output logic [pt.DEPTH-1:0]  ibank
);
  // The VeeR branch-predictor shape: a one-hot write enable per bank.
  for (genvar i = 0; i < 2; i++) begin : BANKS
    wire [pt.DEPTH-1:0] wr;
    assign wr = pt.DEPTH'(en[i] << addr);
    if (i == 0) begin : B0
      assign bank0 = wr;
    end else begin : B1
      always_ff @(posedge clk) bank1 <= pt.DEPTH'(en[i] << addr);
    end
  end
  // A procedural cast in the instance's own scope.
  initial #1 ibank = pt.DEPTH'(en[0] << addr);
endmodule

module tb;
  import cfg_pkg::*;
  localparam cfg_t pt = '{DEPTH: 256, W: 5, HI: 9, LO: 2};
  localparam int DEPTH = 4;  // a same-named parameter must not size the cast
  logic clk = 0;
  logic [1:0] en = 2'b11;
  logic [9:2] addr = 8'd14;
  logic [3:0] a4 = 4'hf, b4 = 4'h1;
  logic [255:0] c_assign, c_comb, c_init, c_paren;
  logic [7:0] c_ctx, c_narrow, c_type;
  logic [11:0] c_typed;
  logic [255:0] bank0, bank1, ibank;
  assign c_assign = pt.DEPTH'(en[0] << addr);
  always_comb c_comb = pt.DEPTH'(en[0] << addr);
  assign c_paren = (pt.DEPTH)'(en[0] << addr);
  assign c_ctx = 8'(pt.W'(a4 + b4));
  assign c_narrow = 8'(pt.W'(8'hff));
  assign c_typed = word_t'(a4 + b4);
  bp #(.pt(pt)) u (.clk(clk), .en(en), .addr(addr), .bank0(bank0), .bank1(bank1), .ibank(ibank));

  task automatic chk(string name, logic [255:0] got, logic [255:0] exp);
    if (got === exp) $display("PASS %s", name);
    else $display("FAIL %s got=%h exp=%h", name, got, exp);
  endtask

  initial begin
    c_init = pt.DEPTH'(en[0] << addr);
    #1 clk = 1;
    #1 clk = 0;
    #1;
    chk("assign", c_assign, 256'(1) << 14);
    chk("always_comb", c_comb, 256'(1) << 14);
    chk("initial", c_init, 256'(1) << 14);
    chk("parenthesized", c_paren, 256'(1) << 14);
    chk("context", c_ctx, 8'h10);
    chk("narrowing", c_narrow, 8'h1f);
    chk("typedef cast", c_typed, 12'h010);
    chk("generate bank", bank0, 256'(1) << 14);
    chk("generate nba", bank1, 256'(1) << 14);
    chk("instance initial", ibank, 256'(1) << 14);
    $finish;
  end
endmodule
"#;

#[test]
fn size_cast_by_struct_parameter_field_sets_operand_context() {
    let got = lines(SRC);
    let expected: Vec<String> = [
        "assign",
        "always_comb",
        "initial",
        "parenthesized",
        "context",
        "narrowing",
        "typedef cast",
        "generate bank",
        "generate nba",
        "instance initial",
    ]
    .iter()
    .map(|n| format!("PASS {n}"))
    .collect();
    assert_eq!(got, expected);
}
