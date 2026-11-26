module mwe12_struct_local_array ;
   typedef struct packed { logic [7:0] d ; logic v ; } s_t ;
   int failures = 0 ;
   function automatic void chk(bit ok, string msg);
      if (!ok) begin failures++ ; $display("FAIL : %s", msg) ; end
   endfunction

   // ---- failing shape 1: struct array local to a TASK ----
   task automatic in_task();
      s_t a [] ;
      if (std::randomize(a) with { a.size() == 3 ;
                                   foreach (a[i]) { a[i].d == i ; a[i].v == 1 ; } })
         chk(a.size() == 3 && a[1].d == 1 && a[1].v == 1, "task: field mismatch") ;
      else chk(0, "task-local: std::randomize returned 0 (spurious)") ;
   endtask

   // ---- failing shape 2: identical, local to a FUNCTION ----
   function automatic void in_func();
      s_t a [] ;
      if (std::randomize(a) with { a.size() == 3 ;
                                   foreach (a[i]) { a[i].d == i ; a[i].v == 1 ; } })
         chk(a.size() == 3 && a[1].d == 1 && a[1].v == 1, "func: field mismatch") ;
      else chk(0, "function-local: std::randomize returned 0 (spurious)") ;
   endfunction

   initial begin
      in_task() ;                     // <-- fails under xezim
      in_func() ;                     // <-- fails under xezim
      if (failures == 0) $display("TEST_PASS");
      else begin $display("TEST_FAIL count=%0d", failures); $fatal(1); end
   end
endmodule



