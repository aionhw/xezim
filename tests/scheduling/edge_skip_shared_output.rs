//! Which clocked blocks may skip an idle edge.
//!
//! A block may skip a fire when none of its data inputs changed. That is only
//! sound while the block alone determines its outputs, so a block whose output
//! is also driven by another block must never skip. The interesting case is
//! the one in between: two generate arms writing `r[3:2]` and `r[1:0]` write
//! the same signal OBJECT but never the same BIT, and each still owns its own
//! bits. Those may skip; `q[2:1]` beside `q[1:0]`, which share bit 1, may not.
use std::path::PathBuf;
use std::process::Command;

fn run(name: &str, src: &str) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("edge_skip_shared_output");
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join(format!("{name}.sv"));
    std::fs::write(&sv, src).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .env("XEZIM_EVENT_EDGE_CENSUS", "1")
        // Merging folds neighbouring blocks into one, which hides which of
        // them could skip on its own; these tests are about the per-block
        // decision.
        .env("XEZIM_EDGE_MERGE", "0")
        // `--verbose`: the "[EVENT-EDGE] measure" line read below.
        .args([
            "--simulate",
            "--verbose",
            "-s",
            "tb",
            "--no-cache",
            sv.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "run failed:\n{text}");
    text
}

/// `N of M gateable` from the engine's census line.
fn gateable(text: &str) -> (u32, u32) {
    let line = text
        .lines()
        .find(|l| l.contains("measure (timestamp)"))
        .unwrap_or_else(|| panic!("no measure line:\n{text}"));
    let after = line.split("timestamp): ").nth(1).unwrap();
    let blocks: u32 = after.split(' ').next().unwrap().parse().unwrap();
    let gate: u32 = after
        .split("blocks, ")
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    (gate, blocks)
}

/// Blocks rejected because another block claims the same bits, and because
/// another block claims the same array, from the census line.
fn shared(text: &str) -> (u32, u32) {
    let line = text
        .lines()
        .find(|l| l.contains("[EVENT-EDGE-CENSUS]"))
        .unwrap_or_else(|| panic!("no census line:\n{text}"));
    let field = |key: &str| -> u32 {
        line.split(key)
            .nth(1)
            .unwrap()
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .unwrap()
            .parse()
            .unwrap()
    };
    (field("shared_bits="), field("shared_array="))
}

/// Two blocks writing the WHOLE signal: neither may skip, or the later write
/// is lost. Without the multi-writer check this printed `q=55`.
#[test]
fn two_whole_signal_writers_cannot_skip() {
    let text = run(
        "overlap",
        r#"
module tb;
  reg clk; reg en; reg [7:0] a, b; reg [7:0] q;
  initial begin
    clk = 0; en = 0; a = 8'hAA; b = 8'h55;
    #100 en = 1;
    #10  en = 0;
    #100 $display("q=%h", q);
    $finish;
  end
  always #5 clk = ~clk;
  always @(posedge clk) q <= a;
  always @(posedge clk) if (en) q <= b;
endmodule
"#,
    );
    assert!(text.contains("q=aa"), "a skipped write was lost:\n{text}");
    let (gate, _) = gateable(&text);
    assert_eq!(
        gate, 0,
        "a doubly driven output must not be skippable:\n{text}"
    );
}

/// Disjoint BIT slices of one register, one generate arm each: all skippable.
#[test]
fn disjoint_bit_slices_stay_skippable() {
    let text = run(
        "bits",
        r#"
module tb;
  reg clk; reg [7:0] d;
  reg [3:0] r;
  initial begin clk = 0; d = 0; #2000 $display("d=%0d r=%0d", d, r); $finish; end
  always #5 clk = ~clk;
  always @(posedge clk) d <= d + 8'd1;
  genvar i;
  generate for (i = 0; i < 4; i = i + 1) begin: H
      always @(posedge clk) r[i] <= d[i];
  end endgenerate
endmodule
"#,
    );
    let (gate, blocks) = gateable(&text);
    assert_eq!(
        gate, blocks,
        "disjoint slice writers lost their skip:\n{text}"
    );
}

