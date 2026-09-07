//! §7.4.2: a procedural LOCAL that is a packed array of packed structs —
//! `sp_t [1:0] a;` and the inline `struct packed {…} [1:0] b;` — inside a
//! submodule's task, written and read member-wise (`a[0].p = v; x = a[0].p`).
//!
//! Two defects met here:
//! 1. `packed_inner_elem_width` had no arm for an inline `Struct` carrying
//!    outer packed dims, so the element width fell to ONE BIT — a whole-
//!    element write stored a single bit — in any scope. The typedef'd form
//!    only worked in the top module because it resolved through the typedef
//!    element tables; in a submodule the local-typedef substitution folds it
//!    into exactly this inline shape.
//! 2. The interpreter's `arr[i].field` walks (write AND read) resolved the
//!    root with `resolve_hier_name`, which under a submodule scope hint gives
//!    `u.a` — while a procedural local registers its layout and storage under
//!    the BARE `a`. No layout → the element read back x. They now fall back
//!    to the bare name and use the frame-aware accessors, so a task/function
//!    local works like a process local.
//!
//! Sibling shapes pinned alongside: two outer dims on an inline struct (needs
//! a `Struct` arm in `packed_full_dims_of` for the two-index slot), a nested
//! typedef'd member inside the struct (must be substituted too, or it sizes
//! to the 32-bit default at run time and shifts every member above it), a
//! FUNCTION local, and an NBA member write on a module-level packed array.
//!
//! Every expected value is the reference simulator's.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn packed_struct_array_locals_in_a_submodule_task() {
    let o = out(r#"
`define V 96'h112233445566778899aabbcc
module sub ();
  typedef struct packed { logic [95:0] p; logic [7:0] t; } s_t;
  logic [127:0] r_td, r_il, r_tv, r_il1; logic [95:0] r_rd;
  task automatic t();
    s_t [1:0] a;                                                // typedef + outer dim
    struct packed { logic [95:0] p; logic [7:0] t; } [1:0] b;   // inline + outer dim
    s_t [1:0] c;
    struct packed { logic [95:0] p; logic [7:0] t; } d;         // inline, no dim
    a[0].p = `V; a[0].t = 8'hAB; r_td = a[0];
    b[0].p = `V; b[0].t = 8'hAB; r_il = b[0];
    c[1] = {`V, 8'hCD};        r_tv = c[1];                     // whole-element write
    d.p = `V; d.t = 8'hAB;     r_il1 = d;
    r_rd = a[0].p;                                              // element MEMBER read
  endtask
  initial begin t();
    $display("SUB td=%028x il=%028x whole=%028x il1=%028x rd=%024x", r_td, r_il, r_tv, r_il1, r_rd);
  end
endmodule
module tb;
  typedef struct packed { logic [95:0] p; logic [7:0] t; } s_t;
  logic [127:0] r_td, r_il;
  sub u ();
  initial begin
    s_t [1:0] a;
    struct packed { logic [95:0] p; logic [7:0] t; } [1:0] b;
    a[0].p = `V; a[0].t = 8'hAB; r_td = a[0];
    b[0].p = `V; b[0].t = 8'hAB; r_il = b[0];
    #1 $display("TOP td=%028x il=%028x", r_td, r_il);
  end
endmodule
"#);
    for expect in [
        "SUB td=00112233445566778899aabbccab il=00112233445566778899aabbccab \
          whole=00112233445566778899aabbcccd il1=00112233445566778899aabbccab \
          rd=112233445566778899aabbcc",
        "TOP td=00112233445566778899aabbccab il=00112233445566778899aabbccab",
    ] {
        let want: String = expect.split_whitespace().collect::<Vec<_>>().join(" ");
        let got: String = o.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(got.contains(&want), "expected `{want}` in:\n{o}");
    }
}

#[test]
fn nested_members_two_dims_function_locals_and_nba() {
    let o = out(r#"
`define V 96'h112233445566778899aabbcc
module sub (input logic clk);
  typedef struct packed { logic [7:0] x; logic [7:0] y; } in_t;
  typedef struct packed { logic [95:0] p; in_t q; } s_t;
  logic [127:0] r_nd, r_fn, r_nba;
  logic [15:0] r_nq;
  function automatic logic [111:0] f();
    s_t [1:0] a; a[1].p = `V; a[1].q.x = 8'h5A; a[1].q.y = 8'hA5; return a[1];
  endfunction
  task automatic t();
    struct packed { logic [95:0] p; in_t q; } [1:0][1:0] m;   // two outer dims, inline
    m[1][0].p = `V; m[1][0].q = 16'hBEEF; r_nd = m[1][0];
    r_nq = m[1][0].q;                                         // nested-struct member read
  endtask
  s_t [1:0] w;
  always @(posedge clk) begin w[0].p <= `V; w[0].q.x <= 8'h11; end
  initial begin t(); r_fn = f(); @(posedge clk); @(posedge clk); r_nba = w[0];
    $display("ND=%028x NQ=%04x FN=%028x NBA=%028x", r_nd, r_nq, r_fn, r_nba);
  end
endmodule
module tb; logic clk = 0; always #5 clk = ~clk; sub u (.clk(clk)); initial #40 $finish; endmodule
"#);
    assert!(
        o.contains("ND=112233445566778899aabbccbeef NQ=beef FN=112233445566778899aabbcc5aa5 NBA=112233445566778899aabbcc11xx"),
        "sibling shapes wrong:\n{o}"
    );
}

#[test]
fn typedef_with_two_packed_dims_parses_as_a_statement_local() {
    // §7.4.1: `s_t [1:0][1:0] m;` inside a block. The statement-declaration
    // lookahead stopped after the FIRST balanced `[..]` and demanded an
    // identifier, so the second `[` sent this down the expression path — a
    // parse error, while the identical declaration at module scope parsed.
    // The expression form `arr[i][j] = v` must still be an assignment.
    let o = out(r#"
module tb;
  typedef struct packed { logic [7:0] x; } s_t;
  logic [7:0] arr [2][2];
  initial begin
    s_t [1:0][1:0] loc;
    loc[1][0].x = 8'hA5;
    arr[1][1] = 8'h3C;                 // still an expression statement
    $display("LOC=%02x ARR=%02x", loc[1][0].x, arr[1][1]);
  end
endmodule
"#);
    assert!(o.contains("LOC=a5 ARR=3c"), "two-dim typedef local parse/shape wrong:\n{o}");
}

