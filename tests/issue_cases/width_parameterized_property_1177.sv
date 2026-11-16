// A class property whose width comes from a package or module parameter, read in an
// expression that is assigned to a narrower variable. The operands take the width of the
// widest of them (64 bits) before the operation, so the result is only cut down afterwards
// (IEEE 1800-2017 11.6.1, 11.8.2). Each method holds a loop that never runs.
package pkg;
    localparam int PW = 64;
endpackage

module top;
    import pkg::*;
    localparam int MW = 64;

    class item;
        bit [PW-1:0] p;  // package parameter
        bit [MW-1:0] m;  // module parameter
        bit [63:0]   l;  // literal width (control)
    endclass

    class calc;
        int q[$];
        int nine = 9;

        function int eight();
            return 8;
        endfunction

        function bit [2:0] p_mod(item t);
            bit [2:0] r = t.p % eight();
            for (int i = 0; i < q.size(); i++);
            return r;
        endfunction

        function bit [2:0] p_mod_var(item t);
            bit [2:0] r = t.p % nine;
            for (int i = 0; i < q.size(); i++);
            return r;
        endfunction

        function bit [2:0] p_mod_cast(item t);
            bit [2:0] r = 3'(t.p % eight());
            for (int i = 0; i < q.size(); i++);
            return r;
        endfunction

        function bit [2:0] p_mod_assign(item t);
            bit [2:0] r;
            r = t.p % eight();
            for (int i = 0; i < q.size(); i++);
            return r;
        endfunction

        function bit [7:0] p_div(item t);
            bit [7:0] r = t.p / eight();
            foreach (q[i]);
            return r;
        endfunction

        function bit [7:0] p_shr(item t);
            bit [7:0] r = t.p >> 4;
            foreach (q[i]);
            return r;
        endfunction

        function bit [7:0] m_shr(item t);
            bit [7:0] r = t.m >> 4;
            foreach (q[i]);
            return r;
        endfunction

        function bit [2:0] m_mod(item t);
            bit [2:0] r = t.m % eight();
            foreach (q[i]);
            return r;
        endfunction

        function bit [7:0] l_shr(item t);
            bit [7:0] r = t.l >> 4;
            foreach (q[i]);
            return r;
        endfunction

        function bit [2:0] l_mod(item t);
            bit [2:0] r = t.l % eight();
            foreach (q[i]);
            return r;
        endfunction
    endclass

    int errors = 0;

    task automatic check(string name, int got, int exp);
        if (got != exp) begin
            errors++;
            if (errors <= 20) $display("FAIL %s = %0h, expected %0h", name, got, exp);
        end
    endtask

    initial begin
        calc  c = new;
        item  t = new;
        t.p = 64'h1234_0305;
        t.m = 64'h1234_0305;
        t.l = 64'h1234_0305;
        for (int n = 0; n < 1100; n++) begin
            check("p % eight()", c.p_mod(t), 5);
            check("p % nine", c.p_mod_var(t), 3'(64'h1234_0305 % 9));
            check("3'(p % eight())", c.p_mod_cast(t), 5);
            check("r = p % eight()", c.p_mod_assign(t), 5);
            check("p / eight()", c.p_div(t), 8'h60);
            check("p >> 4", c.p_shr(t), 8'h30);
            check("m >> 4", c.m_shr(t), 8'h30);
            check("m % eight()", c.m_mod(t), 5);
            check("l >> 4", c.l_shr(t), 8'h30);
            check("l % eight()", c.l_mod(t), 5);
        end
        if (errors == 0) $display("TEST PASSED");
        else $display("TEST FAILED (%0d errors)", errors);
        $finish;
    end
endmodule
