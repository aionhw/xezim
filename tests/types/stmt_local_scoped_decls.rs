//! §6.20.2/§6.18/§23.10: a declaration INSIDE a statement — a task or function
//! local, a named block, a fork branch — must be sized in the scope that wrote
//! it, even when that scope is a submodule instance.
//!
//! Elaboration binds a module's parameters and typedefs to their BARE names
//! only while that module is being elaborated; afterwards only the
//! instance-qualified keys survive (`u_fifo.DW`, `u_fifo.word_t`). A
//! module-LEVEL declaration is sized during that window, but a statement-local
//! one is sized much later, from the flat design-wide map — where the bare name
//! it spells matches nothing. Two different silent truncations followed:
//!
//!   * a parameterized width (`logic [DW-1:0] v;`) lost the dimension outright
//!     — `resolve_type_width` SKIPS a dimension it cannot evaluate — leaving
//!     the local ONE BIT wide, so every value stored in it became its LSB;
//!   * a typedef'd width (`word_t v;`) fell back to the 32-bit default.
//!
//! Neither is visible in a small design: it needs a submodule (the top's own
//! names stay bare-bound and so always worked) and a type wider than 32 bits.
//! It surfaced as a memory BFM whose 128-bit read beats all read back as 0 or
//! 1 — the low bit of the data that should have been there.
//!
//! Both are now folded into the declaration while the names are still in
//! scope. Every expected value below is the reference simulator's.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 200).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_submodule_parameter_sizes_statement_local_declarations() {
    // Two instances of one module with DIFFERENT parameters: each local must
    // take its OWN instance's width, so the same store truncates differently.
    // A block-scope `localparam` of the same name shadows the parameter.
    let o = out(r#"
module w #(parameter int DW = 8, parameter int NE = 2) ();
  logic [95:0] r_task, r_func, r_deep, r_arr, r_shadow;
  task automatic t();
    logic [DW-1:0] v;
    logic [7:0] a [NE-1:0];               // unpacked dimension, also parameterized
    v = 96'h112233445566778899aabbcc;
    r_task = v;
    foreach (a[i]) a[i] = 8'hA0 + i[7:0];
    r_arr = 0;
    foreach (a[i]) r_arr[i*8 +: 8] = a[i];
  endtask
  function automatic logic [DW-1:0] f();
    logic [DW-1:0] v; v = 96'h112233445566778899aabbcc; return v;
  endfunction
  task automatic deep();                  // fork branch inside a loop
    for (int i = 0; i < 1; i++) fork
      begin : nest
        logic [DW-1:0] v; v = 96'h112233445566778899aabbcc; r_deep = v;
      end
    join
  endtask
  task automatic shad();
    begin : blk
      localparam int DW = 16;             // shadows the module parameter
      logic [DW-1:0] v; v = 96'h112233445566778899aabbcc; r_shadow = v;
    end
  endtask
  initial begin
    t(); r_func = f(); deep(); shad();
    $display("W%0d task=%024x func=%024x deep=%024x arr=%024x shadow=%024x",
             DW, r_task, r_func, r_deep, r_arr, r_shadow);
  end
endmodule
module tb;
  w #(.DW(96), .NE(4)) u96 ();
  w #(.DW(32), .NE(2)) u32 ();
endmodule
"#);
    for expect in [
        "W96 task=112233445566778899aabbcc func=112233445566778899aabbcc \
          deep=112233445566778899aabbcc arr=0000000000000000a3a2a1a0 \
          shadow=00000000000000000000bbcc",
        "W32 task=000000000000000099aabbcc func=000000000000000099aabbcc \
          deep=000000000000000099aabbcc arr=00000000000000000000a1a0 \
          shadow=00000000000000000000bbcc",
    ] {
        let want: String = expect.split_whitespace().collect::<Vec<_>>().join(" ");
        let got: String = o.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(got.contains(&want), "expected `{want}` in:\n{o}");
    }
}

