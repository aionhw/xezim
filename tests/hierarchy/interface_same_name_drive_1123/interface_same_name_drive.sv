// IEEE 1800-2023 §23.6/§23.9 (cross-scope drive), upstream #1123.
//
// Driving an interface net from a signal of the SAME name (top's `n` driving
// `u_same.n`, where `u_same` also has a net `n` with its own internal z
// driver) must resolve the interface net to the driving signal's value once
// the interface's enable is 0 (its internal driver is high-impedance z).
interface sub_if;
    wire  n;
    logic val = 1'b0;
    bit   en = 1'b0;
    assign n = en ? val : 1'bz;
endinterface

module tb;
    logic  n = 1'b0;
    int    fails = 0;
    sub_if u_same ();
    assign u_same.n = n;
    task automatic check(string what, logic got, logic exp);
        if (got !== exp) begin
            fails++;
            $display("FAIL %-34s got %b, expected %b", what, got, exp);
        end else begin
            $display("PASS %s", what);
        end
    endtask
    initial begin
        #1;
        check("same name, outside 0", u_same.n, 1'b0);
        n = 1'b1;
        #1;
        check("same name, outside 1", u_same.n, 1'b1);
        // Interface's internal driver fully on: it and the top drive agree.
        u_same.en  = 1'b1;
        u_same.val = 1'b1;
        #1;
        check("same name, inside 1, outside 1", u_same.n, 1'b1);
        $display("%s", fails ? "TEST FAILED" : "TEST PASSED");
        $finish;
    end
endmodule