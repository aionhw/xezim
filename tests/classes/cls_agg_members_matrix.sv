`timescale 1ns/1ps

// ----- self-check macros (tests/classes SVTEST_* style, inlined) -----
`ifndef SVTEST_DEFS_SVH
`define SVTEST_DEFS_SVH

`define SVTEST_INIT \
int failures = 0;

`define SVTEST_CHECK(expr, msg) \
if (!(expr)) begin \
  failures++; \
  $display("FAIL @%0t : %s", $time, msg); \
end

`define SVTEST_PASSFAIL \
if (failures == 0) begin \
  $display("TEST_PASS"); \
end else begin \
  $display("TEST_FAIL count=%0d", failures); \
  $fatal(1); \
end

`endif

// ----- production-shaped packed struct: 524,640 bits -----
typedef struct packed {
  bit [63:0]          id;
  bit [31:0]          len;
  bit [255:0]         attr;
  bit [2047:0][255:0] mem;          // packed-array member, 2048 x 256-bit
} BigPkt;

// ----- small control struct: 1,504 bits -----
typedef struct packed {
  bit [63:0]       id;
  bit [31:0]       len;
  bit [255:0]      attr;
  bit [3:0][255:0] mem;
} SmallPkt;

// ----- width probe: wide SCALAR member next to an array member -----
typedef struct packed {
  bit [63:0]    id;
  bit [2047:0]  wide;               // 2048-bit scalar member
  bit [3:0][255:0] mem;             // 1024-bit array member
} WidePkt;

// ----- nesting probe: packed struct inside packed struct -----
typedef struct packed {
  bit [31:0]       tag;
  bit [3:0][255:0] mem;
} InnerT;

typedef struct packed {
  bit [31:0] tag;
  InnerT     inner;
} OuterT;

// ----- unpacked-struct contrast -----
typedef struct {
  bit [63:0]       id;
  bit [3:0][255:0] mem;             // packed-array member of an UNPACKED struct
} UPkt;

// ----- unpacked-ARRAY member of an unpacked struct (K7) -----
typedef struct {
  bit [63:0]  id;
  bit [255:0] mem [4];              // UNPACKED-array member (dims after name)
} UPktA;

// ----- dynamic/multi-dim container members of an unpacked struct (K8) -----
typedef struct {
  bit [63:0]  id;
  bit [255:0] dmem [];              // DYNAMIC-array member
  bit [255:0] qmem [$];             // QUEUE member
  bit [255:0] amem [int];           // ASSOCIATIVE-array member
  bit [255:0] mmem [2][2];          // MULTI-DIM unpacked-array member
  string     sname;                 // string member (boundary: works)
} UPktD;

// __MWE_CLASSES__

class BigDrv;
  BigPkt pkt;                       // packed-struct class property

  function void fill(int n);        // element stores, METHOD scope
    bit [159:0] din;
    for (int k = 0; k < n; k++) begin
      din  = 160'h00A5000000000000 + k;
      pkt.mem[k] = din;             // PRIMARY BUG PATH: dropped by xezim
    end
  endfunction

  function BigPkt prepare();        // production-faithful by-value return
    fill(4);
    pkt.id   = 64'h00D15EA5E5C0FFEE;
    pkt.len  = 32'd4;
    pkt.attr = 256'h7126330007071000;
    return pkt;
  endfunction

  function void load(BigPkt p);     // whole-struct property store, method scope
    pkt = p;
  endfunction

  function void set_mem_arr(bit [2047:0][255:0] a);  // whole-member store, method scope
    pkt.mem = a;
  endfunction

  function bit [255:0] peek(int i); // element READ, method scope
    return pkt.mem[i];
  endfunction
endclass

class SmallDrv;
  SmallPkt pkt;
  function void fill(int n);
    bit [159:0] din;
    for (int k = 0; k < n; k++) begin
      din  = 160'h00A5000000000000 + k;
      pkt.mem[k] = din;             // same bug, small struct
    end
  endfunction
  function SmallPkt prepare();
    fill(4);
    pkt.id   = 64'h00D15EA5E5C0FFEE;
    pkt.len  = 32'd4;
    pkt.attr = 256'h7126330007071000;
    return pkt;
  endfunction
endclass

class WideDrv;
  WidePkt pkt;
endclass

class OuterDrv;
  OuterT pkt;
  function void fill_inner(int n);  // element store at struct DEPTH 2
    bit [159:0] din;
    for (int k = 0; k < n; k++) begin
      din  = 160'h00A5000000000000 + k;
      pkt.inner.mem[k] = din;       // cousin: member-of-member element store
    end
  endfunction
  function void set_inner(InnerT i); // member-of-member WHOLE store (contrast)
    pkt.inner = i;
  endfunction
endclass

class UPktDrv;
  UPkt pkt;                         // UNPACKED-struct property (contrast)
  function void fill(int n);
    bit [159:0] din;
    for (int k = 0; k < n; k++) begin
      din  = 160'h00A5000000000000 + k;
      pkt.mem[k] = din;             // expected OK: member-wise storage model
    end
  endfunction
endclass

class UADrv;                        // unpacked-ARRAY member property (K7)
  UPktA pkt;
  function void fill(int n);        // element stores into UNPACKED-array member
    for (int k = 0; k < n; k++)
      pkt.mem[k] = {96'b0, 160'h00A5000000000000} + k;
    pkt.id = 64'h1122334455667788;
  endfunction
  function bit [255:0] rd(int k);   // element read, METHOD scope
    return pkt.mem[k];
  endfunction
  function UPktA get_pkt();         // by-value return of the unpacked struct
    return pkt;
  endfunction
endclass

class UDynDrv;                       // dynamic/multi-dim container members (K8)
  UPktD      pkt;
  bit [255:0] dprop [];              // DIRECT dynamic-array property (boundary)
  bit [255:0] qprop [$];             // DIRECT queue property (boundary)
  function void fill();
    pkt.dmem = new[4];
    for (int k = 0; k < 4; k++) begin
      pkt.dmem[k] = {96'b0, 160'h00A5000000000000} + k;      // dyn elem store, METHOD scope
      pkt.qmem.push_back({96'b0, 160'h00A5000000000000} + k); // queue push, METHOD scope
      pkt.amem[k]   = {96'b0, 160'h00A5000000000000} + k;    // assoc elem store, METHOD scope
    end
    pkt.mmem[0][0] = {96'b0, 160'h00A5000000000000};
    pkt.mmem[0][1] = {96'b0, 160'h00A5000000000000} + 1;
    pkt.mmem[1][0] = {96'b0, 160'h00A5000000000000} + 2;
    pkt.mmem[1][1] = {96'b0, 160'h00A5000000000000} + 3;
    pkt.id    = 64'h1122334455667788;
    pkt.sname = "HELLO";
    dprop = new[4];
    for (int k = 0; k < 4; k++) dprop[k] = {96'b0, 160'h00A5000000000000} + k;
    for (int k = 0; k < 4; k++) qprop.push_back({96'b0, 160'h00A5000000000000} + k);
  endfunction
endclass

class PlainArrDrv;                  // directly-declared packed array property (baseline)
  bit [2047:0][255:0] arr;
  function void fill(int n);
    bit [159:0] din;
    for (int k = 0; k < n; k++) begin
      din  = 160'h00A5000000000000 + k;
      arr[k] = din;                 // expected OK: handled by class_packed_elem_ref
    end
  endfunction
endclass

// __MWE_MODULE__

module tb_top;
  `SVTEST_INIT
  BigPkt       mod_pkt;             // module-level struct var (production chain)
  SmallPkt     mod_spkt;
  BigDrv       bd, bd2, bd3, bdn;
  SmallDrv     sd;
  WideDrv      wd;
  OuterDrv     od;
  UPktDrv      ud;
  UADrv        ua;
  UDynDrv      udd;
  PlainArrDrv  pd;
  logic        clk;
  logic        nba_go;
  logic        nba2_go;

  function automatic bit [255:0] exp_word(input int p);
    return {96'b0, 160'h00A5000000000000} + p;
  endfunction

  // element probe value with a distinctive top byte (bits 255:248 = 0xDE)
  // and low word (bits 15:0 = 0xBEEF); built by concatenation so the bit
  // positions cannot be miscounted
  localparam bit [255:0] ELEM1    = {8'hDE, 232'b0, 16'hBEEF};
  localparam bit [255:0] ELEM1_AB = {8'hAB, 232'b0, 16'hBEEF};


  always #1 clk = ~clk;

  // NBA cousin: element store from a clocked always block
  always @(posedge clk) if (nba_go)
    bdn.pkt.mem[0] <= 256'h0000000000000000000000000000000000000000000000000000FACE;

  // NBA cousin on the UNPACKED-array member shape (K7)
  always @(posedge clk) if (nba2_go)
    ua.pkt.mem[0] <= 256'h0000000000000000000000000000000000000000000000000000FACE;

  initial begin
    clk    = 0;
    nba_go = 0;
    nba2_go = 0;

    // ===== A: local struct variable, element store -> read (baseline) =====
    begin
      BigPkt a;
      a.mem[0] = exp_word(0);
      a.mem[1] = exp_word(1);
      a.mem[2] = exp_word(2);
      a.mem[3] = exp_word(3);
      for (int p = 0; p < 4; p++)
        `SVTEST_CHECK(a.mem[p] === exp_word(p),
          $sformatf("A: local elem p=%0d exp=%h got=%h", p, exp_word(p), a.mem[p]))
      $display("check A (local struct elem r/w) done, failures so far: %0d", failures);
    end

    // ===== E: plain packed-array class property (baseline contrast) =====
    begin
      bit [255:0] got;
      pd = new();
      pd.fill(4);
      for (int p = 0; p < 4; p++) begin
        got = pd.arr[p];
        `SVTEST_CHECK(got === exp_word(p),
          $sformatf("E: plain array elem p=%0d exp=%h got=%h", p, exp_word(p), got))
      end
      $display("check E (plain array property elem r/w) done, failures so far: %0d", failures);
    end

    // ===== B1: element store inside method -> copy-out verify (PRIMARY) =====
    // The whole property is copied to a local and read element-wise there, so
    // the check isolates the STORE from the (also broken) direct element read.
    begin
      BigPkt t;
      bd = new();
      bd.fill(4);
      t = bd.pkt;
      for (int p = 0; p < 4; p++)
        `SVTEST_CHECK(t.mem[p] === exp_word(p),
          $sformatf("B1: method elem store p=%0d exp=%h got=%h (copy-out)", p, exp_word(p), t.mem[p]))
      $display("check B1 (method elem store, copy-out verify) done, failures so far: %0d", failures);
    end

    // ===== B3: same with a small [3:0][255:0] member (size dependence) =====
    begin
      SmallPkt t;
      sd = new();
      sd.fill(4);
      t = sd.pkt;
      for (int p = 0; p < 4; p++)
        `SVTEST_CHECK(t.mem[p] === exp_word(p),
          $sformatf("B3: small struct elem store p=%0d exp=%h got=%h (copy-out)", p, exp_word(p), t.mem[p]))
      $display("check B3 (small struct elem store) done, failures so far: %0d", failures);
    end

    // ===== F2: whole-member store, method + module scope (store family) =====
    begin
      bit [2047:0][255:0] a1, a2;
      BigPkt t;
      for (int p = 0; p < 4; p++) begin
        a1[p] = exp_word(p);
        a2[p] = exp_word(3 - p);
      end
      bd.set_mem_arr(a1);           // method scope: `pkt.mem = a;`
      bd.pkt.mem = a2;              // module scope: `obj.prop.mem = a;`
      t = bd.pkt;
      for (int p = 0; p < 4; p++)
        `SVTEST_CHECK(t.mem[p] === exp_word(3 - p),
          $sformatf("F2: whole-member store p=%0d exp=%h got=%h (copy-out)", p, exp_word(3 - p), t.mem[p]))
      $display("check F2 (whole-member store) done, failures so far: %0d", failures);
    end

    // ===== W: wide SCALAR member vs array member, same struct (width vs shape) =====
    begin
      WidePkt t;
      wd = new();
      wd.pkt.id   = 64'h1122334455667788;
      wd.pkt.wide = {4{512'h0000111100002222000033330000444400005555000066660000A5A5}}; // 2048-bit
      wd.pkt.mem[1] = exp_word(1);
      t = wd.pkt;
      `SVTEST_CHECK(t.id === 64'h1122334455667788,
        $sformatf("W: scalar member id exp=%h got=%h", 64'h1122334455667788, t.id))
      `SVTEST_CHECK(t.wide === {4{512'h0000111100002222000033330000444400005555000066660000A5A5}},
        $sformatf("W: wide scalar member store got=%h", t.wide[2047:1792]))
      `SVTEST_CHECK(t.mem[1] === exp_word(1),
        $sformatf("W: array member elem store p=1 exp=%h got=%h (copy-out)", exp_word(1), t.mem[1]))
      $display("check W (wide scalar vs array member) done, failures so far: %0d", failures);
    end

    // __MWE_CHECKS2__

    // ===== K1: whole-struct property store (also the data injector for R*) =====
    begin
      BigPkt src, t;
      bd2 = new();
      src.id   = 64'h00D15EA5E5C0FFEE;
      src.len  = 32'd4;
      src.attr = 256'h7126330007071000;
      src.mem[0] = 256'h3;
      src.mem[1] = ELEM1;
      src.mem[2] = 256'h1;
      src.mem[3] = 256'h2;
      bd2.load(src);                // method scope: `pkt = p;`
      t = bd2.pkt;
      `SVTEST_CHECK(t.id === 64'h00D15EA5E5C0FFEE && t.len === 32'd4 && t.attr === 256'h7126330007071000,
        $sformatf("K1: whole-struct store head fields id=%h len=%0d attr=%h", t.id, t.len, t.attr))
      `SVTEST_CHECK(t.mem[0] === 256'h3 && t.mem[1] === ELEM1 && t.mem[2] === 256'h1 && t.mem[3] === 256'h2,
        $sformatf("K1: whole-struct store array member got[0]=%h [1]=%h", t.mem[0], t.mem[1]))
      $display("check K1 (whole-struct property store) done, failures so far: %0d", failures);
    end

    // ===== R: direct element READ at module scope (flattened shape) =====
    // Data is present (injected via the whole-struct store above). A correct
    // read returns the full 256-bit element; the bit-select fallthrough
    // returns a single bit of the member slice.
    begin
      bit [255:0] r0, r1;
      r0 = bd2.pkt.mem[0];
      r1 = bd2.pkt.mem[1];
      `SVTEST_CHECK(r0 === 256'h3, $sformatf("R: module-scope elem read p=0 exp=3 got=%h", r0))
      `SVTEST_CHECK(r1 === ELEM1, $sformatf("R: module-scope elem read p=1 exp=%h got=%h", ELEM1, r1))
      $display("check R (module-scope elem read) done, failures so far: %0d", failures);
    end

    // ===== R2: element read inside a method (`peek`) =====
    begin
      bit [255:0] r1;
      r1 = bd2.peek(1);
      `SVTEST_CHECK(r1 === ELEM1, $sformatf("R2: method elem read p=1 exp=%h got=%h", ELEM1, r1))
      $display("check R2 (method elem read) done, failures so far: %0d", failures);
    end

    // ===== R3: part-select reads `prop.mem[1][255:248]` and [15:0] =====
    begin
      bit [7:0] b;
      bit [15:0] w;
      b = bd2.pkt.mem[1][255:248];
      `SVTEST_CHECK(b === 8'hDE, $sformatf("R3: part-select read mem[1][255:248] exp=DE got=%h", b))
      w = bd2.pkt.mem[1][15:0];
      `SVTEST_CHECK(w === 16'hBEEF, $sformatf("R3b: part-select read mem[1][15:0] exp=BEEF got=%h", w))
      $display("check R3 (part-select reads) done, failures so far: %0d", failures);
    end

    // ===== K3w: part-select WRITE `prop.mem[1][255:248] = 8'hAB` =====
    begin
      BigPkt t;
      bit [255:0] exp1;
      bd3 = new();
      bd3.load(bd2.pkt);
      bd3.pkt.mem[1][255:248] = 8'hAB;
      t = bd3.pkt;
      exp1 = ELEM1_AB;
      `SVTEST_CHECK(t.mem[1] === exp1,
        $sformatf("K3w: part-select write mem[1][255:248] exp=%h got=%h", exp1, t.mem[1]))
      $display("check K3w (part-select write) done, failures so far: %0d", failures);
    end

    // ===== B2m: element store at MODULE scope (flattened `obj.prop.mem[i]=v`) =====
    begin
      BigPkt t;
      bd3.pkt.mem[2] = exp_word(2);
      t = bd3.pkt;
      `SVTEST_CHECK(t.mem[2] === exp_word(2),
        $sformatf("B2m: module-scope elem store p=2 exp=%h got=%h (copy-out)", exp_word(2), t.mem[2]))
      $display("check B2m (module-scope elem store) done, failures so far: %0d", failures);
    end

    // __MWE_CHECKS3__

    // ===== K4: NBA element store from a clocked always block =====
    begin
      BigPkt t;
      bdn = new();
      nba_go = 1;
      #5;                              // let a posedge fire and the NBA land
      nba_go = 0;
      t = bdn.pkt;
      `SVTEST_CHECK(t.mem[0] === 256'h0000000000000000000000000000000000000000000000000000FACE,
        $sformatf("K4: NBA elem store mem[0] exp=FACE got=%h (copy-out)", t.mem[0]))
      $display("check K4 (NBA elem store from always) done, failures so far: %0d", failures);
    end

    // ===== K5: unpacked-struct property with packed-array member (contrast) =====
    begin
      bit [255:0] got;
      ud = new();
      ud.fill(4);
      for (int p = 0; p < 4; p++) begin
        got = ud.pkt.mem[p];
        `SVTEST_CHECK(got === exp_word(p),
          $sformatf("K5: unpacked-struct elem p=%0d exp=%h got=%h", p, exp_word(p), got))
      end
      $display("check K5 (unpacked struct property elem r/w) done, failures so far: %0d", failures);
    end

    // ===== K6: element store at struct depth 2 (`prop.inner.mem[i]`) =====
    begin
      OuterT ot;
      InnerT ii;
      od = new();
      od.fill_inner(4);
      ot = od.pkt;
      for (int p = 0; p < 4; p++)
        `SVTEST_CHECK(ot.inner.mem[p] === exp_word(p),
          $sformatf("K6: depth-2 elem store p=%0d exp=%h got=%h (copy-out)", p, exp_word(p), ot.inner.mem[p]))
      ii.tag = 32'h0BAD0BAD;
      for (int p = 0; p < 4; p++) ii.mem[p] = exp_word(3 - p);
      od.set_inner(ii);               // member-of-member WHOLE store (contrast)
      ot = od.pkt;
      `SVTEST_CHECK(ot.inner.tag === 32'h0BAD0BAD,
        $sformatf("K6: member-of-member store tag exp=%h got=%h", 32'h0BAD0BAD, ot.inner.tag))
      for (int p = 0; p < 4; p++)
        `SVTEST_CHECK(ot.inner.mem[p] === exp_word(3 - p),
          $sformatf("K6: member-of-member whole store p=%0d exp=%h got=%h", p, exp_word(3 - p), ot.inner.mem[p]))
      $display("check K6 (nested struct member stores) done, failures so far: %0d", failures);
    end

    // ===== K7: UNPACKED-array member of an unpacked-struct property =====
    // Boundary first (element r/w + by-value return work in xezim — the
    // per-element machinery exists for unpacked dimensions), then the three
    // cousins that still break on this shape.
    begin
      UPktA t2, t3;
      bit [255:0] got;
      ua = new();
      ua.fill(4);

      // K7a: element read at MODULE scope and inside a METHOD (boundary)
      for (int p = 0; p < 4; p++) begin
        got = ua.pkt.mem[p];
        `SVTEST_CHECK(got === exp_word(p),
          $sformatf("K7a: elem read p=%0d exp=%h got=%h", p, exp_word(p), got))
      end
      for (int p = 0; p < 4; p++) begin
        got = ua.rd(p);
        `SVTEST_CHECK(got === exp_word(p),
          $sformatf("K7a: method elem read p=%0d exp=%h got=%h", p, exp_word(p), got))
      end

      // K7r: by-value return of the unpacked struct (boundary)
      t3 = ua.get_pkt();
      for (int p = 0; p < 4; p++)
        `SVTEST_CHECK(t3.mem[p] === exp_word(p),
          $sformatf("K7r: by-value return p=%0d exp=%h got=%h", p, exp_word(p), t3.mem[p]))
      `SVTEST_CHECK(t3.id === 64'h1122334455667788,
        $sformatf("K7r: by-value return id exp=%h got=%h", 64'h1122334455667788, t3.id))

      // K7b: whole-property copy-out to a FRESH local (cousin: elements lost)
      t2 = ua.pkt;
      for (int p = 0; p < 4; p++)
        `SVTEST_CHECK(t2.mem[p] === exp_word(p),
          $sformatf("K7b: copy-out elem p=%0d exp=%h got=%h", p, exp_word(p), t2.mem[p]))
      `SVTEST_CHECK(t2.id === 64'h1122334455667788,
        $sformatf("K7b: copy-out id exp=%h got=%h", 64'h1122334455667788, t2.id))

      // K7a2: element store at MODULE scope (boundary; new value so a
      // dropped store cannot hide behind the value fill() already wrote)
      ua.pkt.mem[2] = ELEM1;
      `SVTEST_CHECK(ua.pkt.mem[2] === ELEM1,
        $sformatf("K7a2: module elem store exp=%h got=%h", ELEM1, ua.pkt.mem[2]))

      // K7p: part-select store into an element, then part-select read (cousin)
      ua.pkt.mem[1][255:248] = 8'hAB;
      `SVTEST_CHECK(ua.pkt.mem[1] === {8'hAB, exp_word(1)[247:0]},
        $sformatf("K7p: part-select store exp=%h got=%h", {8'hAB, exp_word(1)[247:0]}, ua.pkt.mem[1]))
      `SVTEST_CHECK(ua.pkt.mem[1][255:248] === 8'hAB,
        $sformatf("K7p: part-select read exp=AB got=%h", ua.pkt.mem[1][255:248]))

      // K7n: NBA element store from a clocked always (cousin: clobbers to 0)
      nba2_go = 1;
      #5;
      nba2_go = 0;
      `SVTEST_CHECK(ua.pkt.mem[0] === 256'h0000000000000000000000000000000000000000000000000000FACE,
        $sformatf("K7n: NBA elem store exp=FACE got=%h", ua.pkt.mem[0]))
      $display("check K7 (unpacked-array member of unpacked struct) done, failures so far: %0d", failures);
    end

    // ===== K8: dynamic/multi-dim container members of an unpacked-struct =====
    // ===== property: dyn [], queue [$], assoc [int], multi-dim [2][2]  =====
    begin
      udd = new();
      udd.fill();

      // K8d: dynamic-array member element reads (stores happened in fill)
      for (int p = 0; p < 4; p++)
        `SVTEST_CHECK(udd.pkt.dmem[p] === exp_word(p),
          $sformatf("K8d: dyn elem read p=%0d exp=%h got=%h", p, exp_word(p), udd.pkt.dmem[p]))
      // K8d2: dynamic-array member element store at MODULE scope (new value)
      udd.pkt.dmem[2] = ELEM1;
      `SVTEST_CHECK(udd.pkt.dmem[2] === ELEM1,
        $sformatf("K8d2: dyn elem store exp=%h got=%h", ELEM1, udd.pkt.dmem[2]))
      // K8q: queue member size + element reads
      `SVTEST_CHECK(udd.pkt.qmem.size() == 4,
        $sformatf("K8q: queue size exp=4 got=%0d", udd.pkt.qmem.size()))
      for (int p = 0; p < 2; p++)
        `SVTEST_CHECK(udd.pkt.qmem[p] === exp_word(p),
          $sformatf("K8q: queue elem read p=%0d exp=%h got=%h", p, exp_word(p), udd.pkt.qmem[p]))
      // K8q2: queue member push_back at MODULE scope
      udd.pkt.qmem.push_back(ELEM1);
      `SVTEST_CHECK(udd.pkt.qmem.size() == 5 && udd.pkt.qmem[4] === ELEM1,
        $sformatf("K8q2: queue push size=%0d elem=%h", udd.pkt.qmem.size(), udd.pkt.qmem[4]))
      // K8a: associative-array member element reads
      for (int p = 0; p < 2; p++)
        `SVTEST_CHECK(udd.pkt.amem[p] === exp_word(p),
          $sformatf("K8a: assoc elem read p=%0d exp=%h got=%h", p, exp_word(p), udd.pkt.amem[p]))
      // K8m: multi-dim unpacked member element reads
      for (int i = 0; i < 2; i++)
        for (int j = 0; j < 2; j++)
          `SVTEST_CHECK(udd.pkt.mmem[i][j] === exp_word(i*2+j),
            $sformatf("K8m: multidim elem read i=%0d j=%0d exp=%h got=%h", i, j, exp_word(i*2+j), udd.pkt.mmem[i][j]))
      // K8s: string member (boundary — expected OK in xezim)
      `SVTEST_CHECK(udd.pkt.sname == "HELLO",
        $sformatf("K8s: string member exp=HELLO got=%s", udd.pkt.sname))
      // K8p: DIRECT container class properties (boundary — expected OK)
      for (int p = 0; p < 2; p++)
        `SVTEST_CHECK(udd.dprop[p] === exp_word(p),
          $sformatf("K8p: direct dyn read p=%0d exp=%h got=%h", p, exp_word(p), udd.dprop[p]))
      `SVTEST_CHECK(udd.qprop[1] === exp_word(1),
        $sformatf("K8p: direct queue read exp=%h got=%h", exp_word(1), udd.qprop[1]))
      $display("check K8 (dynamic/multidim container members) done, failures so far: %0d", failures);
    end

    // ===== C: full production chain (method element stores -> by-value =====
    // ===== return -> module-level struct -> indexed reads) ==================
    begin
      bit [255:0] got;
      bd = new();                     // fresh instance
      mod_pkt = bd.prepare();
      `SVTEST_CHECK(mod_pkt.id === 64'h00D15EA5E5C0FFEE,
        $sformatf("C: chain head id exp=%h got=%h", 64'h00D15EA5E5C0FFEE, mod_pkt.id))
      `SVTEST_CHECK(mod_pkt.len === 32'd4, $sformatf("C: chain head len exp=4 got=%0d", mod_pkt.len))
      `SVTEST_CHECK(mod_pkt.attr === 256'h7126330007071000,
        $sformatf("C: chain head attr exp=%h got=%h", 256'h7126330007071000, mod_pkt.attr))
      for (int p = 0; p < 4; p++) begin
        got = mod_pkt.mem[p];
        `SVTEST_CHECK(got === exp_word(p),
          $sformatf("C: chain array p=%0d exp=%h got=%h", p, exp_word(p), got))
      end
      $display("check C (full chain) done, failures so far: %0d", failures);
    end

    // ===== D: control — full chain with a small array member =====
    begin
      bit [255:0] got;
      sd = new();
      mod_spkt = sd.prepare();
      `SVTEST_CHECK(mod_spkt.id === 64'h00D15EA5E5C0FFEE,
        $sformatf("D: small chain head id exp=%h got=%h", 64'h00D15EA5E5C0FFEE, mod_spkt.id))
      for (int p = 0; p < 4; p++) begin
        got = mod_spkt.mem[p];
        `SVTEST_CHECK(got === exp_word(p),
          $sformatf("D: small chain array p=%0d exp=%h got=%h", p, exp_word(p), got))
      end
      $display("check D (small struct full chain) done, failures so far: %0d", failures);
    end

    // ===== verdict =====
    #1;
    `SVTEST_PASSFAIL
    $finish;
  end
endmodule