/// Overlapping slices of one register: bit 1 has two drivers, so neither
/// block may skip. Relaxing this to "same signal" alone dropped 39 of 40
/// posedge writes.
#[test]
fn overlapping_bit_slices_cannot_skip() {
    let text = run(
        "overlap_bits",
        r#"
module tb;
  reg clk = 0;
  reg [3:0] q = 0;
  int errors = 0;
  always #2 clk = ~clk;
  always @(posedge clk) q[2:1] <= 2'b11;
  always @(negedge clk) q[1:0] <= 2'b00;
  initial begin
    repeat (40) begin
      @(posedge clk); #1; if (q[2:1] !== 2'b11) errors++;
      @(negedge clk); #1; if (q[1:0] !== 2'b00) errors++;
    end
    $display("errors=%0d", errors);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        text.contains("errors=0"),
        "a shared bit lost its driver:\n{text}"
    );
    let (gate, _) = gateable(&text);
    assert_eq!(gate, 0, "overlapping slice writers must not skip:\n{text}");
}

/// Disjoint ELEMENTS of one array, one generate arm each: all skippable. A
/// constant index names one element, so the arms are not four drivers of
/// the whole array.
#[test]
fn disjoint_array_elements_stay_skippable() {
    let text = run(
        "arr",
        r#"
module tb;
  reg clk; reg [7:0] d;
  reg [7:0] mem [0:3];
  initial begin clk = 0; d = 0; #2000 $display("d=%0d m0=%0d m3=%0d", d, mem[0], mem[3]); $finish; end
  always #5 clk = ~clk;
  always @(posedge clk) d <= d + 8'd1;
  genvar i;
  generate for (i = 0; i < 4; i = i + 1) begin: H
      always @(posedge clk) mem[i] <= d + i[7:0];
  end endgenerate
endmodule
"#,
    );
    assert!(
        text.contains("d=200 m0=199 m3=202"),
        "element writes went wrong:\n{text}"
    );
    let (gate, blocks) = gateable(&text);
    assert_eq!(
        gate, blocks,
        "disjoint element writers lost their skip:\n{text}"
    );
}

/// A constant-index writer with a STABLE input beside a run-time-index
/// writer of the same array. The dynamic writer claims every element, so
/// `mem[0] <= x` must keep firing: once `p` wraps to 0 the other block has
/// overwritten its element, and only a fresh fire restores it.
#[test]
fn dynamic_element_writer_blocks_constant_sibling() {
    let text = run(
        "arr_dyn",
        r#"
module tb;
  reg clk = 0; reg [1:0] p = 0; reg [7:0] x = 8'd5;
  reg [7:0] mem [0:3];
  int errors = 0;
  always #5 clk = ~clk;
  always @(posedge clk) p <= p + 2'd1;
  always @(posedge clk) mem[0] <= x;
  always @(posedge clk) mem[p] <= 8'd7;
  // After a posedge whose OLD p was not 0, only the constant writer touched
  // element 0. (When both wrote it, the order is not defined.)
  always @(negedge clk) if (p != 2'd1 && mem[0] !== 8'd5) errors++;
  initial begin
    #2000 $display("errors=%0d", errors);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        text.contains("errors=0"),
        "constant writer skipped under a dynamic sibling:\n{text}"
    );
    // The constant writer is rejected by the dynamic writer's claim on its
    // element, the dynamic writer by the constant writer's; the counter and
    // the checker may skip.
    assert_eq!(
        shared(&text),
        (1, 1),
        "both array writers must be rejected:\n{text}"
    );
    let (gate, _) = gateable(&text);
    assert_eq!(
        gate, 2,
        "only the counter and the checker may skip:\n{text}"
    );
}

/// The everyday form of that hazard: a reset loop clears the array through
/// a loop index while each element has its own writer with a stable input.
/// After reset, the element writers must re-drive their elements.
#[test]
fn reset_loop_beside_element_writers() {
    let text = run(
        "arr_reset",
        r#"
module tb;
  reg clk = 0; reg rst = 1; reg [7:0] a = 8'd11, b = 8'd22;
  reg [7:0] mem [0:3];
  integer k;
  always #5 clk = ~clk;
  always @(posedge clk) if (rst) for (k = 0; k < 4; k = k + 1) mem[k] <= 8'd0;
  always @(posedge clk) mem[1] <= a;
  always @(posedge clk) mem[2] <= b;
  initial begin
    repeat (3) @(posedge clk);
    #1 rst = 0;
    repeat (3) @(posedge clk);
    #1 $display("after reset m1=%0d m2=%0d", mem[1], mem[2]);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        text.contains("after reset m1=11 m2=22"),
        "element writers did not re-drive:\n{text}"
    );
}

