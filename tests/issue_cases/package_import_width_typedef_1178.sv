// Types and variables declared in a package, sized by a parameter imported from another
// package. The imported parameter gives the width just as a local one does (IEEE 1800-2017
// 26.3), so every $bits below is 8 or 12, and a value stored in such a type keeps all its bits.
package widths;
    localparam int AW = 8;
    localparam int BW = 4;
endpackage

package user_wild;
    import widths::*;
    typedef bit [AW-1:0] vec_t;
    typedef struct packed {
        bit [AW-1:0] a;
        bit [BW-1:0] b;
    } pair_t;
    typedef union packed {
        bit [AW-1:0] x;
        bit [AW-1:0] y;
    } alt_t;
    typedef struct {
        bit [AW-1:0] a;
        bit [BW-1:0] b;
    } upair_t;
    pair_t g;

    class holder;
        typedef struct packed {
            bit [AW-1:0] a;
            bit [BW-1:0] b;
        } key_t;
        int count[key_t];

        function key_t make(bit [AW-1:0] a, bit [BW-1:0] b);
            key_t k = '{a: a, b: b};
            return k;
        endfunction
    endclass
endpackage

package user_named;
    import widths::AW;
    import widths::BW;
    typedef struct packed {
        bit [AW-1:0] a;
        bit [BW-1:0] b;
    } pair_t;
endpackage

package user_scoped;
    typedef struct packed {
        bit [widths::AW-1:0] a;
        bit [widths::BW-1:0] b;
    } pair_t;
endpackage

module top;
    int errors = 0;

    task automatic check(string name, int got, int exp);
        if (got != exp) begin
            errors++;
            $display("FAIL %s = %0h, expected %0h", name, got, exp);
        end
    endtask

    initial begin
        user_wild::holder        h = new;
        user_wild::holder::key_t k;
        user_wild::pair_t        p;
        user_named::pair_t       n;
        user_wild::upair_t       u;

        check("$bits(user_wild::vec_t)", $bits(user_wild::vec_t), 8);
        check("$bits(user_wild::pair_t)", $bits(user_wild::pair_t), 12);
        check("$bits(user_wild::alt_t)", $bits(user_wild::alt_t), 8);
        check("$bits(user_wild::upair_t)", $bits(user_wild::upair_t), 12);
        check("$bits(user_wild::g)", $bits(user_wild::g), 12);
        check("$bits(user_wild::holder::key_t)", $bits(user_wild::holder::key_t), 12);
        check("$bits(user_named::pair_t)", $bits(user_named::pair_t), 12);
        check("$bits(user_scoped::pair_t)", $bits(user_scoped::pair_t), 12);

        p = '{a: 8'h5A, b: 4'h3};
        check("user_wild::pair_t value", p, 12'h5A3);
        n = '{a: 8'h5A, b: 4'h3};
        check("user_named::pair_t value", n, 12'h5A3);
        u = '{a: 8'h5A, b: 4'h3};
        check("user_wild::upair_t .a", u.a, 8'h5A);
        user_wild::g = 12'h5A3;
        check("user_wild::g value", user_wild::g, 12'h5A3);

        k = h.make(8'h01, 4'h0);
        check("holder.make(1, 0)", k, 12'h010);
        h.count[h.make(8'h01, 4'h0)]++;
        h.count[h.make(8'h02, 4'h0)]++;
        h.count[h.make(8'h03, 4'h0)]++;
        check("holder.count.num()", h.count.num(), 3);

        if (errors == 0) $display("TEST PASSED");
        else $display("TEST FAILED (%0d errors)", errors);
        $finish;
    end
endmodule
