//! Packed lvalue paths that mix SEVERAL dynamic steps — two indices, an
//! index plus a member, an index plus a part-select. Each store arm resolves
//! one dynamic component, and until these compiled, the combination bailed to
//! the AST interpreter; because a per-statement fallback is forbidden inside a
//! register-variable loop, one such statement demoted its whole loop, which is
//! why a DRAM model spent most of its run interpreting (~0.5 ms per masked
//! S-box write, ~30 µs per SRAM lane write).
//!
//! Every expected value here is the reference simulator's. The tests also
//! assert the constructs COMPILE: a `[PROF] fallback_reason` line naming one
//! of these reasons means the statement went back to the interpreter.
use std::path::PathBuf;
use std::process::Command;

fn run(name: &str, src: &str) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("packed_path_dynamic_stores");
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join(format!("{name}.sv"));
    std::fs::write(&sv, src).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "--profile",
            "-s",
            "t",
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

/// The store reasons this file is about. `$display`/`$finish` fall back by
/// design and are ignored.
const STORE_BAILS: [&str; 5] = [
    "blocking_target",
    "nba_index_other",
    "nba_range_unresolved",
    "nba_member_access",
    "nba_sel_base_maps",
];

fn assert_compiled(text: &str) {
    for line in text.lines().filter(|l| l.contains("fallback_reason")) {
        for bail in STORE_BAILS {
            assert!(
                !line.contains(bail),
                "statement fell back to the interpreter: {}",
                line.trim()
            );
        }
    }
}

/// Compiles a masked-S-box style `o[b][m] = i[m][b]` (both indices dynamic).
#[test]
fn two_level_blocking_write_into_a_packed_2d() {
    let text = run(
        "two_level_blocking_write_into_a_packed_2d",
        r#"
// Item 1 variant: 2-level blocking write, PACKED-2D base.
module t;
  logic clk = 0; always #5 clk = ~clk;
  logic [7:0][7:0] sbox_or_trees;
  logic [7:0][7:0] sbox_mid_masked;
  always @(posedge clk)
    for (int b = 0; b < 8; b++)
      for (int m = 0; m < 8; m++)
        sbox_or_trees[b][m] = sbox_mid_masked[m][b];
  initial begin
    for (int i = 0; i < 8; i++) sbox_mid_masked[i] = 8'(i * 8'h13 + 1);
    repeat (3) @(posedge clk);
    #1 $display("SBOX2 %h", sbox_or_trees);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        text.contains("SBOX2 80706c4a1896cc55"),
        "wrong value:\n{text}"
    );
    assert_compiled(&text);
}

/// Compiles an SRAM byte-lane write `mem[ab][(i*W) +: W] <= …` (element and lane both dynamic).
#[test]
fn part_select_of_a_dynamic_element_nba() {
    let text = run(
        "part_select_of_a_dynamic_element_nba",
        r#"
// Item 2 variant: NBA part-select of a dyn-index PACKED-2D element.
module t;
  logic clk = 0; always #5 clk = ~clk;
  localparam WP = 8, NW = 4;
  logic [15:0][31:0] mem;
  logic [31:0] db = 32'hdead_beef;
  logic [3:0]  ab = 4'd3;
  always @(posedge clk)
    for (int i = 0; i < NW; i++)
      mem[ab][(i*WP) +: WP] <= db[(i*WP) +: WP];
  initial begin
    mem = '0;
    repeat (3) @(posedge clk);
    #1 $display("SRAM2 mem3=%h", mem[3]);
    $finish;
  end
endmodule
"#,
    );
    assert!(text.contains("SRAM2 mem3=deadbeef"), "wrong value:\n{text}");
    assert_compiled(&text);
}

/// Compiles `s.arr[i].field <= …`, a member of a dynamically selected element of a member array.
#[test]
fn struct_member_of_a_dynamic_element_nba() {
    let text = run(
        "struct_member_of_a_dynamic_element_nba",
        r#"
// Item 3: NBA into struct-member-in-array-in-struct.
module t;
  logic clk = 0; always #5 clk = ~clk;
  typedef struct packed { logic [7:0] f; logic [7:0] g; } lane_t;
  typedef struct packed { lane_t [3:0] lane_dec; logic [7:0] x; } rd_t;
  rd_t hc;
  logic [7:0] v = 8'h5a;
  always @(posedge clk)
    for (int i = 0; i < 4; i++)
      hc.lane_dec[i].f <= 8'(v + i);
  initial begin
    hc = '0;
    repeat (3) @(posedge clk);
    #1 $display("MEMB %h", hc);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        text.contains("MEMB 5d005c005b005a0000"),
        "wrong value:\n{text}"
    );
    assert_compiled(&text);
}

