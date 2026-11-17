// Cross-scope z driver resolved against a same-named top drive. The leaf's
// internal `assign n = 1'bz` is high-impedance, so the top drive's value must
// win and the net must NOT read x (this is the exact #1123 self-read that
// previously requalified the top driver read into the leaf net).
module leaf;
    wire n;
    assign n = 1'bz;
endmodule

module tb;
    logic n = 1'b0;
    leaf l ();
    assign l.n = n;
    initial begin
        #1;
        if (l.n !== 1'b0) begin
            $display("FAIL z-resolve got %b, expected 0", l.n);
        end else begin
            $display("PASS");
        end
        n = 1'b1;
        #1;
        if (l.n !== 1'b1) begin
            $display("FAIL z-resolve high got %b, expected 1", l.n);
        end else begin
            $display("PASS");
        end
        $finish;
    end
endmodule