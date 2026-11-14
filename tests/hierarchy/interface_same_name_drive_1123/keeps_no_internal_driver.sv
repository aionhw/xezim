// Interface net with NO internal driver, driven from a same-named top signal.
// A single cross-scope drive with a name collision must still flow through
// (§23.6 — the RHS `n` in `assign u_same.n = n` is tb's `n`).
interface sub_if;
    wire n;   // no internal driver at all
endinterface

module tb;
    logic n = 1'b0;
    sub_if u_same ();
    assign u_same.n = n;
    initial begin
        #1;
        if (u_same.n !== 1'b0) begin
            $display("FAIL no-internal-driver got %b, expected 0", u_same.n);
        end else begin
            $display("PASS");
        end
        n = 1'b1;
        #1;
        if (u_same.n !== 1'b1) begin
            $display("FAIL no-internal-driver high got %b, expected 1", u_same.n);
        end else begin
            $display("PASS");
        end
        $finish;
    end
endmodule