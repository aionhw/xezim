// Same-name cross-scope drive of a net with an internal z driver, where the
// driving signal has a DIFFERENT name. This always worked and is a control
// against the #1123 regression: only the name collision (top `n` vs leaf
// net `n`) must not resurrect the self-read -> x.
module leaf;
    wire n;
    assign n = 1'bz;
endmodule

module tb;
    logic q = 1'b1;
    leaf l ();
    assign l.n = q;
    initial begin
        #1;
        if (l.n !== 1'b1) begin
            $display("FAIL different-name control got %b, expected 1", l.n);
        end else begin
            $display("PASS");
        end
        $finish;
    end
endmodule