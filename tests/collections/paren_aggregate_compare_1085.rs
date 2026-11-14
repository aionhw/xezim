//! §11.3.2, §11.4.5, §13.4.1 (#1085): parentheses do not change an
//! operand's value, so the equality operators compare parenthesized unpacked
//! arrays and structs element by element (member by member) exactly as bare
//! ones — UVM's comparer macros write `(LVALUE) !== (RVALUE)`. A call that
//! returns an unpacked array compares its returned elements, the call running
//! once. Expected values are the reference simulator's, except the open
//! relation of x elements, which is x (§11.4.5; the reference simulator
//! prints 0).

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 1_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .collect()
}

fn expect(msgs: &[String], want: &[&str]) {
    for w in want {
        assert!(msgs.iter().any(|m| m == w), "missing {w:?} in {msgs:#?}");
    }
    assert!(
        !msgs.iter().any(|m| m.starts_with("FAIL")),
        "unexpected FAIL in {msgs:#?}"
    );
}

/// The reporter's program: fixed, dynamic, queue, logic, 1-bit, 2-D and
/// sub-array operands, class properties from outside and from a method, and
/// the result used in `if`, `?:`, `!` and `&&`.
#[test]
fn parenthesized_unpacked_arrays_compare_element_wise() {
    let msgs = lines(
        r##"
class holder;
    bit [7:0] s[2];
    bit [7:0] d[];
    bit [7:0] q[$];
    function new(bit [7:0] last);
        s = '{8'h01, last};
        d = '{8'h01, last};
        q = '{8'h01, last};
    endfunction
    function bit s_differs(holder r); return (s) !== (r.s); endfunction
    function bit d_differs(holder r); return (this.d) !== (r.d); endfunction
    function bit q_differs(holder r);
        if ((q) !== (r.q)) return 1'b1;
        return 1'b0;
    endfunction
endclass

