// Cousin-bug sweep for #1177: parameter-width properties in compiled
// methods assigned to NARROWER variables, exercising every division/shift/
// power operator + a broader mix of widths, plus methods WITHOUT the
// always-present loop to check the loop isn't what hides/triggers it.
package pkg;
    localparam int PW = 64;
endpackage

module top;
    import pkg::*;
    localparam int MW = 32;

    class item;
        bit [PW-1:0] p;
        bit [MW-1:0] m;
        bit [63:0]   l;
    endclass

    class calc;
        function int eight(); return 8; endfunction
        function int sev();   return 7; endfunction

        // %% and / into 3 bits
        function bit [2:0] mod3(item t);
            bit [2:0] r = t.p % eight();
            return r;
        endfunction
        function bit [2:0] div3(item t);
            bit [2:0] r = t.p / eight();
            return r;
        endfunction
        // >> and << into 8 bits
        function bit [7:0] shr8(item t);
            bit [7:0] r = t.p >> 4;
            return r;
        endfunction
        function bit [7:0] shl8(item t);
            bit [7:0] r = t.p << 4;
            return r;
        endfunction
        // arithmetic >> and << on a signed context
        function bit [7:0] ashr8(item t);
            bit [7:0] r = t.p >>> 4;
            return r;
        endfunction
        // ** into 16 bits
        function bit [15:0] pow16(item t);
            bit [15:0] r = 8'(t.p % 3) ** 2;
            return r;
        endfunction
        // % with a 7 (not power of two) divisor into 3 bits
        function bit [2:0] mod7(item t);
            bit [2:0] r = t.p % sev();
            return r;
        endfunction
        // / of a value that only fits in full width, into 8 bits
        function bit [7:0] divbig8(item t);
            bit [7:0] r = t.p / sev();
            return r;
        endfunction
        // operand on the RIGHT is the param-width property
        function bit [7:0] rhs_div8(item t);
            bit [7:0] r = 400 / t.p;
            return r;
        endfunction
        // nested: `%` result then `>>`
        function bit [2:0] nested(item t);
            bit [2:0] r = (t.p % eight()) >> 1;
            return r;
        endfunction
    endclass

    int errors = 0;
    task check(string name, int got, int exp);
        if (got != exp) begin
            errors++;
            if (errors <= 20) $display("FAIL %s = %0h, expected %0h", name, got, exp);
        end
    endtask

    initial begin
        calc c = new;
        item t = new;
        t.p = 64'h1234_0305;
        t.m = 32'h1234_0305;
        t.l = 64'h1234_0305;
        // self-determined reference values computed at full width.
        check("mod3",          c.mod3(t),    3'(64'h1234_0305 % 8));
        check("div3",          c.div3(t),    3'(64'h1234_0305 / 8));
        check("shr8",          c.shr8(t),    8'(64'h1234_0305 >> 4));
        check("shl8",          c.shl8(t),    8'(64'h1234_0305 << 4));
        check("ashr8",         c.ashr8(t),   8'(64'h1234_0305 >>> 4));
        check("mod7",          c.mod7(t),    3'(64'h1234_0305 % 7));
        check("divbig8",       c.divbig8(t), 8'(64'h1234_0305 / 7));
        check("rhs_div8",      c.rhs_div8(t),8'(400 / 64'h1234_0305));
        check("nested",        c.nested(t),  3'((64'h1234_0305 % 8) >> 1));
        if (errors == 0) $display("TEST PASSED");
        else $display("TEST FAILED (%0d errors)", errors);
        $finish;
    end
endmodule