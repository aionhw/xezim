//! §7.2, §7.4.2: a member write through constant element selects of a packed
//! struct (`st.e[1].dw[0].value <= d`) is one static bit range of the
//! container, which elaboration lists in its flattened layout. The compiler
//! sent every such NBA and blocking write to the AST interpreter
//! (`nba_member_access`, `blocking_target_member_access`); a key-vault-shaped
//! design with 384 of them in generate loops ran about 100x slower than its
//! flat-array twin. Expected values are Verilator's (5.046).

use std::path::PathBuf;
use std::process::Command;

#[test]
fn constant_element_member_writes_compile_and_match() {
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/perf/packed_member_path_store.sv");
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "packed_member_path_store_check",
            source.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_FALLBACK_SITES", "1")
        .output()
        .expect("run packed member path workload");

    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(
        output.status.success(),
        "packed member path workload failed:\n{text}"
    );
    assert!(
        text.contains("st=23222120 c=2120 a=9a0a1a2 top_st=77556688 rd=22 s2_o=99000000 s2=11")
            && text.contains("PACKED_MEMBER_PATH_OK"),
        "packed member path values did not match:\n{text}"
    );
    assert!(
        !text.contains("member_access"),
        "a member write through element selects fell back to the interpreter:\n{text}"
    );
}

/// §11.6.1: a sized index expression keeps its width (`1'b1 + 1'b1` is 0,
/// so the NBA writes `e[0]`), and a function's own struct shadows the
/// module signal of the same name. Both stay on the interpreter's path.
#[test]
fn index_width_and_a_shadowing_local() {
    let got: Vec<String> = xezim::simulate(
        r#"
// width of an index expression, and a function-local container that shadows a module signal
package gp;
  typedef struct packed { logic [7:0] value; } dw_t;
  typedef struct packed { dw_t [1:0] dw; } nest_t;
  typedef struct packed { nest_t [2:0] e; } nest3_t;
endpackage
module gsub import gp::*; (input logic clk, output nest3_t st, output logic [7:0] fr);
  nest3_t s;
  function automatic logic [7:0] rd_local();
    nest3_t s;
    s = '0;
    s.e[1].dw[0].value = 8'h77;
    return s.e[1].dw[0].value;
  endfunction
  always_ff @(posedge clk) begin
    st <= '0;
    st.e[1'b1 + 1'b1].dw[0].value <= 8'h5A;
  end
  initial begin s = '0; s.e[1].dw[0].value = 8'h11; end
  always_comb fr = rd_local();
endmodule
module gtop;
  logic clk = 0; gp::nest3_t st; logic [7:0] fr;
  gsub u (.clk(clk), .st(st), .fr(fr));
  initial begin #1 clk = 1; #1 $display("g1: st=%h fr=%h", st, fr); end
endmodule
"#,
        100,
    )
    .expect("simulate failed")
    .output
    .iter()
    .map(|o| o.message.clone())
    .collect();
    let want = "g1: st=00000000005a fr=77";
    assert!(
        got.iter().any(|l| l == want),
        "missing `{want}` in {got:#?}"
    );
}

/// A member wider than 32 bits keeps every bit in a compiled blocking write:
/// the right-hand side is sized by the member, not by a 32-bit default.
#[test]
fn wide_member_through_element_selects() {
    let got: Vec<String> = xezim::simulate(
        r#"
// a 64-bit member written through constant element selects
module w64;
  typedef struct packed { logic [63:0] value; } dw_t;
  typedef struct packed { dw_t [1:0] dw; } nest_t;
  typedef struct packed { nest_t [1:0] e; } nest2_t;
  nest2_t s, t;
  logic clk = 0;
  always_comb begin
    s = '0;
    s.e[1].dw[0].value = 64'h1122334455667788;
  end
  always_ff @(posedge clk) t.e[0].dw[1].value <= 64'h99aabbccddeeff00;
  initial begin t = '0; #1 clk = 1; #1 $display("w64: s=%h t=%h", s.e[1].dw[0].value, t.e[0].dw[1].value); end
endmodule
"#,
        100,
    )
    .expect("simulate failed")
    .output
    .iter()
    .map(|o| o.message.clone())
    .collect();
    let want = "w64: s=1122334455667788 t=99aabbccddeeff00";
    assert!(
        got.iter().any(|l| l == want),
        "missing `{want}` in {got:#?}"
    );
}