/// Compiles `q[i][ptr[i]] <= …`, two dynamic indices into a packed array of structs.
#[test]
fn two_level_dynamic_index_nba() {
    let text = run(
        "two_level_dynamic_index_nba",
        r#"
// Item 4: NBA to a 2-level dynamic index of a packed-2D-of-structs.
module t;
  logic clk = 0; always #5 clk = ~clk;
  typedef struct packed { logic [7:0] a; } e_t;
  e_t [3:0][7:0] bqueue;
  logic [2:0] wptr [0:3];
  always @(posedge clk)
    for (int i = 0; i < 4; i++) begin
      bqueue[i][wptr[i]] <= e_t'(8'(i * 8'h11 + 1));
      wptr[i] <= wptr[i] + 1;
    end
  initial begin
    bqueue = '0; foreach (wptr[i]) wptr[i] = 3'(i);
    repeat (3) @(posedge clk);
    #1 $display("QUEUE %h", bqueue);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        text.contains("QUEUE 0000343434000000000000232323000000000000121212000000000000010101"),
        "wrong value:\n{text}"
    );
    assert_compiled(&text);
}

/// Compiles `v[i][2:0] <= …` on a non-zero-based outer dimension.
#[test]
fn range_select_on_a_dynamic_element_nba() {
    let text = run(
        "range_select_on_a_dynamic_element_nba",
        r#"
// Item 5 variant: range SELECT on a label-mapped (non-zero-based) packed-2D base.
module t;
  logic clk = 0; always #5 clk = ~clk;
  localparam CNLANES = 4;
  logic [CNLANES:1][CNLANES:0] next_pattr_same_req;
  logic [CNLANES:0] row = 5'h15;
  logic [2:0] got;
  always @(posedge clk)
    for (int i = 1; i <= CNLANES; i++) begin
      next_pattr_same_req[i][2:0] <= 3'(row ^ 5'(i));
      got <= next_pattr_same_req[i][2:0];
    end
  initial begin
    next_pattr_same_req = '0;
    repeat (3) @(posedge clk);
    #1 $display("LABEL2 %h got=%h", next_pattr_same_req, got);
    $finish;
  end
endmodule
"#,
    );
    assert!(text.contains("LABEL2 098e4 got=1"), "wrong value:\n{text}");
    assert_compiled(&text);
}

/// §7.4.6 / §11.5.1 edge cases on the same paths: an unknown or out-of-range
/// index modifies nothing, a non-zero-based dimension addresses the slot its
/// labels name, and `-:` runs downward from its base.
#[test]
fn unknown_and_out_of_range_indices_modify_nothing() {
    let text = run(
        "edge",
        r#"
// Edge cases for the composed packed-path stores: unknown and out-of-range
// indices must modify nothing (IEEE 1800 7.4.6 / 11.5.1), non-zero-based and
// descending dimensions must address the same bits as the interpreter, and
// `-:` part-selects must run downward.
module t;
  logic clk = 0; always #5 clk = ~clk;
  typedef struct packed { logic [7:0] f; logic [7:0] g; } lane_t;
  typedef struct packed { lane_t [3:0] lane_dec; logic [7:0] x; } rd_t;

  logic [7:0][7:0] q;          // packed 2-D, zero-based
  logic [8:1][7:0] nz;         // packed 2-D, non-zero-based outer
  rd_t hc;
  logic [15:0][31:0] mem;
  logic [3:0] ix, bad;
  logic [2:0] uk;
  int step = 0;

  always @(posedge clk) begin
    step <= step + 1;
    case (step)
      0: begin ix = 2; bad = 4'd9; uk = 3'bx1x; end
      1: begin
           q[ix][3] <= 1'b1;                 // in range
           nz[4][2] <= 1'b1;                 // constant, non-zero-based
           hc.lane_dec[ix].g <= 8'h77;
           mem[ix][(3*8) -: 8] <= 8'h5a;     // indexed-down part-select
         end
      2: begin
           q[uk][3] <= 1'b1;                 // x index: no write
           hc.lane_dec[uk].g <= 8'hff;       // x index: no write
           mem[uk][(1*8) +: 8] <= 8'hff;     // x index: no write
         end
      3: begin
           q[bad][3] <= 1'b1;                // out of range: no write
           mem[ix][(9*8) +: 8] <= 8'hff;     // range past the element
         end
      4: begin
           nz[ix+2][1] <= 1'b1;              // dynamic, non-zero-based
         end
    endcase
  end
  initial begin
    q = '0; nz = '0; hc = '0; mem = '0;
    repeat (8) @(posedge clk);
    #1 $display("EDGE q=%h nz=%h hc=%h mem2=%h mem3=%h", q, nz, hc, mem[2], mem[3]);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        text.contains("EDGE q=0000000000080000 nz=0000000006000000 hc=000000770000000000 mem2=00b40000 mem3=00000000"),
        "wrong value:\n{text}"
    );
    assert_compiled(&text);
}

/// §7.4.6, §11.5.1: each index or slice stays within its own packed dimension.
#[test]
fn packed_path_stores_respect_each_dimension() {
    let text = run(
        "packed_path_stores_respect_each_dimension",
        r#"
module t;
  logic [2:0] k5 = 3'd5, k6 = 3'd6;
  logic [3:0][5:0][7:0] a;
  logic [1:0][2:0][7:0] b;
  always_comb begin
    a = '0;
    a[1][k6] = 8'hee;
  end
  always_comb begin
    b = '0;
    b[0][1][k5 + 3'd1 +: 4] = 4'hf;
  end
  initial begin
    #1;
    $display("BOUNDS %h %h", a, b);
    $finish;
  end
endmodule
"#,
    );
    assert!(
        text.contains("BOUNDS 000000000000000000000000000000000000000000000000 00000000c000"),
        "{text}"
    );
    assert_compiled(&text);
}
