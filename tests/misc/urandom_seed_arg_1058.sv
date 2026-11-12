// ---------------------------------------------------------------------------
// BUG M  $urandom(seed) WRITES THE GENERATED VALUE BACK INTO THE CALLER'S
//        SEED ARGUMENT.
//
//   The seed argument of $urandom is declared input-only:
//
//       function int unsigned $urandom (input int unsigned seed);   // 20.11
//
//   so the call may read the argument but must never modify it.  (Compare
//   $random, whose seed really is an inout and therefore MAY be updated --
//   that write-back is exercised as a control in check E01 so this file
//   cannot be dismissed as "all randomness differs".)
//
//   Here the seeded call overwrites the caller's variable with the value it
//   just returned:
//
//       s = 12345;  void'($urandom(s));   // s is no longer 12345
//
//   The damage is not limited to the one variable.  Every shape of storage is
//   hit -- local, formal, class property, array/queue/associative-array
//   element, struct member, enum, hierarchical reference, `ref` formal -- and
//   so is every execution context (loops, case, ternary, fork/join, an
//   assertion predicate).  A testbench that keeps one seed variable and hands
//   it to more than one consumer therefore silently feeds the second
//   consumer a different seed, which is what breaks the design-level
//   reproduction in section R.
//
// BUG N  $urandom(seed) DOES NOT RESEED THE GENERATOR THAT THE UNSCEEDED
//        CALLS DRAW FROM.
//
//   The seed argument is supposed to seed the generator; a call with no
//   argument continues from the seed in effect.  Two calls with the same seed
//   must therefore be followed by the SAME stream of unseeded $urandom and
//   $urandom_range draws.  Here the seeded value is consumed by the call
//   itself and the shared generator is left where it was, so the streams
//   after two identical seeds diverge (D02-D05).
//
// ---------------------------------------------------------------------------
// Every check matches the reference simulator except E11, which depends on
//   the sign of one random value (§18.13.1 leaves the sequence to the tool).
//
// No check compares a random VALUE across the two simulators -- the two
// tools use different generators and the standard does not fix the sequence.
// Every check either (a) looks at the seed ARGUMENT after the call, or
// (b) compares two streams produced inside the same run.
//
// Check-ID map (every row is greppable by its ID):
//   R01-R06    design-shaped reproduction: the testbench seed path
//   A01-A17    bug M core: the seed argument is input-only
//   B01-B16    bug M-wide: every STORAGE SHAPE of the seed argument
//   C01-C12    bug M-wide: every EXECUTION CONTEXT of the same call
//   D01-D05    bug N: the seed argument must reseed the shared generator
//   E01-E13    controls: the write-backs and behaviours that must keep working
// ---------------------------------------------------------------------------

`timescale 1ns / 1ps

`ifndef SVTEST_DEFS_SVH
`define SVTEST_DEFS_SVH
`define SVTEST_INIT \
  int failures = 0;

`define SVTEST_CHECK(expr, msg) \
  if (!(expr)) begin \
    failures++; \
    $display("FAIL: %s", msg); \
  end

`define SVTEST_PASSFAIL \
  if (failures == 0) begin \
    $display("TEST_PASS"); \
  end else begin \
    $display("TEST_FAIL count=%0d", failures); \
    $fatal(1); \
  end
