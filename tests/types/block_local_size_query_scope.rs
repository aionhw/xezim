//! Block-local `logic [$clog2($size(arr))-1:0]` must size from the CURRENT
//! instance's unpacked array, not a same-named top-level array nor 1 bit.
use xezim::simulate;

fn line(src: &str, want: &str) {
    let sim = simulate(src, 500).expect("simulate failed");
    let msgs: Vec<_> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        msgs.iter().any(|m| m == want),
        "missing {want:?} in {msgs:?}"
    );
}

const CHILD_BODY: &str = r#"
  logic [6:0] mem [0:6];
  logic [7:0] comb_idx;
  logic [7:0] comb_w;
  logic clk;
  logic [7:0] ff_idx;
  logic [7:0] ff_w;
  always @(*) begin
    logic [$clog2($size(mem))-1:0] v;
    comb_w = $bits(v);
    v = 6;
    comb_idx = v;
  end
  always_ff @(posedge clk) begin
    logic [$clog2($size(mem))-1:0] v;
    ff_w <= $bits(v);
    v <= 6;
    ff_idx <= v;
  end
  initial clk = 0;
  always #1 clk = ~clk;
  initial #5 begin
    $display("comb=%0d bits_comb=%0d ff=%0d bits_ff=%0d",
             comb_idx, comb_w, ff_idx, ff_w);
    $finish;
  end
"#;

#[test]
fn child_only_no_parent_array_name() {
    line(
        &format!(
            "module child (); {CHILD_BODY} endmodule
module tb; child u (); endmodule"
        ),
        "comb=6 bits_comb=3 ff=6 bits_ff=3",
    );
}

#[test]
fn child_wins_over_parent_mem3() {
    line(
        &format!(
            "module child (); {CHILD_BODY} endmodule
module tb;
  logic [2:0] mem [0:2];
  child u ();
endmodule"
        ),
        "comb=6 bits_comb=3 ff=6 bits_ff=3",
    );
}

#[test]
fn child_wins_over_parent_mem16() {
    line(
        &format!(
            "module child (); {CHILD_BODY} endmodule
module tb;
  logic [15:0] mem [0:15];
  child u ();
endmodule"
        ),
        "comb=6 bits_comb=3 ff=6 bits_ff=3",
    );
}

#[test]
fn top_level_block_local_uses_own_mem() {
    line(
        r#"module tb;
  logic [6:0] mem [0:6];
  logic [7:0] idx;
  logic [7:0] w;
  always @(*) begin
    logic [$clog2($size(mem))-1:0] v;
    w = $bits(v);
    v = 6;
    idx = v;
  end
  initial #1 begin
    $display("top idx=%0d bits=%0d", idx, w);
    $finish;
  end
endmodule"#,
        "top idx=6 bits=3",
    );
}