#[test]
fn a_submodule_typedef_sizes_statement_local_declarations() {
    // The typedef half of the same bug — and it does NOT need a parameter:
    // `sl_t`/`vl_t` are written with literal widths and still defaulted to 32
    // bits inside the task. `chain_t` covers a typedef of a typedef.
    let o = out(r#"
`define V 256'h0f1e2d3c4b5a69788796a5b4c3d2e1f0
module sub #(parameter int DW = 128) ();
  typedef struct packed { logic [DW-1:0] p; logic [7:0] t; } sp_t;
  typedef struct packed { logic [127:0]  p; logic [7:0] t; } sl_t;
  typedef logic [DW-1:0] vp_t;
  typedef logic [127:0]  vl_t;
  typedef vl_t           chain_t;
  logic [255:0] r_sp, r_sl, r_vp, r_vl, r_ch;
  task automatic t();
    sp_t sp; sl_t sl; vp_t vp; vl_t vl; chain_t ch;
    sp.p = `V; sp.t = 8'hAB; r_sp = sp;
    sl.p = `V; sl.t = 8'hAB; r_sl = sl;
    vp = `V; r_vp = vp;
    vl = `V; r_vl = vl;
    ch = `V; r_ch = ch;
  endtask
  initial begin t();
    $display("SP=%040x SL=%040x", r_sp, r_sl);
    $display("VP=%040x VL=%040x CH=%040x", r_vp, r_vl, r_ch);
  end
endmodule
module tb; sub #(.DW(128)) u (); endmodule
"#);
    for expect in [
        "SP=0000000f1e2d3c4b5a69788796a5b4c3d2e1f0ab \
          SL=0000000f1e2d3c4b5a69788796a5b4c3d2e1f0ab",
        "VP=000000000f1e2d3c4b5a69788796a5b4c3d2e1f0 \
          VL=000000000f1e2d3c4b5a69788796a5b4c3d2e1f0 \
          CH=000000000f1e2d3c4b5a69788796a5b4c3d2e1f0",
    ] {
        let want: String = expect.split_whitespace().collect::<Vec<_>>().join(" ");
        let got: String = o.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(got.contains(&want), "expected `{want}` in:\n{o}");
    }
}

#[test]
fn a_parameterized_enum_base_is_not_rejected_in_a_submodule() {
    // The §6.19 lint reads the base width out of the flat parameter map, where
    // a submodule's `DW` is not spelled bare. It measured 1 bit and then
    // REFUSED to elaborate: every member "too large for its base type", and
    // all of them "the same value 0". `enum logic [DW-1:0]` was unusable
    // anywhere but the top module. The lint now stands down on a width it
    // cannot determine, rather than reporting a violation it cannot see.
    let o = out(r#"
module sub #(parameter int DW = 48) ();
  typedef enum logic [DW-1:0] { Z = 48'h0, ONE = 48'h112233445566 } ep_t;
  ep_t m;
  logic [63:0] r_loc;
  task automatic t(); ep_t e; e = ONE; r_loc = e; endtask
  initial begin m = ONE; t(); $display("EP mod=%012x loc=%012x", m, r_loc); end
endmodule
module tb; sub #(.DW(48)) u (); endmodule
"#);
    assert!(
        o.contains("EP mod=112233445566 loc=112233445566"),
        "parameterized enum base rejected or mis-sized:\n{o}"
    );
}

#[test]
fn instances_with_different_parameters_do_not_share_a_width() {
    // The fold happens per (module, parameter-set) and is CACHED, so this
    // pins that two parameterizations cannot collapse onto one another —
    // including a third instance that shares `a`'s parameters (and therefore
    // its cache entry) and one reached through an intermediate module.
    let o = out(r#"
`define V 128'h0f1e2d3c4b5a69788796a5b4c3d2e1f0
module leaf #(parameter int DW = 128) ();
  typedef logic [DW-1:0] vp_t;
  typedef struct packed { logic [DW-1:0] p; logic [7:0] t; } sp_t;
  logic [255:0] r_v, r_s;
  task automatic t(); vp_t v; sp_t s;
    v = `V; r_v = v; s.p = `V; s.t = 8'hAB; r_s = s;
  endtask
  initial begin t(); $display("DW%0d v=%040x s=%040x", DW, r_v, r_s); end
endmodule
module mid #(parameter int MW = 64) (); leaf #(.DW(MW)) l (); endmodule
module tb;
  leaf #(.DW(128)) a ();
  leaf #(.DW(64))  b ();
  leaf #(.DW(128)) c ();          // shares a's prepare-cache entry
  mid  #(.MW(96))  m ();          // parameter arrives through another module
endmodule
"#);
    for expect in [
        "DW128 v=000000000f1e2d3c4b5a69788796a5b4c3d2e1f0 \
           s=0000000f1e2d3c4b5a69788796a5b4c3d2e1f0ab",
        "DW64 v=0000000000000000000000008796a5b4c3d2e1f0 \
           s=00000000000000000000008796a5b4c3d2e1f0ab",
        "DW96 v=00000000000000004b5a69788796a5b4c3d2e1f0 \
           s=000000000000004b5a69788796a5b4c3d2e1f0ab",
    ] {
        let want: String = expect.split_whitespace().collect::<Vec<_>>().join(" ");
        let got: String = o.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(got.contains(&want), "expected `{want}` in:\n{o}");
    }
}
