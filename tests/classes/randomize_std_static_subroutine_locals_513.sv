module tb;
   int errors, checks, cells;
   int mt;            // module-scope scalar
   int ma [4];        // module-scope array
   typedef struct packed { bit [7:0] a; bit [7:0] b; } S;
   S mp;              // module-scope struct (control that works)

   function automatic void chk(input int id, input int got, input int exp);
      checks++;
      if (got !== exp) begin
         errors++;
         $display("CHECK_FAIL id=%0d got=%0d exp=%0d", id, got, exp);
      end
   endfunction

   // =====================================================================
   // (A) static subroutine locals: scalar + array   -- checks 1..26
   // =====================================================================

   function void static_both();
      int a [4]; int t; int rc; int s;
      rc = std::randomize(t, a) with {
               t == 100; a.sum() with (32'(item)) == 100;
               foreach (a[i]) a[i] inside {[20:60]}; };
      s = a.sum() with (32'(item));
      chk(1, rc, 1); chk(2, t, 100); chk(3, s, 100);
      cells++;
   endfunction

   function void static_array_first();
      int a [4]; int t; int rc; int s;
      rc = std::randomize(a, t) with {
               t == 100; a.sum() with (32'(item)) == 100;
               foreach (a[i]) a[i] inside {[20:60]}; };
      s = a.sum() with (32'(item));
      chk(4, rc, 1); chk(5, t, 100); chk(6, s, 100);
      cells++;
   endfunction

   function void static_two_scalars();
      int a [4]; int t, u; int rc; int s;
      rc = std::randomize(t, u, a) with {
               t == 100; u == 5; a.sum() with (32'(item)) == 100;
               foreach (a[i]) a[i] inside {[20:60]}; };
      s = a.sum() with (32'(item));
      chk(7, rc, 1); chk(8, t, 100); chk(9, u, 5); chk(10, s, 100);
      cells++;
   endfunction

   function void static_declared();
      static int a [4]; static int t; int rc; int s;
      rc = std::randomize(t, a) with {
               t == 100; a.sum() with (32'(item)) == 100;
               foreach (a[i]) a[i] inside {[20:60]}; };
      s = a.sum() with (32'(item));
      chk(11, rc, 1); chk(12, t, 100); chk(13, s, 100);
      cells++;
   endfunction

   // ---- controls that already work ----
   function automatic void automatic_both();
      int a [4]; int t; int rc; int s;
      rc = std::randomize(t, a) with {
               t == 100; a.sum() with (32'(item)) == 100;
               foreach (a[i]) a[i] inside {[20:60]}; };
      s = a.sum() with (32'(item));
      chk(14, rc, 1); chk(15, t, 100); chk(16, s, 100);
      cells++;
   endfunction

   task automatic automatic_task();
      int a [4]; int t; int rc; int s;
      rc = std::randomize(t, a) with {
               t == 100; a.sum() with (32'(item)) == 100;
               foreach (a[i]) a[i] inside {[20:60]}; };
      s = a.sum() with (32'(item));
      chk(17, rc, 1); chk(18, t, 100); chk(19, s, 100);
      cells++;
   endtask

   task module_both();
      int rc; int s;
      mt = 0;
      foreach (ma[i]) ma[i] = 0;
      rc = std::randomize(mt, ma) with {
               mt == 100; ma.sum() with (32'(item)) == 100;
               foreach (ma[i]) ma[i] inside {[20:60]}; };
      s = ma.sum() with (32'(item));
      chk(20, rc, 1); chk(21, mt, 100); chk(22, s, 100);
      cells++;
   endtask

   function void local_array_only();
      int a [4]; int rc; int s;
      rc = std::randomize(a) with {
               a.sum() with (32'(item)) == 100;
               foreach (a[i]) a[i] inside {[20:60]}; };
      s = a.sum() with (32'(item));
      chk(23, rc, 1); chk(24, s, 100);
      cells++;
   endfunction

   function void local_scalar_only();
      int t; int rc;
      rc = std::randomize(t) with { t == 100; };
      chk(25, rc, 1); chk(26, t, 100);
      cells++;
   endfunction

   // =====================================================================
   // (B) local PACKED-STRUCT target   -- checks 27..53  (was ids 1..27)
   // =====================================================================

   function void local_struct_and_array();
      S p; int v; int a [4]; int rc; int s;
      rc = std::randomize(p, v, a) with {
               p.a == 15; p.b == 4; v == 100;
               a.sum() with (32'(item)) == 100;
               foreach (a[i]) a[i] inside {[20:60]}; };
      s = a.sum() with (32'(item));
      chk(27, rc, 1); chk(28, p.a, 15); chk(29, p.b, 4);
      chk(30, v, 100); chk(31, s, 100);
      cells++;
   endfunction

   function void local_struct_and_array_array_first();
      S p; int v; int a [4]; int rc; int s;
      rc = std::randomize(a, p, v) with {
               p.a == 15; p.b == 4; v == 100;
               a.sum() with (32'(item)) == 100;
               foreach (a[i]) a[i] inside {[20:60]}; };
      s = a.sum() with (32'(item));
      chk(32, rc, 1); chk(33, p.a, 15); chk(34, p.b, 4);
      chk(35, v, 100); chk(36, s, 100);
      cells++;
   endfunction

   // ---- controls ----
   function void module_struct_and_array();
      int v; int a [4]; int rc; int s;
      rc = std::randomize(mp, v, a) with {
               mp.a == 15; mp.b == 4; v == 100;
               a.sum() with (32'(item)) == 100;
               foreach (a[i]) a[i] inside {[20:60]}; };
      s = a.sum() with (32'(item));
      chk(37, rc, 1); chk(38, mp.a, 15); chk(39, mp.b, 4);
      chk(40, v, 100); chk(41, s, 100);
      cells++;
   endfunction

   function void local_struct_only();
      S p; int rc;
      rc = std::randomize(p) with { p.a == 15; p.b == 4; };
      chk(42, rc, 1); chk(43, p.a, 15); chk(44, p.b, 4);
      cells++;
   endfunction

   function void local_struct_and_scalar();
      S p; int v; int rc;
      rc = std::randomize(p, v) with { p.a == 15; p.b == 4; v == 100; };
      chk(45, rc, 1); chk(46, p.a, 15); chk(47, p.b, 4); chk(48, v, 100);
      cells++;
   endfunction

   function automatic void automatic_struct_and_array();
      S p; int v; int a [4]; int rc; int s;
      rc = std::randomize(p, v, a) with {
               p.a == 15; p.b == 4; v == 100;
               a.sum() with (32'(item)) == 100;
               foreach (a[i]) a[i] inside {[20:60]}; };
      s = a.sum() with (32'(item));
      chk(49, rc, 1); chk(50, p.a, 15); chk(51, p.b, 4);
      chk(52, v, 100); chk(53, s, 100);
      cells++;
   endfunction

   initial begin
      // (A)
      static_both();            static_array_first();
      static_two_scalars();     static_declared();
      automatic_both();         automatic_task();
      module_both();            local_array_only();   local_scalar_only();
      // (B)
      local_struct_and_array(); local_struct_and_array_array_first();
      module_struct_and_array();local_struct_only();
      local_struct_and_scalar();automatic_struct_and_array();

      // Coverage guard: a cell that never ran, or bailed before its checks, must
      // not be able to hide behind a green verdict.  15 cells / 53 checks is the
      // exact union of the two MWEs this file replaces (26 + 27).
      if (cells != 15 || checks != 53) begin
         $display("CHECKS_MISSING cells=%0d/15 checks=%0d/53", cells, checks);
         errors++;
      end

      $display("T|checks=%0d failed=%0d", checks, errors);
      if (errors == 0) $display("TEST_PASS");
      else             $display("TEST_FAIL: %0d CHECKS", errors);
      $display("summary: %0d checks, %0d failed", checks, errors);
      $finish;
   end
endmodule