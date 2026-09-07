//! Three shapes that fell back to the AST interpreter for no good reason.
//!
//! 1. A NESTED packed member (`s.p.hi`, and every `union`-in-struct form).
//!    Elaboration flattens the layout into one dotted key ("p.hi") stored
//!    under the ROOT signal, but the resolver only ever split the LAST path
//!    segment — so it looked for a container named `s.p`, missed, and sent
//!    every member at depth >= 2 to the AST path.
//! 2. An assignment pattern whose target is an ARRAY ELEMENT
//!    (`arr[i] <= '{...}`). The pattern compiler needs the destination's
//!    field layout installed; the installer only accepted a bare `Ident`, so
//!    an indexed target got no layout and the pattern bailed.
//! 3. A function whose body is `return '{...}` — the ordinary way to build a
//!    struct result. The purity walker had no arm for assignment patterns, so
//!    it fell through to "impure" and the call was never inlined.
//!
//! Each test asserts the reference simulator's values AND that the specific
//! bail no longer fires; without the second half the first only re-tests the
//! AST path, which was already correct.

use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

fn run(src: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_pm_nest_{tag}"));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("dut.sv");
    std::fs::write(&path, src).expect("write");
    let out = Command::new(xezim())
        .current_dir(&dir)
        .arg("--simulate")
        .arg("--max-time")
        .arg("300")
        .arg("-s")
        .arg("tb")
        .arg(&path)
        .env("XEZIM_PROFILE_REPORT", "1")
        .output()
        .expect("run xezim");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn a_nested_packed_member_write_compiles() {
    const SRC: &str = r#"
typedef struct packed { logic [15:0] hi; logic [15:0] lo; } inner_t;
typedef struct packed { logic v; inner_t p; logic [7:0] tag; } outer_t;
module tb;
  logic clk = 0; outer_t s; int unsigned cyc = 0; logic [15:0] o;
  always #5 clk = ~clk;
  always @(posedge clk) begin
    cyc    <= cyc + 1;
    s.v    <= 1'b1;        // depth 1
    s.tag  <= cyc[7:0];    // depth 1
    s.p.hi <= cyc[15:0];   // depth 2
    s.p.lo <= ~cyc[15:0];  // depth 2
  end
  initial begin
    repeat (6) @(posedge clk); #1; o = s.p.hi;
    $display("HI=%04x LO=%04x TAG=%02x V=%b", o, s.p.lo, s.tag, s.v);
    $finish;
  end
endmodule
"#;
    let t = run(SRC, "nest");
    // Reference simulator: HI=0005 LO=fffa TAG=05 V=1
    assert!(
        t.contains("HI=0005 LO=fffa TAG=05 V=1"),
        "nested member values do not match the reference:\n{t}"
    );
    assert!(
        !t.contains("nba_ident_unresolved"),
        "nested member write still falls back:\n{t}"
    );
}

#[test]
fn a_union_member_and_its_nested_struct_member_compile() {
    const SRC: &str = r#"
typedef struct packed { logic [15:0] hi; logic [15:0] lo; } h_t;
typedef union packed { logic [31:0] word; h_t h; } u_t;
module tb;
  logic clk = 0; u_t a, b, c; int unsigned cyc = 0;
  always #5 clk = ~clk;
  always @(posedge clk) begin
    cyc    <= cyc + 1;
    a.word <= cyc * 32'd7;
    b.h.hi <= cyc[15:0];            // union -> struct -> member
    c      <= cyc ^ 32'hFFFF_0000;  // whole union
  end
  initial begin
    repeat (6) @(posedge clk); #1;
    $display("A=%08x B=%08x C=%08x", a.word, b.word, c.word);
    $finish;
  end
endmodule
"#;
    let t = run(SRC, "union");
    // Reference simulator: A=00000023 B=0005xxxx C=ffff0005
    // B's low half is x: nothing ever writes b.h.lo.
    assert!(
        t.contains("A=00000023 B=0005xxxx C=ffff0005"),
        "union member values do not match the reference:\n{t}"
    );
    assert!(
        !t.contains("nba_ident_unresolved"),
        "union member write still falls back:\n{t}"
    );
}

#[test]
fn an_assignment_pattern_into_an_array_element_compiles() {
    const SRC: &str = r#"
typedef struct packed { logic v; logic [14:0] d; } e_t;
module tb;
  logic clk = 0; e_t arr[8]; int unsigned cyc = 0; integer i;
  always #5 clk = ~clk;
  always @(posedge clk) begin
    cyc <= cyc + 1;
    for (i = 0; i < 8; i++)
      arr[i] <= '{v: cyc[0], d: cyc[14:0] + i[14:0]};
  end
  initial begin
    repeat (20) @(posedge clk); #1;
    $display("D=%04x %04x", arr[0].d, arr[7].d);
    $finish;
  end
endmodule
"#;
    let t = run(SRC, "pattern");
    // Reference simulator: D=0013 001a
    assert!(
        t.contains("D=0013 001a"),
        "array-element pattern values do not match the reference:\n{t}"
    );
    assert!(
        !t.contains("Expr_AssignmentPattern"),
        "array-element assignment pattern still falls back:\n{t}"
    );
}

#[test]
fn a_function_returning_an_assignment_pattern_is_inlined() {
    const SRC: &str = r#"
typedef struct packed { logic [15:0] a; logic [15:0] b; } p_t;
module tb;
  logic clk = 0; p_t r; int unsigned cyc = 0;
  function automatic p_t mk(input int unsigned c);
    return '{a: c[15:0], b: ~c[15:0]};
  endfunction
  always #5 clk = ~clk;
  always @(posedge clk) begin cyc <= cyc + 1; r <= mk(cyc); end
  initial begin
    repeat (6) @(posedge clk); #1;
    $display("A=%04x B=%04x", r.a, r.b);
    $finish;
  end
endmodule
"#;
    let t = run(SRC, "fnpat");
    // Reference simulator: A=0005 B=fffa
    assert!(
        t.contains("A=0005 B=fffa"),
        "struct-returning function values do not match the reference:\n{t}"
    );
    assert!(
        !t.contains("Expr_Call_impure"),
        "a `return '{{...}}` function is still treated as impure:\n{t}"
    );
}
