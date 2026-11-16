// Cousin-bug sweep for #1178: package types sized by an imported parameter,
// in every affected shape the issue listed plus deeper nesting.
package widths;
    localparam int AW = 8;
    localparam int BW = 4;
    localparam int CW = 16;
endpackage

package user_mix;
    import widths::*;
    // vector typedef
    typedef bit [AW-1:0] vec_t;
    // packed struct
    typedef struct packed { bit [AW-1:0] a; bit [BW-1:0] b; } pair_t;
    // packed union
    typedef union packed { bit [AW-1:0] x; bit [AW-1:0] y; } alt_t;
    // unpacked struct
    typedef struct { bit [AW-1:0] a; bit [BW-1:0] b; } upair_t;
    // nested struct typedef referencing another typedef
    typedef struct packed { pair_t p; bit [CW-1:0] c; } deep_t;

    pair_t g;
    deep_t dg;

    // typedef inside a class, plus a property
    class holder;
        typedef struct packed { bit [AW-1:0] a; bit [BW-1:0] b; } key_t;
        int count[key_t];
        bit [AW-1:0] v;   // property sized by imported param
        function key_t make(bit [AW-1:0] a, bit [BW-1:0] b);
            key_t k = '{a: a, b: b};
            return k;
        endfunction
    endclass
endpackage

package user_named;
    import widths::AW;
    import widths::BW;
    import widths::CW;
    typedef struct packed { bit [AW-1:0] a; bit [BW-1:0] b; } pair_t;
    typedef struct packed { pair_t p; bit [CW-1:0] c; } deep_t;
endpackage

module top;
    int errors = 0;
    task automatic check(string name, int got, int exp);
        if (got != exp) begin
            errors++;
            if (errors <= 20) $display("FAIL %s = %0h, expected %0h", name, got, exp);
        end
    endtask

    initial begin
        user_mix::holder h  = new;
        user_mix::holder::key_t k;
        user_mix::pair_t   p;
        user_mix::alt_t    a;
        user_mix::upair_t  u;
        user_mix::deep_t   d;

        check("$bits(vec_t)",     $bits(user_mix::vec_t), 8);
        check("$bits(pair_t)",    $bits(user_mix::pair_t), 12);
        check("$bits(alt_t)",     $bits(user_mix::alt_t), 8);
        check("$bits(upair_t)",   $bits(user_mix::upair_t), 12);
        check("$bits(deep_t)",    $bits(user_mix::deep_t), 28);
        check("$bits(user_mix::g)", $bits(user_mix::g), 12);
        check("$bits(user_mix::dg)", $bits(user_mix::dg), 28);
        check("$bits(holder::key_t)", $bits(user_mix::holder::key_t), 12);
        check("$bits(user_named::deep_t)", $bits(user_named::deep_t), 28);

        p = '{a: 8'hAB, b: 4'h7};
        check("pair value", p, 12'hAB7);
        d = '{p: p, c: 16'h1234};
        check("deep value", d, 28'h0AB71234);
        a.y = 8'h5A;
        check("alt .x", a.x, 8'h5A);

        h.v = 8'hE5;
        check("prop value", h.v, 8'hE5);

        k = h.make(8'h0F, 4'hA);
        check("make(15,10)", k, 12'h0FA);
        h.count[h.make(8'h01, 4'h0)]++;
        h.count[h.make(8'h02, 4'h0)]++;
        h.count[h.make(8'h03, 4'h0)]++;
        check("assoc count num", h.count.num(), 3);

        if (errors == 0) $display("TEST PASSED");
        else $display("TEST FAILED (%0d errors)", errors);
        $finish;
    end
endmodule