/// Constant element writers on OPPOSITE edges of the same element, with
/// constant data (so neither has a data input at all): both must fire on
/// every edge.
#[test]
fn opposite_edge_element_writers_cannot_skip() {
    let text = run(
        "arr_edges",
        r#"
module tb;
  reg clk = 0;
  reg [7:0] mem [0:1];
  int errors = 0;
  always #5 clk = ~clk;
  always @(posedge clk) mem[1] <= 8'd1;
  always @(negedge clk) mem[1] <= 8'd0;
  initial begin
    repeat (40) begin
      @(posedge clk); #1; if (mem[1] !== 8'd1) errors++;
      @(negedge clk); #1; if (mem[1] !== 8'd0) errors++;
    end
    $display("errors=%0d", errors);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        text.contains("errors=0"),
        "an element writer skipped its edge:\n{text}"
    );
    let (gate, _) = gateable(&text);
    assert_eq!(gate, 0, "two writers of one element must not skip:\n{text}");
}

/// A DESCENDING declared range: the fold has to respect the declared bounds,
/// not assume `[0:N-1]`.
#[test]
fn descending_range_elements_stay_skippable() {
    let text = run(
        "arr_desc",
        r#"
module tb;
  reg clk = 0; reg [7:0] d = 0;
  reg [7:0] mem [3:0];
  always #5 clk = ~clk;
  always @(posedge clk) d <= d + 8'd1;
  always @(posedge clk) mem[3] <= d;
  always @(posedge clk) mem[0] <= d + 8'd3;
  initial begin
    #2000 $display("d=%0d m3=%0d m0=%0d", d, mem[3], mem[0]);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        text.contains("d=200 m3=199 m0=202"),
        "descending range elements:\n{text}"
    );
    let (gate, blocks) = gateable(&text);
    assert_eq!(
        gate, blocks,
        "descending range writers lost their skip:\n{text}"
    );
}

/// Elements of a TWO-dimensional array, one generate arm each.
#[test]
fn multi_dim_constant_elements_stay_skippable() {
    let text = run(
        "arr_2d",
        r#"
module tb;
  reg clk = 0; reg [7:0] d = 0;
  reg [7:0] grid [0:1][0:1];
  always #5 clk = ~clk;
  always @(posedge clk) d <= d + 8'd1;
  genvar i, j;
  generate for (i = 0; i < 2; i = i + 1) begin: R
    for (j = 0; j < 2; j = j + 1) begin: C
      always @(posedge clk) grid[i][j] <= d + i[7:0] * 8'd2 + j[7:0];
    end
  end endgenerate
  initial begin
    #2000 $display("d=%0d g00=%0d g11=%0d", d, grid[0][0], grid[1][1]);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        text.contains("d=200 g00=199 g11=202"),
        "2-D element writes went wrong:\n{text}"
    );
    let (gate, blocks) = gateable(&text);
    assert_eq!(gate, blocks, "2-D element writers lost their skip:\n{text}");
}

/// A blocking element write beside a non-blocking one: both claim one
/// element, and they are different elements.
#[test]
fn blocking_and_nonblocking_element_writers_stay_skippable() {
    let text = run(
        "arr_blk",
        r#"
module tb;
  reg clk = 0; reg [7:0] d = 0;
  reg [7:0] mem [0:1];
  always #5 clk = ~clk;
  always @(posedge clk) d <= d + 8'd1;
  always @(posedge clk) mem[0] = d;
  always @(posedge clk) mem[1] <= d + 8'd1;
  initial begin
    #2000 $display("d=%0d m0=%0d m1=%0d", d, mem[0], mem[1]);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        text.contains("d=200 m0=199 m1=200"),
        "element writes went wrong:\n{text}"
    );
    let (gate, blocks) = gateable(&text);
    assert_eq!(gate, blocks, "element writers lost their skip:\n{text}");
}

/// A constant index OUTSIDE the declared range writes nothing (§7.4.6) and
/// must neither disturb its sibling's element nor be folded to one.
#[test]
fn out_of_range_constant_index_is_harmless() {
    let text = run(
        "arr_oob",
        r#"
module tb;
  reg clk = 0; reg [7:0] d = 0;
  reg [7:0] mem [0:3];
  initial begin mem[0] = 8'd9; mem[3] = 8'd9; end
  always #5 clk = ~clk;
  always @(posedge clk) d <= d + 8'd1;
  always @(posedge clk) mem[0] <= d;
  always @(posedge clk) mem[5] <= d;
  initial begin
    #2000 $display("d=%0d m0=%0d m3=%0d", d, mem[0], mem[3]);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        text.contains("d=200 m0=199 m3=9"),
        "an out-of-range write leaked:\n{text}"
    );
}
