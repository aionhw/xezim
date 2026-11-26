//! §6.21, §13.3.1, §13.4.2: a variable of a static task or function keeps its
//! value from one call to the next, which a call can observe only when it
//! reads the variable before writing it whole. Constant selects that, one
//! after another and none of them reading it, write every bit of the
//! variable (`n[0] = a[31]; ... n[31] = a[0]; f = n;`, the body of a CRC or
//! bit-reversal function) write it whole, as `n = a;` does. Such a local was
//! judged to carry its value, so elaboration kept it as a static variable,
//! the function counted as impure, and every continuous assign calling it ran
//! on the AST interpreter at each evaluation (#1296).
//!
//! The guard is the settle profile's per-entry histogram: every continuous
//! assign of the first design is a compiled entry (`contassign_c`), none an
//! AST one (`contassign_ast`), and no call bails as impure. The second design
//! pins the values of locals the selects do not prove written whole, which
//! keep their value between calls; the expected values are those of the LRM,
//! which the reference simulator gives too.

fn run(src: &str, env: &[(&str, &str)]) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_static_select_cover_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, src).expect("write temporary design");
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.args([
        "--simulate",
        "-s",
        "top",
        path.to_str().unwrap(),
        "--no-cache",
    ])
    .env_remove("XEZIM_METHOD_CACHE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "design failed:\n{text}");
    text
}

/// `name=<n>` from the `[PROF] entry_hist` line; 0 when the kind is absent.
fn entry_hist(text: &str, kind: &str) -> u64 {
    let line = text
        .lines()
        .find(|l| l.starts_with("[PROF] entry_hist"))
        .unwrap_or_else(|| panic!("missing entry_hist:\n{text}"));
    line.split_whitespace()
        .find_map(|tok| tok.strip_prefix(&format!("{kind}=")))
        .map(|n| n.parse().expect("count"))
        .unwrap_or(0)
}

fn all_pass(text: &str, checks: usize) {
    let fails: Vec<&str> = text.lines().filter(|l| l.starts_with("FAIL")).collect();
    assert!(fails.is_empty(), "{fails:#?}\n{text}");
    assert_eq!(
        text.lines().filter(|l| l.starts_with("PASS")).count(),
        checks,
        "{text}"
    );
}

