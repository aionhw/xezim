// Pure-SV regression test for IEEE 1800-2023 §6.21 / §13.3.1:
// A `static` local in a subroutine retains its value across invocations.
// In a phase-jump / re-entry pattern, a task suspends (e.g. `#10`), concurrent
// processes execute and suspend inside subroutines, and the task mutates its
// static local (`first = 0`) before jumping back to restart the schedule.
// On the re-entered call, the task must observe `first == 0` (the static
// latched through the jump), preventing an infinite jump loop.

module top;
  event ev_jump;
  event ev_done;
  int phase_cycle = 0;

  class Component;
    int run_count = 0;

    task main_phase();
      static int first = 1;
      #10;
      run_count++;
      if (first == 1) begin
        $display("TAG_FIRST at %0t first=%0d", $time, first);
        first = 0;
        ->ev_jump;
      end else if (first == 0) begin
        $display("TAG_SECOND at %0t first=%0d", $time, first);
        ->ev_done;
      end else begin
        $display("TAG_FAIL unexpected first=%0d", first);
      end
    endtask
  endclass

  // Background worker that stays suspended inside a subroutine frame
  task automatic bg_service();
    #2;
    bg_nested_wait();
  endtask

  task automatic bg_nested_wait();
    #200;
  endtask

  Component comp;

  initial begin
    comp = new();

    fork
      // Background worker: suspends at t=2 inside bg_nested_wait
      begin
        bg_service();
      end

      // Phase scheduler loop
      begin
        while (phase_cycle < 5) begin
          phase_cycle++;
          fork
            begin
              comp.main_phase();
            end
          join_none

          // Wait for jump or completion
          @(ev_jump or ev_done);
          if (ev_done.triggered) begin
            break;
          end
          // On jump, small delta before re-launching schedule
          #1;
        end
      end
    join_any

    #2;
    if (comp.run_count == 2 && phase_cycle == 2) begin
      $display("TAG_PASS latched across jump (run_count=%0d, cycles=%0d)",
               comp.run_count, phase_cycle);
    end else begin
      $display("TAG_FAIL run_count=%0d cycles=%0d",
               comp.run_count, phase_cycle);
    end
  end
endmodule
