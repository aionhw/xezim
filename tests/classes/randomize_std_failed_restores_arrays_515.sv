`timescale 1ns/1ps
// A FAILED std::randomize() must leave every variable exactly as it was
// (IEEE 1800-2017 18.12).
//
// The call snapshots its targets before the trial draws, but the snapshot kept
// only the integral SCALAR targets: `array_operand_name(a).is_none()` filtered
// arrays out.  The trial draws and the per-item repair write the array targets
// too, and the joint solver commits an array element by element, so a call that
// ended unsatisfied left the rejected draw behind in the array while the
// scalars were correctly put back.  The class rand path already restores both.
//
// Reproduces under both static and automatic subroutines and at module scope,
// and even when the joint solver ran and correctly returned UNSAT -- so it is
// not an artefact of the draw-and-check fallback.
//
// Expected values come from the reference simulator, which passes every
// check.
module tb;
   int errors, checks;
   int mt;                     // module-scope scalar
   int ma [4];                 // module-scope array

   function automatic void chk(input int id, input int got, input int exp);
      checks++;
      if (got !== exp) begin
         errors++;
         $display("CHECK_FAIL id=%0d got=%0d exp=%0d", id, got, exp);
      end
   endfunction

   // UNSAT: 4 elements bounded to [20:60] can never sum to 999999
   function void rollback_static_fn();
      int a [4]; int t; int rc; int s;
      foreach (a[i]) a[i] = 7;
      t = 4242;
      rc = std::randomize(t, a) with {
               t == 100; a.sum() with (32'(item)) == 999999;
               foreach (a[i]) a[i] inside {[20:60]}; };
      s = a.sum() with (32'(item));
      chk(1, rc, 0); chk(2, t, 4242); chk(3, s, 28);
   endfunction

   function automatic void rollback_automatic_fn();
      int a [4]; int t; int rc; int s;
      foreach (a[i]) a[i] = 7;
      t = 4242;
      rc = std::randomize(t, a) with {
               t == 100; a.sum() with (32'(item)) == 999999;
               foreach (a[i]) a[i] inside {[20:60]}; };
      s = a.sum() with (32'(item));
      chk(4, rc, 0); chk(5, t, 4242); chk(6, s, 28);
   endfunction

   function void rollback_array_only();
      int a [4]; int rc; int s;
      foreach (a[i]) a[i] = 7;
      rc = std::randomize(a) with {
               a.sum() with (32'(item)) == 999999;
               foreach (a[i]) a[i] inside {[20:60]}; };
      s = a.sum() with (32'(item));
      chk(7, rc, 0); chk(8, s, 28);
   endfunction

   task rollback_module_scope();
      int rc; int s;
      mt = 4242;
      foreach (ma[i]) ma[i] = 7;
      rc = std::randomize(mt, ma) with {
               mt == 100; ma.sum() with (32'(item)) == 999999;
               foreach (ma[i]) ma[i] inside {[20:60]}; };
      s = ma.sum() with (32'(item));
      chk(9, rc, 0); chk(10, mt, 4242); chk(11, s, 28);
   endtask

   // ---- class rand path: the reference implementation, guards against
   // regression.  One class per shape: xezim evaluates ALL class constraints,
   // so constraint_mode() inside with{...} cannot select one at a time. ----
   class Pkt_ok;
      rand int t;
      rand int a [4];
      constraint c { t == 100; a.sum() with (32'(item)) == 100;
                     foreach (a[i]) a[i] inside {[20:60]}; }
   endclass

   class Pkt_bad;
      rand int t;
      rand int a [4];
      constraint c { a.sum() with (32'(item)) == 999999;
                     foreach (a[i]) a[i] inside {[20:60]}; }
   endclass

   task class_rand_checks();
      Pkt_ok  ok; Pkt_bad bad; int rc; int s;
      ok = new;
      rc = ok.randomize();
      s = ok.a.sum() with (32'(item));
      chk(12, rc, 1); chk(13, ok.t, 100); chk(14, s, 100);
      bad = new;
      bad.t = 4242;
      foreach (bad.a[i]) bad.a[i] = 7;
      chk(18, bad.a[0], 7);
      chk(19, bad.a.sum() with (32'(item)), 28);
      rc = bad.randomize();
      s = bad.a.sum() with (32'(item));
      chk(15, rc, 0); chk(16, bad.t, 4242); chk(17, s, 28);
   endtask

   initial begin
      rollback_static_fn();    rollback_automatic_fn();
      rollback_array_only();   rollback_module_scope();
      class_rand_checks();
      $display("T|checks=%0d failed=%0d", checks, errors);
      if (errors == 0) $display("TEST_PASS");
      else             $display("TEST_FAIL: %0d CHECKS", errors);
      $finish;
   end
endmodule