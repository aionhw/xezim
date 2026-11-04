// Multi-dimensional array handling in std::randomize() and class constraints.
//
// ND (2-D and 3-D) unpacked arrays are not modelled as randomize targets, on
// BOTH entry points:
//
//   (A) std::randomize(t, a) with an ND local -- ids  1..29
//       * static subroutine locals    -> the whole call bails, nothing is solved
//       * automatic locals            -> the array is SILENTLY DROPPED from the
//                                        solve ("scope solve: arrays []") and the
//                                        call fails
//       Elements come back as uninitialised garbage, not as defaults.
//
//   (B) class `rand int m[2][2]` + inline with{} -- ids 30..61, cell B9
//       randomize() returns 0 and leaves garbage, even though the class solver
//       is otherwise the reference implementation for every 1-D shape here.
//       Verified to fail identically in isolation, so it is not an ordering
//       artifact.
//
// ids 24..29 are a BEFORE/AFTER pair of the SAME 1-D call, which is a third
// symptom: an unsupported ND call corrupts solver state so that a LATER,
// unrelated, fully-supported std::randomize() also fails.  The identical 1-D
// shape passes when run alone and when run first (ids 24..26), and fails when
// run after the ND cells (ids 27..29).  ND support must make both green.
//
// Cell ORDER is load-bearing and deliberate: the class cells run first, then the
// ND cells, then the poisoning probe.  Measured on unpatched main, letting an ND
// cell run before the class cells makes ids 36/37/40 pass -- an unsupported ND
// call silently repairs the class-member write path.  That is state leaking in
// the other direction, not a fix, so the ordering here keeps every cell
// independent and stops this file masking a separate defect.  Read 36/37/40
// against a build that already restores class member arrays.
//
// Per-element bands plus a joint coupling to a scalar (a[0][0] == t - 90 with
// t == 100 forces a[0][0] == 10) avoid .sum() with a with-clause: xrun rejects
// 32'(item) on ND arrays because `item` is the sub-array, not an element.
//
// Expected: TEST_PASS  (xrun agrees on every check)
module tb;
   int errors, checks, cells;
   int mts;                        // module-scope scalar

   task automatic chk(input int id, input int got, input int exp);
      checks++;
      if (got !== exp) begin
         errors++;
         $display("CHECK_FAIL id=%0d got=%0d exp=%0d", id, got, exp);
      end
   endtask

   function automatic int inband(input int x);
      inband = (x >= 10 && x <= 40) ? 1 : 0;
   endfunction

   function automatic int inband3(input int x);
      inband3 = (x >= 1 && x <= 20) ? 1 : 0;
   endfunction

   function automatic int overlim(input int x);
      overlim = (x > 30) ? 1 : 0;
   endfunction

   // =====================================================================
   // (A) std::randomize() with ND targets   -- ids 1..29
   // =====================================================================

   // ---- 2-D, static function locals: whole call bails ----
   function void local_2d_static();
      int a [2][2]; int t; int rc;
      rc = std::randomize(t, a) with {
               t == 100; a[0][0] == t - 90;
               foreach (a[i]) foreach (a[i][j]) a[i][j] inside {[10:40]}; };
      chk(1, rc, 1); chk(2, t, 100); chk(3, a[0][0], 10);
      chk(4, inband(a[0][1]), 1); chk(5, inband(a[1][0]), 1);
      chk(6, inband(a[1][1]), 1);
      cells++;
   endfunction

   // ---- 2-D, automatic function locals: array silently dropped ----
   function automatic void local_2d_automatic();
      int a [2][2]; int t; int rc;
      rc = std::randomize(t, a) with {
               t == 100; a[0][0] == t - 90;
               foreach (a[i]) foreach (a[i][j]) a[i][j] inside {[10:40]}; };
      chk(7, rc, 1); chk(8, t, 100); chk(9, a[0][0], 10);
      chk(10, inband(a[0][1]), 1); chk(11, inband(a[1][0]), 1);
      chk(12, inband(a[1][1]), 1);
      cells++;
   endfunction

   // ---- 2-D array as the only target ----
   function void local_2d_alone();
      int a [2][2]; int rc;
      rc = std::randomize(a) with {
               a[0][0] == 10;
               foreach (a[i]) foreach (a[i][j]) a[i][j] inside {[10:40]}; };
      chk(13, rc, 1); chk(14, a[0][0], 10);
      chk(15, inband(a[0][1]), 1); chk(16, inband(a[1][1]), 1);
      cells++;
   endfunction

   // ---- module-scope scalar + local 2-D array ----
   function void module_scalar_local_2d();
      int a [2][2]; int rc;
      mts = 0;
      rc = std::randomize(mts, a) with {
               mts == 100; a[0][0] == mts - 90;
               foreach (a[i]) foreach (a[i][j]) a[i][j] inside {[10:40]}; };
      chk(17, rc, 1); chk(18, mts, 100); chk(19, a[0][0], 10);
      chk(20, inband(a[1][1]), 1);
      cells++;
   endfunction

   // ---- 3-D array ----
   function void local_3d();
      int a [2][2][2]; int rc;
      rc = std::randomize(a) with {
               a[0][0][0] == 5;
               foreach (a[i]) foreach (a[i][j]) foreach (a[i][j][k])
                  a[i][j][k] inside {[1:20]}; };
      chk(21, rc, 1); chk(22, a[0][0][0], 5);
      chk(23, inband3(a[1][1][1]), 1);
      cells++;
   endfunction

   // ---- 1-D control, run BEFORE the ND cells: must pass ----
   function automatic void local_1d_before_nd();
      int a [4]; int t; int rc; int s;
      rc = std::randomize(t, a) with {
               t == 100; a.sum() with (32'(item)) == 100;
               foreach (a[i]) a[i] inside {[20:60]}; };
      s = a.sum() with (32'(item));
      chk(24, rc, 1); chk(25, t, 100); chk(26, s, 100);
      cells++;
   endfunction

   // ---- the SAME 1-D control, run AFTER the ND cells: state-poisoning probe.
   //      Identical body to local_1d_before_nd; passes alone and first, fails
   //      after an ND call. ----
   function automatic void local_1d_after_nd();
      int a [4]; int t; int rc; int s;
      rc = std::randomize(t, a) with {
               t == 100; a.sum() with (32'(item)) == 100;
               foreach (a[i]) a[i] inside {[20:60]}; };
      s = a.sum() with (32'(item));
      chk(27, rc, 1); chk(28, t, 100); chk(29, s, 100);
      cells++;
   endfunction

   // =====================================================================
   // (B) class `rand` equivalents   -- ids 30..61
   // The class solver is the reference implementation: it already handles the
   // joint CSP solve, packed-struct members and failure rollback for 1-D
   // shapes, so only cell B9 (ND) may fail.  One class per shape: xezim
   // evaluates ALL class constraints, so constraint_mode() inside with{...}
   // cannot select one at a time.
   // =====================================================================

   typedef struct packed { bit [7:0] sa; bit [7:0] sb; } S;

   class C1_sum;      rand int t; rand int a [4];
      constraint c { t == 100; a.sum() with (32'(item)) == 100;
                     foreach (a[i]) a[i] inside {[20:60]}; } endclass
   class C2_struct;   rand S st;
      constraint c { st.sa == 15; st.sb == 4; } endclass
   class C3_rollback; rand int t; rand int a [4];
      constraint c { a.sum() with (32'(item)) == 999999;
                     foreach (a[i]) a[i] inside {[20:60]}; }
      // Seed from inside the class: writing `k3.a[i] = 7` from module scope is
      // itself broken on unpatched main and would confound this rollback check.
      function void seed();
         t = 4242;
         foreach (a[i]) a[i] = 7;
      endfunction
   endclass
   class C4_two;      rand int a [4]; rand int b [4];
      constraint c { a.sum() with (32'(item)) == 100; b.sum() with (32'(item)) == 100;
                     foreach (a[i]) a[i] inside {[20:60]};
                     foreach (b[i]) b[i] inside {[20:60]}; } endclass
   class C5_exact;    rand int a [4];
      constraint c { a[0] == 25; a[1] == 25; a[2] == 25; a[3] == 25; } endclass
   class C6_inline;   rand int a [4]; endclass
   class C7_randc;    randc int c; endclass
   class C8_dyn;      rand int d []; endclass
   class C9_2d;       rand int m [2][2]; endclass
   class C10_method;  int v; int rc;
      function void go();
         rc = std::randomize(v) with { v == 100; };
      endfunction
   endclass

   C1_sum      k1;  C2_struct   k2;  C3_rollback k3;  C4_two  k4;
   C5_exact    k5;  C6_inline   k6;  C7_randc    k7;  C8_dyn  k8;
   C9_2d       k9;  C10_method  k10;
   int crc, s, ix;
   int lim;

   initial begin
      // ---- A0: the 1-D baseline, before anything ND has touched solver state ----
      local_1d_before_nd();

      // ---- B1 scalar + array, joint sum ----
      k1 = new; crc = k1.randomize(); s = k1.a.sum() with (32'(item));
      chk(30, crc, 1); chk(31, k1.t, 100); chk(32, s, 100);
      cells++;

      // ---- B2 packed-struct member ----
      k2 = new; crc = k2.randomize();
      chk(33, crc, 1); chk(34, k2.st.sa, 15); chk(35, k2.st.sb, 4);
      cells++;

      // ---- B3 UNSAT rollback restores the array ----
      k3 = new; k3.seed();
      chk(36, k3.a[0], 7);
      chk(37, k3.a.sum() with (32'(item)), 28);
      crc = k3.randomize(); s = k3.a.sum() with (32'(item));
      chk(38, crc, 0); chk(39, k3.t, 4242); chk(40, s, 28);
      cells++;

      // ---- B4 two arrays, joint sums ----
      k4 = new; crc = k4.randomize();
      chk(41, crc, 1);
      chk(42, k4.a.sum() with (32'(item)), 100);
      chk(43, k4.b.sum() with (32'(item)), 100);
      cells++;

      // ---- B5 exact elements ----
      k5 = new; crc = k5.randomize();
      chk(44, crc, 1); chk(45, k5.a[0], 25); chk(46, k5.a[3], 25);
      cells++;

      // ---- B6 inline with{} reading a module variable ----
      lim = 30;
      k6 = new;
      crc = k6.randomize() with { a.sum() with (32'(item)) == 100;
                                  foreach (a[i]) a[i] inside {[0:lim]}; };
      chk(47, crc, 1);
      chk(48, k6.a.sum() with (32'(item)), 100);
      // Unconditional so the check count cannot depend on the draw.
      for (ix = 0; ix < 4; ix++) chk(49 + ix, overlim(k6.a[ix]), 0);
      cells++;

      // ---- B7 randc ----
      k7 = new; crc = k7.randomize() with { c == 7; };
      chk(53, crc, 1); chk(54, k7.c, 7);
      cells++;

      // ---- B8 dynamic array + size() ----
      k8 = new; k8.d = new [4];
      crc = k8.randomize() with { d.size() == 4; d.sum() with (32'(item)) == 100;
                                  foreach (d[i]) d[i] inside {[20:60]}; };
      chk(55, crc, 1);
      chk(56, k8.d.sum() with (32'(item)), 100);
      cells++;

      // ---- B9 ND member array: the class-side half of this issue ----
      k9 = new;
      crc = k9.randomize() with { m[0][0] == 10;
                                  foreach (m[i]) foreach (m[i][j]) m[i][j] inside {[10:40]}; };
      chk(57, crc, 1);
      chk(58, k9.m[0][0], 10);
      chk(59, inband(k9.m[1][1]), 1);
      cells++;

      // ---- B10 std::randomize() called from inside a class method ----
      k10 = new; k10.go();
      chk(60, k10.rc, 1); chk(61, k10.v, 100);
      cells++;

      // ---- (A) the ND std::randomize cells, deliberately LAST so they cannot
      //      perturb the class cells above, then the poisoning probe behind them ----
      local_2d_static();       local_2d_automatic();
      local_2d_alone();        module_scalar_local_2d();
      local_3d();
      local_1d_after_nd();     // must stay last: probes ND -> 1-D state poisoning

      // Coverage guard: a cell that never ran, or bailed before its checks, must
      // not hide behind a green verdict.  17 cells / 61 checks is the exact union
      // of the two MWEs this replaces (26 + 28), plus the 3 checks of the
      // before/after 1-D pair that the original trailing control accounted for once.
      if (cells != 17 || checks != 61) begin
         $display("CHECKS_MISSING cells=%0d/17 checks=%0d/61", cells, checks);
         errors++;
      end

      if (errors == 0) $display("TEST_PASS");
      else             $display("TEST_FAIL: %0d CHECKS", errors);
      $display("summary: %0d checks, %0d failed", checks, errors);
      $finish;
   end
endmodule

