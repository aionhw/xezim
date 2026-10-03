// Self-test: whole-collection EQUALITY must not depend on how each side's
// elements were stored (IEEE 1800-2017 §7.2, §11.4.5).
//
// Reproduces the packed/unpacked object compare() miscompare behind the
// report-message element-table regression family: after do_copy() copies a
// randomized object and a pack/unpack round-trip rebuilds the other one,
// `do_compare`'s `q == rhs.q` / `sa == rhs.sa` / `da == rhs.da` returned 0
// although every element read compared equal.
//
// Element stores must normalize to the declared element type regardless of
// the stored expression's width/signedness:
//  - a same-width unsigned local (bit/logic [15:0], the pack/unpack macro
//    shape) stored into a shortint element,
//  - a wide int local stored into a shortint element,
//  - push_back vs assignment-pattern vs index-assign on narrow elements.
// Class-property associative array equality must also yield 1/0, never x.
class c;
  rand shortint da [];
  rand byte     q  [$];
  rand int      sa [2];
  constraint cs { da.size == 2; q.size == 2; }

  byte q_pb  [$];
  byte q_pat [$];
  byte q_idx [$];
  int  aa    [int];
endclass

module top;
  c h, m1, m2;
  int fails;
  initial begin
    fails = 0;

    // ---- pack/unpack round-trip shape (the UVM regression) ----
    m1 = new; m2 = new;
    void'(m2.randomize());
    m1.da = m2.da; m1.q = m2.q; m1.sa = m2.sa;      // do_copy shape
    m2.da = new[2]; m2.q.delete();
    for (int i = 0; i < 2; i++) begin
      logic [15:0] v16; logic [7:0] v8; logic [31:0] v32;
      v16 = m1.da[i];                                  // unpack macro locals
      m2.da[i] = v16;
      v8 = m1.q[i];
      m2.q.push_back(v8);
      v32 = m1.sa[i];
      m2.sa[i] = v32;
    end
    if (!(m1.da == m2.da)) begin fails++; $display("FAIL unpack dyn-array da"); end
    if (!(m1.q  == m2.q )) begin fails++; $display("FAIL unpack queue q"); end
    if (!(m1.sa == m2.sa)) begin fails++; $display("FAIL unpack sarray sa"); end

    // ---- element-store width/signedness variants vs copy-built side ----
    m2.da = new[2];
    for (int i = 0; i < 2; i++) m2.da[i] = m1.da[i]; // direct same-type source
    if (!(m1.da == m2.da)) begin fails++; $display("FAIL direct shortint store"); end

    m2.da = new[2];
    for (int i = 0; i < 2; i++) begin
      bit [15:0] v; v = m1.da[i]; m2.da[i] = v;      // 2-state same-width local
    end
    if (!(m1.da == m2.da)) begin fails++; $display("FAIL bit[15:0] local store"); end

    m2.da = new[2];
    for (int i = 0; i < 2; i++) begin
      int v; v = m1.da[i]; m2.da[i] = v;             // wide int local
    end
    if (!(m1.da == m2.da)) begin fails++; $display("FAIL int local store"); end

    // ---- construction-path variants on a byte queue ----
    h = new;
    h.q_pb.push_back(-3); h.q_pb.push_back(4);
    h.q_pat = '{-3, 4};
    if (!(h.q_pb == h.q_pat)) begin fails++; $display("FAIL byte queue pb-vs-pattern"); end
    h.q_idx.push_back(0); h.q_idx[0] = -3;
    h.q_pb.delete(); h.q_pb.push_back(-3);
    if (!(h.q_pb == h.q_idx)) begin fails++; $display("FAIL byte queue pb-vs-idx"); end
    begin
      byte q_copy [$];
      q_copy = h.q_idx;
      if (!(h.q_idx == q_copy)) begin fails++; $display("FAIL byte queue copy"); end
    end

    // ---- class-property associative array equality must not be x ----
    h.aa[1] = 5;
    if ((h.aa == h.aa) !== 1'b1) begin fails++; $display("FAIL assoc self-eq is-x"); end
    begin
      int aa2 [int];
      h.aa[1] = -7; h.aa[2] = 9;
      aa2[1] = -7; aa2[2] = 9;                     // element-built equal twin
      if (!(h.aa == aa2)) begin fails++; $display("FAIL assoc cross-eq"); end
      aa2.delete(); aa2[1] = -7;                   // different size -> unequal
      if (h.aa == aa2)  begin fails++; $display("FAIL assoc size-mismatch"); end
    end

    // ---- std::randomize(obj) must randomize the object, not corrupt it ----
    // (a class-handle argument is obj.randomize(), never a scalar draw into
    // the pointer; the corrupted handle left the assoc size 0 and == x)
    begin
      c z;
      z = new;
      void'(std::randomize(z));                   // item has no rand members
      z.aa[1] = 3;
      if ((z.aa == z.aa) !== 1'b1) begin fails++; $display("FAIL std-rand object assoc x"); end
      if (z.aa.size() != 1) begin fails++; $display("FAIL std-rand object assoc size"); end
    end

    if (fails == 0) $display("TAG_PASS");
    else             $display("TAG_FAIL fails=%0d", fails);
  end
endmodule