/// §10.6.2, §6.11.2: a forced container keeps its forced value under a
/// compiled member write, and a two-state container stores x/z as 0.
#[test]
fn forced_and_two_state_containers() {
    let got: Vec<String> = xezim::simulate(
        r#"
// a forced container ignores compiled member writes; a two-state container
// stores x/z as 0
module fz;
  typedef struct packed { logic [7:0] value; } dw_t;
  typedef struct packed { dw_t [1:0] dw; } nest_t;
  typedef struct packed { nest_t [1:0] e; } nest2_t;
  typedef struct packed { bit [7:0] value; } bdw_t;
  typedef struct packed { bdw_t [1:0] dw; } bnest_t;
  typedef struct packed { bnest_t [1:0] e; } bnest2_t;
  nest2_t s;
  bnest2_t b;
  logic [7:0] d = 8'h11;
  logic [7:0] dx = 8'h0f;
  always_comb s.e[1].dw[0].value = d;
  always_comb b.e[1].dw[0].value = dx;
  initial begin
    force s = 32'haaaa_aaaa;
    #1 d = 8'h55;
    dx = 8'bxxxx_0101;
    #1 $display("fz: s=%h b=%h", s, b);
  end
endmodule
"#,
        100,
    )
    .expect("simulate failed")
    .output
    .iter()
    .map(|o| o.message.clone())
    .collect();
    let want = "fz: s=aaaaaaaa b=00050000";
    assert!(
        got.iter().any(|l| l == want),
        "missing `{want}` in {got:#?}"
    );
    // the same 2-state container inside an instance
    let got: Vec<String> = xezim::simulate(
        r#"
module leaf;
  typedef struct packed { bit [7:0] value; } bdw_t;
  typedef struct packed { bdw_t [1:0] dw; } bnest_t;
  typedef struct packed { bnest_t [1:0] e; } bnest2_t;
  bnest2_t b;
  logic [7:0] dx = 8'h0f;
  always_comb b.e[1].dw[0].value = dx;
  initial begin
    #1 dx = 8'bxxxx_0101;
    #1 $display("fz2: b=%h", b);
  end
endmodule
module top; leaf u(); endmodule
"#,
        100,
    )
    .expect("simulate failed")
    .output
    .iter()
    .map(|o| o.message.clone())
    .collect();
    let want = "fz2: b=00050000";
    assert!(
        got.iter().any(|l| l == want),
        "missing `{want}` in {got:#?}"
    );
}

/// §23.9: a frame local hides a module parameter of the same name, so
/// `s.e[I]` with a function's own `int I` is a dynamic element select, not
/// `parameter I`'s element. A block's static `int I` likewise.
#[test]
fn frame_local_index_hides_a_parameter() {
    let got: Vec<String> = xezim::simulate(
        r#"
// a frame local `I` hides `parameter I` in an element select
module psh;
  typedef struct packed { logic [7:0] value; } dw_t;
  typedef struct packed { dw_t [1:0] dw; } nest_t;
  typedef struct packed { nest_t [2:0] e; } nest3_t;
  parameter int I = 0;
  nest3_t st, rs;
  logic [1:0] sel = 2'd2;
  logic [7:0] rd, cm, fn;
  logic clk = 0;
  function automatic logic [7:0] f(input int k);
    int I;
    I = k;
    return rs.e[I].dw[1].value;
  endfunction
  always_ff @(posedge clk) begin
    int I;
    I = sel;
    st.e[I].dw[0].value <= 8'h5A;
    rd <= rs.e[I].dw[1].value;
  end
  always_comb begin
    int I;
    I = sel;
    cm = rs.e[I].dw[1].value;
  end
  always_comb fn = f(sel);
  initial begin
    st = '0;
    rs = '0;
    rs.e[2].dw[1].value = 8'h3C;
    rs.e[0].dw[1].value = 8'hC3;
    #1 clk = 1;
    #1 $display("psh: st=%h rd=%h cm=%h fn=%h", st, rd, cm, fn);
  end
endmodule
"#,
        100,
    )
    .expect("simulate failed")
    .output
    .iter()
    .map(|o| o.message.clone())
    .collect();
    let want = "psh: st=005a00000000 rd=3c cm=3c fn=3c";
    assert!(
        got.iter().any(|l| l == want),
        "missing `{want}` in {got:#?}"
    );
}
