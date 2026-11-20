//! §12.7.3: procedural `foreach` over a class property with a LONE packed
//! dimension (`bit [15:0] row; foreach (row[i])`).
//!
//! `class_prop_packed_dims` dropped a property's dimensions unless it had
//! at least TWO of them, so the single-variable member loop never received
//! the property's geometry and fell through to the width fallback, which
//! sizes the loop from a same-named module- or package-scope variable — or
//! from nothing at all (one iteration). Every width, declaration form
//! (static, signed, typedef'd, parameterised, `this.`-qualified, inherited,
//! task body) and receiver (bare in a method, `obj.member` at module
//! scope) collapsed the same way; only `bit [0:0]` agreed with the
//! fallback by accident.
//!
//! Every expectation is the reference simulator's.

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
fn foreach_over_lone_packed_member_iterates_the_declared_range() {
    let o = out(r#"
module top;
  class c;
    bit [3:0]    r4;
    bit [7:0]    r8;
    bit [15:0]   r16;
    bit [31:0]   r32;
    byte         rb;
    int          ri;
    logic [15:0] rl;
    bit [4:11]   rnz;   // non-zero bounds: 8 bits, i runs 4..11
    bit [0:15]   rasc;  // ascending declaration: i runs 0..15
    bit [0:0]    r1;
    int n4, n8, n16, n32, nb, ni, nl, nnz, nasc, n1, f_first, f_last;
    function void fill();
      n4 = 0;  foreach (r4[i])  begin n4++;  r4[i]  = 1'b1; end
      n8 = 0;  foreach (r8[i])  begin n8++;  r8[i]  = 1'b1; end
      n16 = 0; foreach (r16[i]) begin n16++; r16[i] = 1'b1; end
      n32 = 0; foreach (r32[i]) begin n32++; r32[i] = 1'b1; end
      nb = 0;  foreach (rb[i])  begin nb++;  rb[i]  = 1'b1; end
      ni = 0;  foreach (ri[i])  begin ni++;  ri[i]  = 1'b1; end
      nl = 0;  foreach (rl[i])  begin nl++;  rl[i]  = 1'b1; end
      f_first = -1; f_last = -1; nnz = 0;
      foreach (rnz[i]) begin
        if (f_first < 0) f_first = i;
        f_last = i;
        nnz++; rnz[i] = 1'b1;
      end
      nasc = 0; foreach (rasc[i]) begin nasc++; rasc[i] = 1'b1; end
      n1 = 0;  foreach (r1[i])  begin n1++;  r1[i]  = 1'b1; end
    endfunction
  endclass
  initial begin
    c o = new();
    o.fill();
    $display("N=%0d/%0d/%0d/%0d/%0d/%0d/%0d/%0d/%0d/%0d",
             o.n4, o.n8, o.n16, o.n32, o.nb, o.ni, o.nl, o.nnz, o.nasc, o.n1);
    $display("V=%h/%h/%h/%h/%h/%h/%h/%h/%h",
             o.r4, o.r8, o.r16, o.r32, o.rb, o.ri, o.rl, o.rnz, o.rasc);
    $display("B=%0d..%0d", o.f_first, o.f_last);
  end
endmodule
"#);
    assert!(
        o.contains("N=4/8/16/32/8/32/16/8/16/1")
            && o.contains("V=f/ff/ffff/ffffffff/ff/ffffffff/ffff/ff/ffff")
            && o.contains("B=4..11"),
        "lone-packed-dimension member foreach wrong:\n{o}"
    );
}

#[test]
fn foreach_over_lone_packed_member_decl_forms() {
    let o = out(r#"
module top;
  typedef bit [15:0] vec_t;
  class base;
    bit [15:0] brow;
  endclass
  class c extends base;
    static bit [15:0]     srow;
    bit signed [15:0]     sgrow;
    bit [0:15]            drow;
    vec_t                 trow;
    rand bit [15:0]       rrow;
    bit [15:0]            row;
    int ns, nsg, nd, nt, nr, nth, ntask, ninh, nguard;
    function void fill();
      ns = 0;  foreach (srow[i])  begin ns++;  srow[i]  = 1'b1; end
      nsg = 0; foreach (sgrow[i]) begin nsg++; sgrow[i] = 1'b1; end
      nd = 0;  foreach (drow[i])  begin nd++;  drow[i]  = 1'b1; end
      nt = 0;  foreach (trow[i])  begin nt++;  trow[i]  = 1'b1; end
      nr = 0;  foreach (rrow[i])  begin nr++;  rrow[i]  = 1'b1; end
      nth = 0; foreach (this.row[i]) begin nth++; this.row[i] = 1'b1; end
    endfunction
    task automatic fill_task();
      ntask = 0;
      foreach (row[i]) begin ntask++; row[i] = 1'b1; end
    endtask
    function void fill_inherited();
      ninh = 0;
      foreach (brow[i]) begin ninh++; brow[i] = 1'b1; end
    endfunction
    function void fill_guarded();
      nguard = 0;
      foreach (row[i]) if (i < 8) begin nguard++; row[i] = 1'b1; end
    endfunction
  endclass
  int ngv;   // module-scope read-back of the filled member
  initial begin
    c o = new();
    o.fill();
    o.fill_task();
    o.fill_inherited();
    o.fill_guarded();
    ngv = o.row;
    $display("D=%0d/%0d/%0d/%0d/%0d/%0d/%0d/%0d/%0d",
             o.ns, o.nsg, o.nd, o.nt, o.nr, o.nth, o.ntask, o.ninh,
             o.nguard);
    $display("S=%h/%h/%h/%h/%h/%h/%h/%h",
             c::srow, o.sgrow, o.drow, o.trow, o.rrow, o.brow, o.row, ngv);
  end
endmodule
"#);
    assert!(
        o.contains("D=16/16/16/16/16/16/16/16/8")
            // ngv is a 32-bit int, so %h zero-extends the 16-bit member
            && o.contains("S=ffff/ffff/ffff/ffff/ffff/ffff/ffff/0000ffff"),
        "declaration-form member foreach wrong:\n{o}"
    );
}

#[test]
fn foreach_over_obj_member_at_module_scope_reads_and_writes() {
    let o = out(r#"
module top;
  class c;
    bit [15:0] row;
  endclass
  initial begin
    c r = new();
    int n = 0;
    int ones = 0;
    foreach (r.row[i]) begin n++; r.row[i] = 1'b1; end
    foreach (r.row[i]) ones += r.row[i];
    $display("W=%0d/%h R=%0d/%0d", n, r.row, ones, ones);
  end
endmodule
"#);
    assert!(
        o.contains("W=16/ffff R=16/16"),
        "module-scope obj.member foreach wrong:\n{o}"
    );
}

#[test]
fn same_named_scope_variable_does_not_size_the_member_loop() {
    let o = out(r#"
package mask_pkg;
  bit [3:0]  mrow_n;   // narrower than the 16-bit property
  bit [15:0] mrow_w;   // wider    than the 8-bit  property
  bit [15:0] mrow_s;   // same     as the 16-bit property
endpackage
module top;
  class cn;
    bit [15:0] mrow_n;
    int iters;
    function void fill();
      iters = 0;
      foreach (mrow_n[i]) begin iters++; mrow_n[i] = 1'b1; end
    endfunction
  endclass
  class cw;
    bit [7:0] mrow_w;
    int iters;
    function void fill();
      iters = 0;
      foreach (mrow_w[i]) begin iters++; mrow_w[i] = 1'b1; end
    endfunction
  endclass
  class cs;
    bit [15:0] mrow_s;
    int iters;
    function void fill();
      iters = 0;
      foreach (mrow_s[i]) begin iters++; mrow_s[i] = 1'b1; end
    endfunction
  endclass
  class cm;
    bit [15:0] mrow_m;
    int iters;
    function void fill();
      iters = 0;
      foreach (mrow_m[i]) begin iters++; mrow_m[i] = 1'b1; end
    endfunction
  endclass
  bit [3:0] mrow_m;   // module-scope, narrower
  initial begin
    cn a = new(); cw b = new(); cs c = new(); cm d = new();
    mrow_m = 4'h0;
    a.fill(); b.fill(); c.fill(); d.fill();
    $display("P=%0d/%h %0d/%h %0d/%h M=%0d/%h/%h",
             a.iters, a.mrow_n, b.iters, b.mrow_w, c.iters, c.mrow_s,
             d.iters, d.mrow_m, mrow_m);
  end
endmodule
"#);
    assert!(
        o.contains("P=16/ffff 8/ff 16/ffff M=16/ffff/0"),
        "same-named scope variable sized the member loop:\n{o}"
    );
}