`endif

//------------------------------------------------------------------------------
// A tiny store model, so section R can show the bug at DESIGN level: the
// scoreboard compares the store contents against a model driven from the same
// seed variable the drivers use.
//------------------------------------------------------------------------------
module store_model;
  int unsigned mem   [0:7];
  int unsigned wcnt;

  task automatic write(input int unsigned a, input int unsigned d);
    mem[a % 8] = d;
    wcnt++;
  endtask

  function automatic int unsigned hash();
    hash = 0;
    for (int i = 0; i < 8; i++) hash = hash * 31 + mem[i];
  endfunction

  task automatic clear();
    for (int i = 0; i < 8; i++) mem[i] = 0;
    wcnt = 0;
  endtask
endmodule

//------------------------------------------------------------------------------
package seed_pkg;
  typedef struct packed {
    int unsigned keep;      // must never move
    int unsigned seed;      // the $urandom argument
  } sc_t;
  typedef enum logic [7:0] { E_A = 8'h11, E_B = 8'h22 } e8_t;
endpackage

module urandom_seed_arg;
  import seed_pkg::*;

  `SVTEST_INIT

  // 4-state-safe value checker: `===` so an expected 'x compares exactly.
  task automatic chkv(input string tag, input int exp, input int got);
    if (exp === got)
      $display("%-58s exp=%0d got=%0d  MATCH", tag, exp, got);
    else begin
      $display("%-58s exp=%0d got=%0d  *** MISMATCH ***", tag, exp, got);
      failures++;
    end
  endtask

  task automatic chkb(input string tag, input bit exp, input bit got);
    if (exp === got)
      $display("%-58s exp=%0d got=%0d  MATCH", tag, exp, got);
    else begin
      $display("%-58s exp=%0d got=%0d  *** MISMATCH ***", tag, exp, got);
      failures++;
    end
  endtask

  store_model dut_store ();                    // the design-level subject

  // ---- shared seed variable: ONE variable, several consumers (section R) ----
  int unsigned live_seed;

  // ---- storage shapes (section B) -------------------------------------------
  wire [31:0]   net_seed;                      // B01 net (control)
  assign        net_seed = 32'hABCD;
  bit  [3:0][31:0] parr;                       // B02 packed-array element
  int           uarr [0:3];                    // B03 unpacked-array element
  int           aarr [string];                 // B04 associative-array element
  int           darr [];                       // B05 dynamic-array element
  int           que  [$];                      // B06 queue element
  e8_t          en;                            // B07 enum variable
  struct { int unsigned a; int unsigned b; } ust;   // B08 unpacked struct
  sc_t          sc;                            // B09 packed struct
  int           refo;                          // B14 ref formal
  int           task_formal_rpt;               // B15 task input formal

  leaf_mod  leaf ();                           // B13: owns seed_v

  // ---- section R: two drivers that share ONE seed variable ------------------
  task automatic drive4();
    int unsigned r;
    for (int i = 0; i < 4; i++) begin
      r = $urandom(live_seed);                 // the LRM says this only READS
      dut_store.write(r % 8, r);
    end
  endtask

  function automatic int unsigned model_hash(input int unsigned frozen);
    int unsigned r, h;
    int unsigned m [0:7];
    for (int i = 0; i < 8; i++) m[i] = 0;
    for (int i = 0; i < 4; i++) begin
      r = $urandom(frozen);
      m[r % 8] = r;
    end
    h = 0;
    for (int i = 0; i < 8; i++) h = h * 31 + m[i];
    model_hash = h;
  endfunction

  function automatic int unsigned mem_hash();
    int unsigned h;
    h = 0;
    for (int i = 0; i < 8; i++) h = h * 31 + dut_store.mem[i];
    mem_hash = h;
  endfunction

  // ---- section B/C helpers --------------------------------------------------
  function automatic int f_seed(input int s);
    int r;
    r = $urandom(s);
    f_seed = s;                                // report the FORMAL after the call
  endfunction

  function automatic int f_seed_void(input int s);
    void'($urandom(s));
    f_seed_void = s;
  endfunction

  function automatic int f_clog2(input int n);
    int r;  r = $clog2(n);      f_clog2 = n;
  endfunction

  function automatic int f_countones(input int n);
    int r;  r = $countones(n);  f_countones = n;
  endfunction

  function automatic int f_signed(input int n);
    int r;  r = $signed(n);     f_signed = n;
  endfunction

  function automatic int f_sformatf(input int n);
    string s;  s = $sformatf("%0d", n);  f_sformatf = n;
  endfunction

  function automatic bit always_true(input int x);
    always_true = 1'b1;
  endfunction

  task automatic poke_ref(ref int x);
    void'($urandom(x));
  endtask

  task automatic poke_task(input int x);
    void'($urandom(x));
    task_formal_rpt = x;                       // report the FORMAL after the call
  endtask

  class agent;
    int prop;
    function int poke();
      int r;
      r = $urandom(prop);
      poke = prop;
    endfunction
    function int local_poke();
      int loc;
      loc = 32'h4711;
      void'($urandom(loc));
      local_poke = loc;
    endfunction
  endclass

  // ---- section D helpers: the stream that follows a reseed ------------------
  function automatic int unsigned seeded_draw(input int unsigned s);
    seeded_draw = $urandom(s);
  endfunction

  function automatic int unsigned stream_after_seed(input int unsigned s);
    int unsigned acc, b;
    void'($urandom(s));                        // reseed
    acc = 0;
    for (int i = 0; i < 4; i++) begin          // then draw four bytes
      b = $urandom_range(0, 255) & 8'hFF;
      acc = (acc << 8) | b;
    end
    stream_after_seed = acc;
  endfunction


  agent     ag;
  int       loop_while, loop_repeat;
  int       casev, ternv, forkv, dispv, assertv;
  int       ctrl_random, ctrl_sscanf, ctrl_plus;
  int       in_range;
  int       task_local_rpt;
  int       keep_v;

  function automatic int f_local();
    int loc;
    loc = 32'h2468;
    void'($urandom(loc));
    f_local = loc;
  endfunction

  task automatic t_local();
    int loc;
    loc = 32'h1357;
    void'($urandom(loc));
    task_local_rpt = loc;
  endtask


  function automatic int unsigned urand_stream(input int unsigned s);
    int unsigned acc;
    void'($urandom(s));                        // reseed
    acc = 0;
    for (int i = 0; i < 2; i++)                // then two unseeded draws
      acc = (acc << 16) | ($urandom() & 16'hFFFF);
    urand_stream = acc;
  endfunction

  // ==========================================================================
  initial begin
    int unsigned SEED;
    string       banner;
    int          s, r, hi, lo, x, trips, guard;

    ag = new;
    #1;                                          // let the continuous assign settle

    // ---------------------------------------------------------------- R -----
    // The testbench seed path, in the shape a real testbench writes it.
    if (!$value$plusargs("SEED=%d", SEED) &&
        !$value$plusargs("SVSEED=%d", SEED))
      SEED = 32'hC0FFEE01;
    $display("--- R: testbench seed path ---");
    $display("R00 info: seed as read from the command line = %0d", SEED);
    void'($urandom(SEED));                     // the one line the testbench writes
    chkv("R01 seed survives the seeding call", 12345, SEED);
    chkb("R02 log banner still carries the seed read",
         1'b1, $sformatf("SEED=%0d", SEED) == "SEED=12345");
    dut_store.clear();
    live_seed = SEED;
    drive4();                                  // driver A
    drive4();                                  // driver B, same seed variable
    chkb("R03 both drivers issued 4 writes", 1'b1, dut_store.wcnt == 8);
    chkv("R04 store contents match the frozen-seed model",
         model_hash(SEED), mem_hash());
    void'($urandom(SEED));                     // closing banner re-seeds
    chkv("R05 seed still intact for the closing banner", 12345, SEED);
    dut_store.clear();
    live_seed = SEED;                          // re-armed from the seed variable
    drive4();
    chkv("R06 re-armed driver replays the model", model_hash(SEED), mem_hash());

    // ---------------------------------------------------------------- A -----
    $display("--- A: the seed argument is input-only ---");
    s = 12345; void'($urandom(s));
    chkv("A01 void'($urandom(s)) leaves s", 12345, s);
    s = 12345; r = $urandom(s);
    chkv("A02 r=$urandom(s) leaves s", 12345, s);
    $display("A03 info: s=%0d r=%0d  (s==r proves the value was written back)", s, r);
    s = 999; void'($urandom());
    chkv("A04 $urandom() with no argument leaves s", 999, s);
    s = 7; void'($urandom(s + 1));
    chkv("A05 an expression argument is not a write target", 7, s);
    s = 0; void'($urandom(s));
    chkv("A06 a zero seed leaves s", 0, s);
    hi = 100; lo = 3; void'($urandom_range(hi, lo));
    chkv("A07 $urandom_range(hi,lo) leaves hi", 100, hi);
    chkv("A08 $urandom_range(hi,lo) leaves lo", 3, lo);
    x = 50; void'($urandom_range(x));
    chkv("A09 $urandom_range(x) leaves x", 50, x);
    s = 12345; void'($random(s));
    $display("A10 info: $random(s) after the call = %0d (an inout seed may legally change)", s);
    s = 4242; r = seeded_draw(s);
    chkv("A11 same seed -> same returned value", seeded_draw(4242), r);
    chkv("A12 same seed -> the seed variable is untouched", 4242, s);
    s = 8888; void'($urandom($urandom(s)));
    chkv("A13 nested $urandom($urandom(s)) leaves s", 8888, s);
    chkv("A14 $clog2(n) leaves n", 64, f_clog2(64));
    chkv("A15 $countones(n) leaves n", 255, f_countones(255));
    chkv("A16 $signed(n) leaves n", 12, f_signed(12));
    chkv("A17 $sformatf with a format argument leaves n", 34, f_sformatf(34));

    // ---------------------------------------------------------------- B -----
    // Every shape of storage the seed argument can have.
    $display("--- B: storage shapes of the seed argument ---");
    chkv("B01 a net argument keeps its driver value", 32'hABCD, net_seed);
    parr[0] = 32'h11; parr[1] = 32'h22; parr[2] = 32'h33; parr[3] = 32'h44;
    void'($urandom(parr[1]));
    chkv("B02 packed-array element parr[1] survives", 32'h22, parr[1]);
    chkv("B02b sibling parr[0] survives", 32'h11, parr[0]);
    chkv("B02c unrelated parr[2] survives", 32'h33, parr[2]);
    uarr[0] = 32'hAA; uarr[1] = 32'hBB;
    void'($urandom(uarr[0]));
    chkv("B03 unpacked-array element uarr[0] survives", 32'hAA, uarr[0]);
    aarr["k"] = 32'hCC;
    void'($urandom(aarr["k"]));
    chkv("B04 associative-array element survives", 32'hCC, aarr["k"]);
    darr = new [2]; darr[0] = 32'hDD; darr[1] = 32'hEE;
    void'($urandom(darr[0]));
    chkv("B05 dynamic-array element survives", 32'hDD, darr[0]);
    que = {32'h1234};
    void'($urandom(que[0]));
    chkv("B06 queue element survives", 32'h1234, que[0]);
    en = E_A;
    void'($urandom(en));
    chkv("B07 enum variable survives", 8'h11, en);
    ust.a = 32'h1234; ust.b = 32'h5678;
    void'($urandom(ust.a));
    chkv("B08 unpacked-struct member ust.a survives", 32'h1234, ust.a);
    chkv("B08b sibling ust.b survives", 32'h5678, ust.b);
    sc.keep = 32'h5A5A; sc.seed = 32'h777;
    void'($urandom(sc.seed));
    chkv("B09 packed-struct member sc.seed survives", 32'h777, sc.seed);
    chkv("B09b sibling sc.keep survives", 32'h5A5A, sc.keep);
    ag.prop = 4321;
    chkv("B10 class property survives a seeded call in a method", 4321, ag.poke());
    chkv("B11 function formal survives r=$urandom(formal)", 555, f_seed(555));
    chkv("B12 function formal survives void'($urandom(formal))", 556, f_seed_void(556));
    leaf.seed_v = 32'hEF;
    void'($urandom(leaf.seed_v));
    chkv("B13 hierarchical reference survives", 32'hEF, leaf.seed_v);
    refo = 32'hAB;
    poke_ref(refo);
    chkv("B14 ref formal survives", 32'hAB, refo);
    poke_task(32'hCD);
    chkv("B15 task input formal keeps its value", 32'hCD, task_formal_rpt);
    ag.prop = 8888;
    void'($urandom_range(ag.prop, 1));
    chkv("B16 $urandom_range leaves a class property alone", 8888, ag.prop);

    // ---------------------------------------------------------------- C -----
    // Every context the same call can sit in.
    $display("--- C: execution contexts of the same call ---");
    trips = 0;
    for (int i = 0; i < 3; i++) begin
      void'($urandom(i));
      trips++;
      if (trips > 20) break;                   // guard: a clobbered index never exits
    end
    chkv("C01 for-loop over a seeded index runs 3 times", 3, trips);
    loop_while = 0; guard = 0;
    while (loop_while < 3 && guard < 20) begin
      guard++;
      void'($urandom(loop_while));
      loop_while++;
    end
    chkv("C02 while-loop counter reaches 3", 3, loop_while);
    chkv("C02b while-loop guard is not exhausted", 3, guard);
    loop_repeat = 32'h99;
    repeat (2) void'($urandom(loop_repeat));
    chkv("C03 repeat-loop variable survives", 32'h99, loop_repeat);
    casev = 32'h88;
    case (1'b1)
      1'b1: void'($urandom(casev));
      default: ;
    endcase
    chkv("C04 case-arm variable survives", 32'h88, casev);
    ternv = 32'h77;
    r = 1'b1 ? $urandom(ternv) : 0;
    chkv("C05 ternary operand variable survives", 32'h77, ternv);
    forkv = 32'h55;
    fork
      void'($urandom(forkv));
    join
    chkv("C06 fork/join variable survives", 32'h55, forkv);
    dispv = 32'h66;
    $display("C07 info: value in a $display argument = %0d", $urandom(dispv));
    chkv("C07 $display argument variable survives", 32'h66, dispv);
    assertv = 32'h99;
    assert (always_true($urandom(assertv))) else $error("C08 unreachable");
    chkv("C08 assertion-predicate variable survives", 32'h99, assertv);
    dut_store.clear();
    for (int i = 0; i < 6; i++) begin
      live_seed = SEED;                        // re-armed every trip
      dut_store.write($urandom(live_seed) % 8, i);
    end
    chkv("C09 store saw one write per loop trip", 6, dut_store.wcnt);
    chkv("C10 function local survives a seeded call", 32'h2468, f_local());
    t_local();
    chkv("C11 task local survives a seeded call", 32'h1357, task_local_rpt);
    chkv("C12 class-method local survives a seeded call", 32'h4711, ag.local_poke());

    // ---------------------------------------------------------------- D -----
    // The seed argument is supposed to seed the generator the unseeded calls
    // draw from.  Only streams produced inside this one run are compared.
    $display("--- D: the seed argument must reseed the shared generator ---");
    chkv("D01 same seed -> same seeded draw",
         seeded_draw(9999), seeded_draw(9999));
    chkv("D02 same seed -> same $urandom_range stream",
         stream_after_seed(12345), stream_after_seed(12345));
    chkv("D03 same seed -> same $urandom stream",
         urand_stream(31337), urand_stream(31337));
    chkb("D04 different seeds -> different streams",
         1'b1, stream_after_seed(12345) != stream_after_seed(98765));
    s = 5555;
    r = stream_after_seed(s);
    chkv("D05 the caller's seed survives a reseed call", 5555, s);

    // ---------------------------------------------------------------- E -----
    // Controls: the write-backs and behaviours that must keep working.
    $display("--- E: controls ---");
    ctrl_random = 12345;
    void'($random(ctrl_random));               // $random's seed really is inout
    chkb("E01 $random(seed) DOES update its inout seed", 1'b1, ctrl_random != 12345);
    ctrl_sscanf = 0;
    void'($sscanf("42", "%d", ctrl_sscanf));
    chkv("E02 $sscanf writes its target", 42, ctrl_sscanf);
    ctrl_plus = 0;
    if (!$value$plusargs("SEED=%d", ctrl_plus)) ctrl_plus = 0;
    chkv("E03 $value$plusargs writes its target", 12345, ctrl_plus);
    r = $urandom(); x = $urandom();
    chkb("E04 two unseeded draws differ", 1'b1, r != x);
    in_range = 1;
    for (int i = 0; i < 20; i++) begin
      r = $urandom_range(0, 3);
      if (r < 0 || r > 3) in_range = 0;
    end
    chkv("E05 $urandom_range(0,3) stays in range", 1, in_range);
    in_range = 1;
    for (int i = 0; i < 20; i++) begin
      r = $urandom_range(3, 0);                // reversed bounds
      if (r < 0 || r > 3) in_range = 0;
    end
    chkv("E06 $urandom_range(3,0) stays in range", 1, in_range);
    in_range = 1;
    for (int i = 0; i < 20; i++) begin
      r = $urandom_range(7);                   // single bound
      if (r < 0 || r > 7) in_range = 0;
    end
    chkv("E07 $urandom_range(7) stays in range", 1, in_range);
    chkv("E08 $urandom_range(5,5) returns 5", 5, $urandom_range(5, 5));
    uarr[2] = 32'h24; uarr[3] = 32'h68;
    void'($urandom_range(uarr[2], uarr[3]));
    chkv("E09 $urandom_range leaves array elements alone", 32'h24, uarr[2]);
    dut_store.clear();
    dut_store.write(3, 32'hBEEF);
    chkv("E10 store model round-trip", 32'hBEEF, dut_store.mem[3]);
    s = 246; r = $urandom(s);
    chkb("E11 a seeded draw is still usable as a 32-bit value",
         1'b1, (r % 8) >= 0 && (r % 8) <= 7);
    keep_v = 32'hA5A5; s = 246;
    void'($urandom(s));
    chkv("E12 an unrelated variable is untouched by a seeded call", 32'hA5A5, keep_v);
    keep_v = 32'h5A5A; s = 1357;
    $display("E13 info: seeded draw printed from $display = %0d", $urandom(s));
    chkv("E13 $display of a seeded call leaves another variable alone",
         32'h5A5A, keep_v);

    $display("=== urandom_seed_arg: %0d MISMATCH(ES) ===", failures);
    `SVTEST_PASSFAIL
    $finish;
  end
endmodule

//------------------------------------------------------------------------------
module leaf_mod;
  int seed_v;                                  // B13 target
endmodule

