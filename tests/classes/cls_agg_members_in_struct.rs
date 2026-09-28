//! IEEE 1800-2017 §7.2/§18.4 — aggregate members of struct-valued class
//! properties: element, part-select and whole-member stores and reads at
//! any member depth, whole-property copy-outs into module and block-local
//! struct variables, by-value returns, and NBA element stores.
//! Validated against a reference simulator (the `cls_agg_members_matrix`
//! family).
//!
//! Four defect families, all silent:
//!
//! 1. An element / part-select on an aggregate class-property member
//!    (`pkt.mem[i]`, `pkt.mem[1][255:248]`, `pkt.inner.mem[k]`) resolved
//!    only one member level deep, so stores were dropped and reads fell
//!    through to a single-BIT select of the member slice.
//! 2. A block-local PACKED struct variable registered its members'
//!    element strides one level deep: `t.mem[i]` after a copy-out worked,
//!    but `t.inner.mem[i]` read back one bit.
//! 3. The member-wise copy-out of an unpacked-struct property moved each
//!    member as ONE cell, so every element of an unpacked-array member
//!    (`mem [4]`) read back 0 (procedural local) or x (module var).
//! 4. The same receiver-resolution gap on the NBA path dropped nonblocking
//!    element stores from a clocked always block.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// Element stores inside a method, element reads at module scope and back
/// through a method, a module-scope store through the flattened name, and a
/// copy-out into a block-local packed struct variable — every element must
/// survive with its full width (the top bit included).
#[test]
fn method_element_store_and_copy_out() {
    let src = r#"
module tb;
  typedef struct packed { bit [31:0] tag; bit [3:0][255:0] mem; } PktB;
  class BDrv;
    PktB pkt;
    function void fill();
      pkt.tag = 32'h0BAD0BAD;
      pkt.mem[0] = 256'hA5;
      pkt.mem[1] = 256'h80000000000000000000000000000000000000000000000000000000000000A6;
      pkt.mem[2] = 256'h5A;
      pkt.mem[3] = 256'hA9;
    endfunction
    function bit [255:0] rd(int i); return pkt.mem[i]; endfunction
  endclass
  BDrv bd;
  int errors = 0;
  initial begin : main
    begin : blk
      PktB t;
      bit [255:0] got;
      bd = new();
      bd.fill();
      got = bd.pkt.mem[0]; if (got !== 256'hA5) errors++;
      got = bd.rd(2);      if (got !== 256'h5A) errors++;
      bd.pkt.mem[2] = 256'hDEAD;               // module-scope elem store
      got = bd.pkt.mem[2]; if (got !== 256'hDEAD) errors++;
      bd.pkt.mem[1][255:248] = 8'hAB;          // part-select into an element
      got = bd.pkt.mem[1];
      if (got !== 256'hAB000000000000000000000000000000000000000000000000000000000000A6) errors++;
      if (bd.pkt.mem[1][255:248] !== 8'hAB) errors++;
      t = bd.pkt;                              // copy-out to a block local
      got = t.mem[0]; if (got !== 256'hA5) errors++;
      got = t.mem[1];
      if (got !== 256'hAB000000000000000000000000000000000000000000000000000000000000A6) errors++;
      got = t.mem[3]; if (got !== 256'hA9) errors++;
      if (t.tag !== 32'h0BAD0BAD) errors++;
    end
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "errors"), 0, "element/part-select/copy-out checks");
}

/// Depth-2 members: element stores inside a method (`pkt.inner.mem[k]`), a
/// member-of-member whole store through a method, and reads of both through
/// a copy-out into a block-local nested-struct variable.
#[test]
fn depth_two_member_stores_and_copy_out() {
    let src = r#"
module tb;
  typedef struct packed { bit [31:0] tag; bit [3:0][255:0] mem; } InnerT;
  typedef struct packed { bit [31:0] tag; InnerT inner; } OuterT;
  class ODrv;
    OuterT pkt;
    function void fill_inner();
      pkt.inner.tag = 32'h0BAD0BAD;
      for (int k = 0; k < 4; k++) pkt.inner.mem[k] = 256'hB6 + k;
    endfunction
    function void set_inner(InnerT ii); pkt.inner = ii; endfunction
  endclass
  ODrv od;
  int errors = 0;
  initial begin : main
    begin : blk
      OuterT ot;
      InnerT ii;
      od = new();
      od.fill_inner();
      ot = od.pkt;
      for (int p = 0; p < 4; p++)
        if (ot.inner.mem[p] !== 256'hB6 + p) errors++;
      if (ot.inner.tag !== 32'h0BAD0BAD) errors++;
      ii.tag = 32'h1BAD1BAD;
      for (int p = 0; p < 4; p++) ii.mem[p] = 256'hC7 + p;
      od.set_inner(ii);                        // member-of-member whole store
      ot = od.pkt;
      if (ot.inner.tag !== 32'h1BAD1BAD) errors++;
      for (int p = 0; p < 4; p++)
        if (ot.inner.mem[p] !== 256'hC7 + p) errors++;
    end
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "errors"), 0, "depth-2 member checks");
}

/// An unpacked-array member of an unpacked-struct property: element reads
/// and stores (K5), a by-value return spread into a block local (K7r), and
/// whole-property copy-outs into BOTH a block local (K7b) and a module-scope
/// variable — the copy must move every element, not one whole-member cell.
#[test]
fn unpacked_array_member_copy_outs() {
    let src = r#"
module tb;
  typedef struct { bit [63:0] id; bit [255:0] mem [4]; } UPktA;
  class UADrv;
    UPktA pkt;
    function void fill();
      pkt.id = 64'h1122334455667788;
      for (int k = 0; k < 4; k++) pkt.mem[k] = 256'hA5 + k;
    endfunction
    function UPktA get_pkt(); return pkt; endfunction
  endclass
  UADrv ua;
  UPktA t_mod;
  int errors = 0;
  initial begin : main
    begin : blk
      UPktA t2, t3;
      bit [255:0] got;
      ua = new();
      ua.fill();
      got = ua.pkt.mem[1]; if (got !== 256'hA6) errors++;   // K5 elem read
      ua.pkt.mem[2] = 256'hFACE;                            // K5 elem store
      got = ua.pkt.mem[2]; if (got !== 256'hFACE) errors++;
      t3 = ua.get_pkt();                                    // by-value return
      for (int p = 0; p < 4; p++)
        if (t3.mem[p] !== (p == 2 ? 256'hFACE : 256'hA5 + p)) errors++;
      if (t3.id !== 64'h1122334455667788) errors++;
      t2 = ua.pkt;                                          // copy-out: local
      for (int p = 0; p < 4; p++)
        if (t2.mem[p] !== (p == 2 ? 256'hFACE : 256'hA5 + p)) errors++;
      if (t2.id !== 64'h1122334455667788) errors++;
      t_mod = ua.pkt;                                       // copy-out: module var
      for (int p = 0; p < 4; p++)
        if (t_mod.mem[p] !== (p == 2 ? 256'hFACE : 256'hA5 + p)) errors++;
      if (t_mod.id !== 64'h1122334455667788) errors++;
    end
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(
        u(&sim, "errors"),
        0,
        "unpacked-array member copy-out checks"
    );
}

/// A nonblocking element store into an aggregate class-property member from
/// a clocked always block must land (K4) and not clobber the element (K7n).
#[test]
fn nba_element_store_from_always() {
    let src = r#"
module tb;
  typedef struct packed { bit [31:0] tag; bit [3:0][255:0] mem; } PktB;
  typedef struct { bit [63:0] id; bit [255:0] mem [4]; } UPktA;
  class BDrv; PktB pkt; endclass
  class UADrv; UPktA pkt; endclass
  BDrv bd;
  UADrv ua;
  logic clk = 0;
  always #1 clk = ~clk;
  bit go = 0;
  int errors = 0;
  always @(posedge clk) if (go) begin
    bd.pkt.mem[0] <= 256'hFACE;
    ua.pkt.mem[0] <= 256'hFACE;
  end
  initial begin : main
    bd = new(); ua = new();
    bd.pkt.tag = 32'h1;
    bd.pkt.mem[0] = 256'h11;
    ua.pkt.id = 64'h22;
    ua.pkt.mem[0] = 256'h33;
    #3 go = 1;
    #5 go = 0;
    #2;
    if (bd.pkt.mem[0] !== 256'hFACE) errors++;
    if (bd.pkt.tag  !== 32'h1)       errors++;
    if (ua.pkt.mem[0] !== 256'hFACE) errors++;
    if (ua.pkt.id    !== 64'h22)     errors++;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "errors"), 0, "NBA element store checks");
}

/// A multi-dim unpacked member (`[2][2]`) of an unpacked-struct property
/// copies out per element through both indices (Cartesian enumeration).
/// The destination here is a module-scope variable: a BLOCK-LOCAL
/// destination's per-element writes land, but a multi-index READ of a
/// block-local unpacked member (`m.mm[i][j]`) is a pre-existing gap in the
/// frame-leaf resolver that predates this change and is out of its scope.
#[test]
fn multidim_member_copy_out() {
    let src = r#"
module tb;
  typedef struct { bit [7:0] mm [2][2]; } MStruct;
  class MDrv;
    MStruct s;
    function void fill();
      for (int i = 0; i < 2; i++)
        for (int j = 0; j < 2; j++)
          s.mm[i][j] = 8'h10 + i*2 + j;
    endfunction
  endclass
  MDrv md;
  MStruct mv;
  int errors = 0;
  initial begin : main
    md = new();
    md.fill();
    if (md.s.mm[1][0] !== 8'h12) errors++;    // element read, both indices
    mv = md.s;                                // copy-out to a module var
    for (int i = 0; i < 2; i++)
      for (int j = 0; j < 2; j++)
        if (mv.mm[i][j] !== 8'h10 + i*2 + j) errors++;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "errors"), 0, "multi-dim member copy-out checks");
}

/// Two block-local packed-struct variables reuse the same `var.member` key
/// with DIFFERENT packed geometry: the later declaration must REPLACE the
/// earlier's registered dims. An insert-only-if-absent registration kept
/// the first block's geometry for the second block's `t.mem[1]`, so `fb`
/// read the wrong slice (`t.mem[1]` 16-bit instead of 8-bit).
#[test]
fn block_local_struct_member_dims_replace() {
    let src = r#"
module tb;
  typedef struct packed { bit [1:0][3:0][3:0] mem; } A_t;
  typedef struct packed { bit [3:0][1:0][3:0] mem; } B_t;
  int errors = 0;
  initial begin : main
    begin : fa
      A_t t;
      t.mem = 32'h87654321;
      if (t.mem[1]     !== 16'h8765) errors++;
      if (t.mem[1][1]  !== 4'h6)     errors++;
    end
    begin : fb
      B_t t;
      t.mem = 32'h87654321;
      if (t.mem[1]     !== 8'h43)    errors++;
      if (t.mem[1][1]  !== 4'h4)     errors++;
    end
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(
        u(&sim, "errors"),
        0,
        "same-named member: later dims replace earlier"
    );
}

/// Collection members of an unpacked-struct class property (§7.8): a
/// dynamic `[]`, a queue `[$]` and an associative `[int]` member take
/// element stores and reads in a method and at module scope, `new[n]`
/// sizing, `push_back` and `size()` — all through the instance-scoped
/// `<handle>#<prop>.<member>` store the struct-member probe registers.
#[test]
fn struct_collection_member_family() {
    let src = r#"
module tb;
  typedef struct {
    bit [63:0]  id;
    bit [255:0] dmem [];
    bit [255:0] qmem [$];
    bit [255:0] amem [int];
  } DPkt;
  class DDrv;
    DPkt pkt;
    function void fill();
      pkt.id = 64'h1122334455667788;
      pkt.dmem = new[4];                       // new[n], METHOD scope
      for (int k = 0; k < 4; k++)
        pkt.dmem[k] = 256'hA5 + k;             // dyn element store, METHOD scope
      for (int k = 0; k < 4; k++)
        pkt.qmem.push_back(256'h5A + k);       // push_back, METHOD scope
      pkt.amem[2] = 256'hC3;                   // assoc element store, METHOD scope
    endfunction
    function bit [255:0] rd_d(int k); return pkt.dmem[k]; endfunction
    function bit [255:0] rd_q(int k); return pkt.qmem[k]; endfunction
    function bit [255:0] rd_a(int k); return pkt.amem[k]; endfunction
  endclass
  DDrv dd;
  int errors = 0;
  initial begin : main
    dd = new();
    dd.fill();
    // reads inside a method
    if (dd.rd_d(1) !== 256'hA6) errors++;
    if (dd.rd_q(2) !== 256'h5C) errors++;
    if (dd.rd_a(2) !== 256'hC3) errors++;
    // element reads at module scope
    if (dd.pkt.dmem[3] !== 256'hA8) errors++;
    if (dd.pkt.qmem[0] !== 256'h5A) errors++;
    if (dd.pkt.amem[2] !== 256'hC3) errors++;
    // size() of the queue member
    if (dd.pkt.qmem.size() != 4) errors++;
    // element stores at module scope
    dd.pkt.dmem[0] = 256'hF0;
    if (dd.pkt.dmem[0] !== 256'hF0) errors++;
    dd.pkt.amem[7] = 256'hE9;
    if (dd.pkt.amem[7] !== 256'hE9) errors++;
    // push_back at module scope
    dd.pkt.qmem.push_back(256'h77);
    if (dd.pkt.qmem.size() != 5) errors++;
    if (dd.pkt.qmem[4]   !== 256'h77) errors++;
    if (dd.pkt.id !== 64'h1122334455667788) errors++;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(
        u(&sim, "errors"),
        0,
        "dynamic/queue/associative struct-member family"
    );
}