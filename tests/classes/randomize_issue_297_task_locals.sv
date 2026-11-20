module mwe14_task_local_arrays ;
   typedef logic [31:0] u32_t ;
   typedef logic [3:0]  u4_t ;

   int failures = 0 ;
   function automatic void chk(bit ok, string msg);
      if (!ok) begin failures++ ; $display("FAIL : %s", msg) ; end
   endfunction

   // =====================================================================
   // FAILING t1 (mwe7/mwe13): byte array LOCAL to an automatic task
   // =====================================================================
   task automatic t1_task_local_sum();
      bit [7:0] a [] ;
      if (std::randomize(a) with {
         a.size() == 5 ;
         a.sum() with (8'(item)) == 3 ;
      }) begin
         chk(a.size() == 5, "t1: size mismatch") ;
         chk(a.sum() with (8'(item)) == 3, "t1: sum mismatch") ;
         $display("OK t1 task-local: size=%0d sum=%0d", a.size(), a.sum() with (8'(item))) ;
      end else chk(0, "t1 task-local array: std::randomize returned 0 (spurious)") ;
   endtask

   // =====================================================================
   // FAILING t2 (mwe13): byte QUEUE local to an automatic task
   // =====================================================================
   task automatic t2_task_local_queue();
      bit [7:0] q [$] ;
      if (std::randomize(q) with {
         q.size() == 5 ;
         q.sum() with (8'(item)) == 3 ;
      }) chk(q.sum() with (8'(item)) == 3, "t2: sum mismatch") ;
      else chk(0, "t2 task-local queue: std::randomize returned 0 (spurious)") ;
   endtask

   // =====================================================================
   // FAILING t3 (mwe13): array local to a CLASS TASK method
   // =====================================================================
   class CK ;
      task automatic m() ;
         bit [7:0] a [] ;
         if (std::randomize(a) with {
            a.size() == 5 ;
            a.sum() with (8'(item)) == 3 ;
         }) chk(a.sum() with (8'(item)) == 3, "t3: sum mismatch") ;
         else chk(0, "t3 class-task-local: std::randomize returned 0 (spurious)") ;
      endtask
   endclass

   // =====================================================================
   // FAILING t4 (mwe9, site S23): three task-local arrays sized by a scalar
   // drawn in the same solve + membership vs pre-filled queues + dist
   // =====================================================================
   u32_t cfg_nowr_addresses [$] ;
   u32_t hc_w1c_set [$] ;
   u32_t hc_on_the_fly_writes_set [$] ;
   task automatic t4_task_local_lists();
      bit    bcast_cfg [] ;
      u32_t  cfg_dest_addr [] ;
      u4_t   cfg_dest_host [] ;
      u4_t   num_reprog_regs ;
      if (std::randomize(num_reprog_regs, bcast_cfg, cfg_dest_addr, cfg_dest_host) with {
            num_reprog_regs >= 4 ;
            cfg_dest_addr.size()  == num_reprog_regs ;
            bcast_cfg.size()      == num_reprog_regs ;
            cfg_dest_host.size()  == num_reprog_regs ;
            foreach (cfg_dest_host[i]) { cfg_dest_host[i] < 2 ; }
            foreach (bcast_cfg[i]) {
               bcast_cfg[i] dist { 1 := 2, 0 := 1 };
               !(cfg_dest_addr[i] inside { cfg_nowr_addresses, hc_w1c_set }) ;
               cfg_dest_addr[i] inside { hc_on_the_fly_writes_set } ;
            }
         }) begin
         $display("OK t4: num_reprog_regs=%0d", num_reprog_regs) ;
         chk(num_reprog_regs >= 4, "t4: num_reprog_regs < 4") ;
         chk(cfg_dest_addr.size() == num_reprog_regs, "t4: size mismatch") ;
         foreach (bcast_cfg[i]) begin
            chk(!(cfg_dest_addr[i] inside {cfg_nowr_addresses, hc_w1c_set}), "t4: addr in nowr/w1c set") ;
            chk(cfg_dest_addr[i] inside {hc_on_the_fly_writes_set}, "t4: addr not in on_the_fly set") ;
            chk(cfg_dest_host[i] < 2, "t4: cfg_dest_host out of range") ;
         end
      end else chk(0, "t4 task-local lists: std::randomize returned 0 (spurious)") ;
   endtask

   // =====================================================================
   // CONTROL c1 (mwe13): IDENTICAL code local to a FUNCTION: passes
   // =====================================================================
   function automatic void c1_function_local();
      bit [7:0] a [] ;
      if (std::randomize(a) with {
         a.size() == 5 ;
         a.sum() with (8'(item)) == 3 ;
      }) chk(a.sum() with (8'(item)) == 3, "c1: sum mismatch") ;
      else chk(0, "c1 function-local: std::randomize returned 0") ;
   endfunction

   // =====================================================================
   // CONTROL c2 (mwe7): module-scope array, same constraints: passes
   // =====================================================================
   bit [7:0] m [] ;
   task automatic c2_module_scope();
      if (std::randomize(m) with {
         m.size() == 5 ;
         m.sum() with (8'(item)) == 3 ;
      }) chk(m.sum() with (8'(item)) == 3, "c2: sum mismatch") ;
      else chk(0, "c2 module-scope: std::randomize returned 0") ;
   endtask

   initial begin
      CK k = new ;
      // pre-filled state for t4 (set by the environment before the randomize)
      cfg_nowr_addresses       = '{ 32'h0000_0180, 32'h0000_0184 } ;
      hc_w1c_set               = '{ 32'h0000_4000 } ;
      hc_on_the_fly_writes_set = '{ 32'h0000_5000, 32'h0000_5004, 32'h0000_5008,
                                    32'h0000_500C, 32'h0000_5010, 32'h0000_5014 } ;
      t1_task_local_sum() ;    // <-- fails under xezim
      t2_task_local_queue() ;  // <-- fails under xezim
      k.m() ;                  // <-- fails under xezim
      t4_task_local_lists() ;  // <-- fails under xezim
      c1_function_local() ;    // control: passes under xezim
      c2_module_scope() ;      // control: passes under xezim
      // CONTROL c3 (mwe7/mwe9): identical code inline in the initial block
      begin
         bit [7:0] b [] ;
         if (std::randomize(b) with {
            b.size() == 5 ;
            b.sum() with (8'(item)) == 3 ;
         }) chk(b.sum() with (8'(item)) == 3, "c3 inline: sum mismatch") ;
         else chk(0, "c3 inline: std::randomize returned 0") ;
      end
      if (failures == 0) $display("TEST_PASS");
      else begin $display("TEST_FAIL count=%0d", failures); $fatal(1); end
   end
endmodule