const COVERED: &str = r##"
module top;
  int errors = 0;
  task automatic check(string what, logic [127:0] got, logic [127:0] want);
    if (got === want) $display("PASS %s", what);
    else begin errors++; $display("FAIL %s: got %h, expected %h", what, got, want); end
  endtask
  function [31:0] reverse_bits(input [31:0] a);
    reg [31:0] n;
    begin
      n[0] = a[31]; n[1] = a[30]; n[2] = a[29]; n[3] = a[28]; n[4] = a[27]; n[5] = a[26]; n[6] = a[25]; n[7] = a[24];
      n[8] = a[23]; n[9] = a[22]; n[10] = a[21]; n[11] = a[20]; n[12] = a[19]; n[13] = a[18]; n[14] = a[17]; n[15] = a[16];
      n[16] = a[15]; n[17] = a[14]; n[18] = a[13]; n[19] = a[12]; n[20] = a[11]; n[21] = a[10]; n[22] = a[9]; n[23] = a[8];
      n[24] = a[7]; n[25] = a[6]; n[26] = a[5]; n[27] = a[4]; n[28] = a[3]; n[29] = a[2]; n[30] = a[1]; n[31] = a[0];
      reverse_bits = n;
    end
  endfunction
  // part-selects, indexed part-selects, overlapping
  function [7:0] swap(input [7:0] a);
    reg [7:0] n;
    begin n[7:4] = a[3:0]; n[0 +: 2] = a[5:4]; n[3 -: 2] = a[7:6]; n[1] = a[4]; swap = n; end
  endfunction
  // an int written by its two halves
  function [31:0] halves(input [15:0] a);
    int n;
    begin n[31:16] = a; n[15:0] = ~a; halves = n; end
  endfunction
  // a nested begin-end block on the straight path
  function [3:0] nested(input [3:0] a);
    reg [3:0] n;
    begin begin n[1:0] = a[3:2]; end n[3:2] = a[1:0]; nested = n; end
  endfunction
  reg [31:0] x;
  wire [31:0] w_rev = reverse_bits(x);
  wire [7:0] w_swap = swap(x[7:0]);
  wire [31:0] w_halves = halves(x[15:0]);
  wire [3:0] w_nested = nested(x[3:0]);
  initial begin
    x = 32'h12345678; #1;
    check("bit-selects, 12345678", w_rev, 32'h1e6a2c48);
    check("part-selects, 12345678", w_swap, 8'h87);
    check("int halves, 12345678", w_halves, 32'h5678a987);
    check("nested block, 12345678", w_nested, 4'h2);
    x = 32'hb4c3d2e1; #1;
    check("bit-selects, b4c3d2e1", w_rev, 32'h874bc32d);
    check("part-selects, b4c3d2e1", w_swap, 8'h1c);
    check("int halves, b4c3d2e1", w_halves, 32'hd2e12d1e);
    check("nested block, b4c3d2e1", w_nested, 4'h4);
    $display("DONE: %0d errors", errors);
    $finish;
  end
endmodule
"##;

const NOT_COVERED: &str = r##"
module top;
  int errors = 0;
  task automatic check(string what, logic [127:0] got, logic [127:0] want);
    if (got === want) $display("PASS %s", what);
    else begin errors++; $display("FAIL %s: got %h, expected %h", what, got, want); end
  endtask

  // every bit by a bit-select
  function [7:0] rev8(input [7:0] a);
    reg [7:0] n;
    begin
      n[0] = a[7]; n[1] = a[6]; n[2] = a[5]; n[3] = a[4];
      n[4] = a[3]; n[5] = a[2]; n[6] = a[1]; n[7] = a[0];
      rev8 = n;
    end
  endfunction
  // part-selects, indexed part-selects, overlapping
  function [7:0] swap(input [7:0] a);
    reg [7:0] n;
    begin n[7:4] = a[3:0]; n[0 +: 2] = a[5:4]; n[3 -: 2] = a[7:6]; n[1] = a[4]; swap = n; end
  endfunction
  // ascending and offset ranges
  function [7:0] asc(input [7:0] a);
    reg [0:7] n;
    begin n[0:3] = a[3:0]; n[4 +: 4] = a[7:4]; asc = n; end
  endfunction
  function [7:0] off(input [7:0] a);
    reg [11:4] n;
    begin n[11:8] = a[3:0]; n[7:4] = a[7:4]; off = n; end
  endfunction
  // an int written by two halves
  function [31:0] halves(input [15:0] a);
    int n;
    begin n[31:16] = a; n[15:0] = ~a; halves = n; end
  endfunction
  // a nested begin-end block on the straight path
  function [3:0] nested(input [3:0] a);
    reg [3:0] n;
    begin begin n[1:0] = a[3:2]; end n[3:2] = a[1:0]; nested = n; end
  endfunction
  // NOT whole: bit 3 is written only under an if, so it keeps the value of
  // the last call that wrote it (a static variable, 6.21)
  function [3:0] partial(input [3:0] a, input en);
    reg [3:0] n;
    begin n[0] = a[0]; n[1] = a[1]; n[2] = a[2]; if (en) n[3] = a[3]; partial = n; end
  endfunction
  // NOT whole: a select write that reads the variable
  function [3:0] reads_self(input [3:0] a);
    reg [3:0] n;
    begin n[0] = a[0]; n[1] = n[0]; n[3:2] = a[3:2]; reads_self = n; end
  endfunction
  // NOT whole: a read between the writes
  reg [3:0] seen;
  function [3:0] read_between(input [3:0] a);
    reg [3:0] n;
    begin n[1:0] = a[1:0]; seen = n; n[3:2] = a[3:2]; read_between = n; end
  endfunction
  // a static task whose output formal is written bit by bit
  task bits_out(input [3:0] a, output [3:0] o);
    begin o[0] = a[3]; o[1] = a[2]; o[2] = a[1]; o[3] = a[0]; end
  endtask

  reg [7:0] x;
  reg [3:0] y;
  reg en;
  wire [7:0] w_rev = rev8(x);
  wire [7:0] w_swap = swap(x);
  wire [7:0] w_asc = asc(x);
  wire [7:0] w_off = off(x);
  wire [31:0] w_halves = halves({x, x});
  wire [3:0] w_nested = nested(x[3:0]);
  reg [3:0] o;
  initial begin
    x = 8'hb4; #1;
    check("bit-selects", w_rev, 8'h2d);
    check("part-selects", w_swap, 8'h4b);
    check("ascending", w_asc, 8'h4b);
    check("offset", w_off, 8'h4b);
    check("int halves", w_halves, 32'hb4b4_4b4b);
    check("nested block", w_nested, 4'h1);
    x = 8'h3c; #1;
    check("bit-selects, second value", w_rev, 8'h3c);
    check("part-selects, second value", w_swap, 8'hc3);
    check("partial, writes bit 3", partial(4'b1010, 1'b1), 4'b1010);
    check("partial, keeps bit 3", partial(4'b0101, 1'b0), 4'b1101);
    check("reads itself", reads_self(4'b1101), 4'b1111);
    check("read between", read_between(4'b0110), 4'b0110);
    check("read between, seen", seen, 4'bxx10);
    bits_out(4'b0001, o);
    check("output formal bit by bit", o, 4'b1000);
    $display("DONE: %0d errors", errors);
    $finish;
  end
endmodule
"##;

/// 32 bit-selects; part-selects, indexed part-selects and an overlapping
/// bit; an `int` written by two halves; a nested block.
#[test]
fn constant_selects_writing_a_static_local_whole_compile() {
    let text = run(
        COVERED,
        &[
            ("XEZIM_PROFILE_TIMING", "1"),
            ("XEZIM_COMPILE_FAIL_STATS", "1"),
        ],
    );
    all_pass(&text, 8);
    assert_eq!(entry_hist(&text, "contassign_ast"), 0, "{text}");
    assert!(entry_hist(&text, "contassign_c") > 0, "{text}");
    assert!(!text.contains("Expr_Call_impure"), "{text}");
}

/// A bit written only under an `if` keeps the value of the last call that
/// wrote it; a select write that reads the variable; a read between the
/// writes; locals declared `[0:7]` and `[11:4]`, and an `output` formal of a
/// static task written bit by bit.
#[test]
fn static_locals_the_selects_do_not_write_whole_keep_their_values() {
    for env in [&[][..], &[("XEZIM_COMPILE_METHODS", "0")][..]] {
        all_pass(&run(NOT_COVERED, env), 14);
    }
}
