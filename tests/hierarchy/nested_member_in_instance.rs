//! `s.arr_member[i].field` on a packed struct: inside an inlined instance the
//! member access folds into one hierarchical name, and the container may be
//! an element of a packed array of structs. Both the interpreter and the
//! bytecode compiler must resolve every shape, and the compiled blocks must
//! stay compiled (no interpreter fallback).
use std::path::PathBuf;
use std::process::Command;

const DESIGN: &str = r#"
typedef struct packed {
   logic [7:0] stamp;
   logic [2:0] region;
   logic       marker;
   logic [1:0] mode;
} rec_t;
typedef struct packed {
   logic [1:0][31:0] samples;
   rec_t [1:0] descriptors;
} bundle_t;

module child(input logic clk, input bundle_t src,
             output logic [2:0] oreg, output logic [7:0] ost,
             output logic [2:0] lreg, output logic [7:0] lst);
  bundle_t loc;
  assign loc = src;
  always_ff @(posedge clk) begin
    oreg <= src.descriptors[0].region;
    ost  <= src.descriptors[1].stamp;
    lreg <= loc.descriptors[0].region;
    lst  <= loc.descriptors[1].stamp;
  end
  always @(posedge clk) #1
    $display("%m I r=%0d st=%h e=%h s=%h", src.descriptors[0].region,
             src.descriptors[1].stamp, src.descriptors[0], src.samples[0]);
endmodule

module top;
  logic clk = 0;
  bundle_t b;
  bundle_t [1:0] grid;
  logic [2:0] rw, re, lw, le; logic [7:0] sw, se, tw, te;
  logic [2:0] gr; logic [7:0] gs;
  child cw(.clk(clk), .src(b),       .oreg(rw), .ost(sw), .lreg(lw), .lst(tw));
  child ce(.clk(clk), .src(grid[1]), .oreg(re), .ost(se), .lreg(le), .lst(te));
  always_ff @(posedge clk) begin
    gr <= grid[1].descriptors[0].region;
    gs <= grid[1].descriptors[1].stamp;
  end
  initial begin
    b = {32'h55667788, 32'h11223344, 8'h3c, 3'h2, 1'b0, 2'h1, 8'h96, 3'h5, 1'b1, 2'h2};
    grid[0] = '0;
    grid[1] = {32'hddeeff00, 32'h99aabbcc, 8'h58, 3'h1, 1'b1, 2'h0, 8'he2, 3'h7, 1'b0, 2'h3};
    #5 clk = 1; #2;
    $display("C rw=%0d sw=%h lw=%0d tw=%h re=%0d se=%h le=%0d te=%h gr=%0d gs=%h",
             rw, sw, lw, tw, re, se, le, te, gr, gs);
    fork $display("F gr=%0d gs=%h br=%0d", grid[1].descriptors[0].region,
                  grid[1].descriptors[1].stamp, b.descriptors[1].region); join
    $finish;
  end
endmodule
"#;

#[test]
fn nested_member_reads_resolve_and_stay_compiled() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("nested_member_in_instance");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("top.sv");
    std::fs::write(&src, DESIGN).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "top", src.to_str().unwrap(), "--no-cache"])
        .env("XEZIM_PROFILE_TIMING", "1")
        .output()
        .expect("run nested member design");
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "design failed:\n{text}");
    for want in [
        "top.cw I r=5 st=3c e=25ae s=11223344",
        "top.ce I r=7 st=58 e=38bb s=99aabbcc",
        "C rw=5 sw=3c lw=5 tw=3c re=7 se=58 le=7 te=58 gr=7 gs=58",
        "F gr=7 gs=58 br=2",
    ] {
        assert!(text.contains(want), "missing `{want}`:\n{text}");
    }
    assert!(
        text.lines().any(|line| line.contains("fallbacks=0")),
        "a nested member read fell back to the interpreter:\n{text}"
    );
}