module top;
    bit   [7:0] sa[2]    = '{8'h01, 8'h02};
    bit   [7:0] sb[2]    = '{8'h01, 8'h03};
    bit   [7:0] da[]     = '{8'h01, 8'h02};
    bit   [7:0] db[]     = '{8'h01, 8'h03};
    bit   [7:0] qa[$]    = '{8'h01, 8'h02};
    bit   [7:0] qb[$]    = '{8'h01, 8'h03};
    logic [7:0] la[2]    = '{8'h01, 8'h02};
    logic [7:0] lb[2]    = '{8'h01, 8'h03};
    bit         ba[3]    = '{1'b1, 1'b0, 1'b1};
    bit         bb[3]    = '{1'b1, 1'b1, 1'b1};
    int         ma[2][2] = '{'{1, 2}, '{3, 4}};
    int         mb[2][2] = '{'{1, 2}, '{3, 5}};
    int         fails    = 0;

    task automatic check(string what, logic got, logic exp);
        if (got !== exp) begin
            fails++;
            $display("FAIL %-34s got %b, expected %b", what, got, exp);
        end
    endtask

    initial begin
        holder x = new(8'h02);
        holder y = new(8'h03);
        logic  r;

        check("sa !== sb", sa !== sb, 1'b1);
        check("sa == sb", sa == sb, 1'b0);

        check("(sa) !== (sb)", (sa) !== (sb), 1'b1);
        check("(sa) === (sb)", (sa) === (sb), 1'b0);
        check("(sa) != (sb)", (sa) != (sb), 1'b1);
        check("(sa) == (sb)", (sa) == (sb), 1'b0);

        check("(sa) !== sb", (sa) !== sb, 1'b1);
        check("sa !== (sb)", sa !== (sb), 1'b1);
        check("((sa)) !== ((sb))", ((sa)) !== ((sb)), 1'b1);

        check("(da) !== (db)  dynamic", (da) !== (db), 1'b1);
        check("(da) == (db)   dynamic", (da) == (db), 1'b0);
        check("(qa) !== (qb)  queue", (qa) !== (qb), 1'b1);
        check("(qa) == (qb)   queue", (qa) == (qb), 1'b0);
        check("(la) !== (lb)  logic", (la) !== (lb), 1'b1);
        check("(ba) !== (bb)  1-bit elements", (ba) !== (bb), 1'b1);
        check("(ma) !== (mb)  two dimensions", (ma) !== (mb), 1'b1);
        check("(ma[1]) !== (mb[1])  sub-array", (ma[1]) !== (mb[1]), 1'b1);

        check("(x.s) !== (y.s)", (x.s) !== (y.s), 1'b1);
        check("(x.d) == (y.d)", (x.d) == (y.d), 1'b0);
        check("method (s) !== (r.s)", x.s_differs(y), 1'b1);
        check("method (this.d) !== (r.d)", x.d_differs(y), 1'b1);
        check("method if ((q) !== (r.q))", x.q_differs(y), 1'b1);

        r = 1'b0;
        if ((sa) !== (sb)) r = 1'b1;
        check("if ((sa) !== (sb))", r, 1'b1);
        check("((sa) !== (sb)) ? 1 : 0", ((sa) !== (sb)) ? 1'b1 : 1'b0, 1'b1);
        check("!((sa) === (sb))", !((sa) === (sb)), 1'b1);
        check("((sa) !== (sb)) && 1", ((sa) !== (sb)) && 1'b1, 1'b1);

        $display("%s", fails ? "TEST FAILED" : "TEST PASSED");
        $finish;
    end
endmodule
"##,
    );
    expect(&msgs, &["TEST PASSED"]);
}

/// Queues, associative arrays, x elements, wildcard equality, property chains,
/// methods, unpacked structs and a used result, all in parentheses.
#[test]
fn wrapped_aggregate_operands() {
    let msgs = lines(
        r##"
class inner_c;
  bit [7:0] s[2];
  int       q[$];
  function new(bit [7:0] l); s = '{8'h01, l}; q = '{1, l}; endfunction
endclass
class outer_c;
  inner_c in;
  function new(bit [7:0] l); in = new(l); endfunction
  function bit differs(outer_c o); return (in.s) !== (o.in.s); endfunction
  function bit qsame(outer_c o); return ((this.in.q)) == (o.in.q); endfunction
endclass
module top;
  typedef struct { int a; byte b; } s_t;
  bit [7:0] sa[2] = '{8'h01, 8'h02};
  bit [7:0] sb[2] = '{8'h01, 8'h03};
  bit [7:0] sc[2] = '{8'h01, 8'h02};
  int       qa[$] = '{1, 2};
  int       qb[$] = '{1, 3};
  int       qc[$] = '{1, 2};
  int       aa[int];
  int       ab[int];
  logic [7:0] la[2] = '{8'h01, 8'hxx};
  logic [7:0] lb[2] = '{8'h01, 8'hxx};
  s_t       u1, u2;
  initial begin
    outer_c x = new(8'h02);
    outer_c y = new(8'h03);
    outer_c z = new(8'h02);
    aa[1] = 5; aa[7] = 6; ab[1] = 5; ab[7] = 9;
    u1 = '{1, 8'd2}; u2 = '{1, 8'd3};
    $display("q %b %b", (qa) == (qc), (qa) != (qb));
    $display("aa %b %b", (aa) == (ab), (aa) !== (ab));
    $display("x %b %b", (la) == (lb), (la) === (lb));
    $display("wild %b %b", (sa) ==? (sb), (sa) !=? (sb));
    $display("chain %b %b %b", (x.in.s) !== (y.in.s), (x.in.s) === (z.in.s), (x.in.q) == (y.in.q));
    $display("method %b %b %b %b", x.differs(y), x.differs(z), x.qsame(z), x.qsame(y));
    $display("struct %b %b %b", (u1) == (u2), (u1) !== (u2), ((u1)) === (u1));
    $display("used %b", ((sa) == (sc)) ? 1'b1 : 1'b0);
  end
endmodule
"##,
    );
    expect(
        &msgs,
        &[
            "q 1 1",
            "aa 0 1",
            // §11.4.5: x elements no other element contradicts leave `==`
            // open (the reference simulator prints 0).
            "x x 1",
            "wild 0 1",
            "chain 1 1 0",
            "method 1 0 1 0",
            "struct 0 1 1",
            "used 1",
        ],
    );
}

/// A call returning a fixed-size, dynamic array or queue (of integral or
/// unpacked-struct elements) on either side, or both, a method call inside
/// its class; every call runs once.
#[test]
fn call_results_compare_element_wise() {
    let msgs = lines(
        r##"
module top;
  typedef bit [7:0] a2_t[2];
  typedef int iq_t[$];
  typedef int id_t[];
  typedef struct { int a; byte b; } s_t;
  typedef s_t sq_t[$];
  typedef s_t sf_t[2];
  bit [7:0] sa[2] = '{8'h01, 8'h02};
  int       qa[$] = '{1, 2};
  int       da[]  = '{1, 2};
  s_t       ss[2];
  s_t       sq[$];
  int calls = 0;
  int fails = 0;
  function automatic a2_t f(bit [7:0] l); calls++; return '{8'h01, l}; endfunction
  function automatic iq_t g(int l); iq_t r = '{1, l}; calls++; return r; endfunction
  function automatic id_t d(int l); id_t r = new[2]; r[0] = 1; r[1] = l; calls++; return r; endfunction
  function automatic sq_t sqf(byte l); sq_t r; r.push_back('{1, 8'd5}); r.push_back('{2, l}); calls++; return r; endfunction
  function automatic sf_t sff(byte l); sf_t r; r[0] = '{1, 8'd5}; r[1] = '{2, l}; calls++; return r; endfunction
  class C;
    int q[$] = '{1, 2};
    function iq_t get(); return q; endfunction
    function bit same(iq_t o); return get() == o; endfunction
    function bit same_this(); return this.get() == q; endfunction
  endclass
  task automatic check(string what, logic got, logic exp);
    if (got !== exp) begin fails++; $display("FAIL %-28s got %b, expected %b", what, got, exp); end
    else $display("ok   %-28s %b", what, got);
  endtask
  initial begin
    C c = new;
    ss[0] = '{1, 8'd5}; ss[1] = '{2, 8'd6};
    sq.push_back('{1, 8'd5}); sq.push_back('{2, 8'd6});
    check("f(2) == sa", f(2) == sa, 1'b1);
    check("f(3) != sa", f(3) != sa, 1'b1);
    check("(f(2)) === (sa)", (f(2)) === (sa), 1'b1);
    check("sa !== f(3)", sa !== f(3), 1'b1);
    check("g(2) == qa", g(2) == qa, 1'b1);
    check("qa == g(3)", qa == g(3), 1'b0);
    check("d(2) == da", d(2) == da, 1'b1);
    check("g(2) == d(2)", g(2) == d(2), 1'b1);
    check("g(2) != g(3)", g(2) != g(3), 1'b1);
    check("f(2) == f(2)", f(2) == f(2), 1'b1);
    check("sqf(6) == sq", sqf(6) == sq, 1'b1);
    check("sqf(7) == sq", sqf(7) == sq, 1'b0);
    check("sff(6) == ss", sff(6) == ss, 1'b1);
    check("sff(7) != ss", sff(7) != ss, 1'b1);
    check("c.get() == qa", c.get() == qa, 1'b1);
    check("c.same(qa)", c.same(qa), 1'b1);
    check("c.same_this()", c.same_this(), 1'b1);
    check("calls", calls == 17, 1'b1);
    $display("calls=%0d", calls);
    $display("%s", fails ? "TEST FAILED" : "TEST PASSED");
    $finish;
  end
endmodule
"##,
    );
    expect(&msgs, &["calls=17", "TEST PASSED"]);
}
