// A method call whose RECEIVER is an index select of a local collection
// (`regs[ridx].m()` — the uvm_reg_hw_reset_seq shape) must splice into the
// suspend-aware process path: the callee's level-sensitive `wait` parks the
// process until its gate opens. Stage-1b splicing used to skip such calls
// (the receiver is not a plain name/member chain), and the synchronous
// fallback evaluated the callee's `wait` non-blockingly — every element
// completed at t=0 before the gate opened. This is the failure behind the
// sequencer "concurrent calls to get_next_item()" / mirrored-value-mismatch
// regression in the UVM register kit. IEEE 1800-2017 §13.3: a blocking task
// call suspends the calling process.
`timescale 1ns/1ns
module top;
    class A;
        int id;
        static int gate = 0;
        static int done = 0;
        function new(int i); id = i; endfunction
        task m();
            wait (gate == 1);
            done += id;
            $display("ELEM ran id=%0d @%0t", id, $time);
        endtask
    endclass

    class B;
        A pool[3];
        task run();
            do_all();
        endtask
        protected virtual task do_all();
            A regs[$];
            fill(regs);
            foreach (regs[ridx]) begin
                regs[ridx].m();   // Index-receiver call inside a spliced body
            end
        endtask
        task fill(ref A q[$]);
            q.push_back(pool[0]);
            q.push_back(pool[1]);
        endtask
    endclass

    B b;
    initial begin
        b = new();
        b.pool[0] = new(1);
        b.pool[1] = new(2);
        fork b.run(); join_none
    end
    initial begin
        #1;
        if (A::done != 0) begin
            // The callee ran through before the gate opened: the wait fell
            // through synchronously instead of suspending.
            $display("TAG_FAIL done=%0d at t=1 (want 0)", A::done);
            $finish;
        end
        #4 A::gate = 1;
        #1;
        if (A::done == 3) $display("TAG_PASS");
        else $display("TAG_FAIL done=%0d (want 3)", A::done);
    end
endmodule
