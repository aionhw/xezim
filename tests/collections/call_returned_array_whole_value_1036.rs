//! §11.4.13, §21.2.1.7, §13.4.1, §18.5.11 (#1036, reported by
//! git-hevangel): a call that returns an unpacked array (queue, dynamic,
//! fixed-size or associative, named through a typedef) used as a whole
//! operand of `inside` or `case ... inside` contributes the elements it
//! returns, as the argument of `%p` prints them as the same value in a
//! variable does, and counts them for `$size`; in an unpacked array
//! concatenation it contributes them in order, and a constraint draws from
//! them. The call runs once. Read as a scalar, the call compared as 0 and
//! printed 0. A bare call in a class method binds to the class first
//! (§8.20), and `this.f()`, `pkg::f()`, `C::f()` and a method known by its
//! `pure virtual` or `extern` prototype are recognized as returning an array
//! too. An associative array variable in an `inside` set contributes its
//! elements as well. Expected values are the reference simulator's.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 100_000)
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

#[test]
fn inside_and_p_of_a_returned_array() {
    let msgs = lines(
        r##"
module top;
  typedef int word_q_t[$];
  word_q_t stored = '{1, 2, 4};

  function automatic word_q_t get_q();
    return stored;
  endfunction

  class holder;
    word_q_t items = '{1, 2, 4, 8, 16};
    extern function word_q_t get_ext();
    function bit has(int n);
      return n inside {get_ext()};   // a bare call of an extern method
    endfunction
  endclass

  function word_q_t holder::get_ext();
    return items;
  endfunction

  initial begin
    holder h = new;
    word_q_t q = get_q();
    string from_call = $sformatf("%p", get_q());
    string from_var  = $sformatf("%p", q);
    $display("%s control: q = f() keeps %0d elements", q.size() == 3 ? "PASS" : "FAIL", q.size());
    $display("%s 4 inside {f()}", (4 inside {get_q()}) ? "PASS" : "FAIL");
    $display("%s 0 is not inside {f()}", !(0 inside {get_q()}) ? "PASS" : "FAIL");
    $display("%s %%p of f() prints %s, the variable prints %s",
             from_call == from_var ? "PASS" : "FAIL", from_call, from_var);
    $display("%s 8 inside {bare call of an extern method}", h.has(8) ? "PASS" : "FAIL");
    $finish;
  end
endmodule
"##,
    );
    expect(
        &msgs,
        &[
            "PASS control: q = f() keeps 3 elements",
            "PASS 4 inside {f()}",
            "PASS 0 is not inside {f()}",
            "PASS %p of f() prints '{1, 2, 4}, the variable prints '{1, 2, 4}",
            "PASS 8 inside {bare call of an extern method}",
        ],
    );
}

/// The reporter's 36 checks: every kind of array, a queue of strings, sets
/// mixing scalars and calls, handle, `extern`, bare and base-class calls.
#[test]
fn every_kind_of_returned_array() {
    let msgs = lines(
        r##"
package defs;
  typedef int word_q_t[$];
endpackage

module top;
  typedef int    word_q_t[$];
  typedef int    word_da_t[];
  typedef int    word_fx_t[3];
  typedef int    word_aa_t[int];
  typedef string name_q_t[$];

  int fails = 0;
  word_q_t stored = '{1, 2, 4};

  task automatic check_int(input string what, input int got, input int want);
    if (got == want) $display("PASS %s", what);
    else begin
      $display("FAIL %s: got %0d, expected %0d", what, got, want);
      fails++;
    end
  endtask

  task automatic check_str(input string what, input string got, input string want);
    if (got == want) $display("PASS %s", what);
    else begin
      $display("FAIL %s: got %s, expected %s", what, got, want);
      fails++;
    end
  endtask

  function automatic word_q_t get_q();
    return stored;
  endfunction

  function automatic word_da_t get_da();
    word_da_t d = new[2];
    d[0] = 5;
    d[1] = 6;
    return d;
  endfunction

  function automatic word_fx_t get_fx();
    word_fx_t f = '{7, 8, 9};
    return f;
  endfunction

  function automatic word_aa_t get_aa();
    word_aa_t a;
    a[3] = 30;
    a[1] = 10;
    return a;
  endfunction

  function automatic name_q_t get_names();
    name_q_t s = '{"x", "yz"};
    return s;
  endfunction

  function automatic int pick();
    return 100;
  endfunction

  class holder;
    defs::word_q_t items = '{1, 2, 4, 8, 16};
    extern function defs::word_q_t get_ext();
    function defs::word_q_t get_in();
      return items;
    endfunction
    function defs::word_q_t pick();
      return items;
    endfunction
    function int in_has(int n);
      return n inside {get_in()};
    endfunction
    function int ext_has(int n);
      return n inside {get_ext()};
    endfunction
    function int pick_has(int n);
      return n inside {pick()};
    endfunction
    function string in_p();
      return $sformatf("%p", get_in());
    endfunction
    function string ext_p();
      return $sformatf("%p", get_ext());
    endfunction
  endclass

  function defs::word_q_t holder::get_ext();
    return items;
  endfunction

  class derived extends holder;
    function int base_has(int n);
      return n inside {get_in()};
    endfunction
  endclass

  initial begin
    holder  h  = new;
    derived dv = new;
    word_q_t q;
    word_da_t da;
    word_fx_t fx;
    word_aa_t aa;
    name_q_t nq;
    int n = 4;
    string s;

    q  = get_q();
    da = get_da();
    fx = get_fx();
    aa = get_aa();
    nq = get_names();

    check_int("control: q = f()",          q.size(), 3);
    check_int("control: f().size()",       get_q().size(), 3);
    check_int("control: copy then inside", 4 inside {q}, 1);

    check_int("inside queue, member",          4 inside {get_q()}, 1);
    check_int("inside queue, variable",        n inside {get_q()}, 1);
    check_int("inside queue, non-member",      3 inside {get_q()}, 0);
    check_int("inside queue, zero",            0 inside {get_q()}, 0);
    check_int("inside dynamic array",          6 inside {get_da()}, 1);
    check_int("inside fixed-size array",       9 inside {get_fx()}, 1);
    check_int("inside associative, value",     30 inside {get_aa()}, 1);
    check_int("inside associative, key only",  3 inside {get_aa()}, 0);
    check_int("inside set of scalars and a call, call",   4 inside {10, get_q(), 99}, 1);
    check_int("inside set of scalars and a call, scalar", 99 inside {10, get_q(), 99}, 1);
    check_int("inside set of scalars and a call, none",   5 inside {10, get_q(), 99}, 0);
    check_int("inside two calls",              5 inside {get_q(), get_da()}, 1);

    s = $sformatf("%p", get_q());     check_str("%p queue",             s, $sformatf("%p", q));
    s = $sformatf("%p", get_da());    check_str("%p dynamic array",     s, $sformatf("%p", da));
    s = $sformatf("%p", get_fx());    check_str("%p fixed-size array",  s, $sformatf("%p", fx));
    s = $sformatf("%p", get_aa());    check_str("%p associative array", s, $sformatf("%p", aa));
    s = $sformatf("%p", get_names()); check_str("%p string queue",      s, $sformatf("%p", nq));
    $swrite(s, "%p", get_q());        check_str("$swrite %p queue",     s, $sformatf("%p", q));

    check_int("handle call, inside",         16 inside {h.get_in()}, 1);
    check_int("handle call, inside, miss",   3 inside {h.get_in()}, 0);
    check_int("extern method, inside",       8 inside {h.get_ext()}, 1);
    s = $sformatf("%p", h.get_in());  check_str("handle call, %p",   s, $sformatf("%p", h.items));
    s = $sformatf("%p", h.get_ext()); check_str("extern method, %p", s, $sformatf("%p", h.items));

    check_int("bare call, inside",            h.in_has(8), 1);
    check_int("bare call, inside, miss",      h.in_has(3), 0);
    check_int("bare extern call, inside",     h.ext_has(16), 1);
    check_int("bare extern call, miss",       h.ext_has(3), 0);
    check_str("bare call, %p",                h.in_p(),  $sformatf("%p", h.items));
    check_str("bare extern call, %p",         h.ext_p(), $sformatf("%p", h.items));
    check_int("bare call binds to the class", h.pick_has(4), 1);
    check_int("bare call is not the module function", h.pick_has(100), 0);
    check_int("bare call of a base method",   dv.base_has(8), 1);
    check_int("bare call of a base method, miss", dv.base_has(3), 0);

    $display("RESULT %s", fails == 0 ? "PASS" : "FAIL");
    $finish;
  end
endmodule
"##,
    );
    assert_eq!(
        msgs.iter().filter(|m| m.starts_with("PASS ")).count(),
        36,
        "{msgs:#?}"
    );
    expect(&msgs, &["RESULT PASS"]);
}

#[test]
fn constraint_draws_from_a_returned_array() {
    let msgs = lines(
        r##"
module top;
  typedef int word_q_t[$];

  function automatic word_q_t allowed();
    return '{1, 2, 4};
  endfunction

  class item;
    rand int x;
    constraint in_returned { x inside {allowed()}; }
  endclass

  initial begin
    item it = new;
    int bad = 0;
    repeat (20) if (!it.randomize() || !(it.x inside {1, 2, 4})) bad++;
    $display("%s x is drawn from the returned elements (%0d of 20 draws were not)", bad == 0 ? "PASS" : "FAIL", bad);
    $finish;
  end
endmodule
"##,
    );
    expect(
        &msgs,
        &["PASS x is drawn from the returned elements (0 of 20 draws were not)"],
    );
}

/// Each `inside` and `%p` runs the call once; `size()` of a bare call and of
/// `this.f()` inside the class reads the returned array.
#[test]
fn the_call_runs_once() {
    let msgs = lines(
        r##"
module top;
  typedef int word_q_t[$];
  int calls = 0;
  function automatic word_q_t f();
    calls++;
    return '{1, 2, 4};
  endfunction
  class k;
    word_q_t items = '{3, 5};
    function word_q_t get(); return items; endfunction
    function int bare_size(); return get().size(); endfunction
    function int this_size(); return this.get().size(); endfunction
  endclass
  initial begin
    k h = new;
    string s;
    bit hit;
    hit = 2 inside {f()};
    s = $sformatf("%p", f());
    $display("hit=%0d p=%s calls=%0d", hit, s, calls);
    hit = 9 inside {7, f(), 9};
    $display("hit=%0d calls=%0d", hit, calls);
    $display("sizes %0d %0d", h.bare_size(), h.this_size());
    $finish;
  end
endmodule
"##,
    );
    expect(
        &msgs,
        &["hit=1 p='{1, 2, 4} calls=2", "hit=1 calls=3", "sizes 2 2"],
    );
}

/// One program per shape (the reporter's generator): where the function is
/// declared and called, by the kind of array, and element types.
#[test]
fn call_shapes() {
    let shapes: &[(&str, usize, &str)] = &[
        (
            "B-D-int",
            3,
            r##"
module top;

  typedef int coll_t[];
  class k;
    function coll_t f(); coll_t c; c = new[3]; c[0] = 11; c[1] = 22; c[2] = 33; return c; endfunction
    function void run();
      coll_t v;
      string a, b;
      int m0, m1, m2, miss;
      m0 = 11; m1 = 22; m2 = 33; miss = 99;
      v = f();
      a = $sformatf("%p", f());
      b = $sformatf("%p", v);
      if (a == b) $display("PASS B-D-int p"); else $display("FAIL B-D-int p: call prints %s, variable prints %s", a, b);
      if ((m0 inside {f()}) && (m1 inside {f()}) && (m2 inside {f()})) $display("PASS B-D-int in-hit");
      else $display("FAIL B-D-int in-hit");
      if (!(miss inside {f()})) $display("PASS B-D-int in-miss"); else $display("FAIL B-D-int in-miss");
    endfunction
  endclass
  k h = new;
  initial begin h.run(); $finish; end
endmodule
"##,
        ),
        (
            "M-A-enum",
            3,
            r##"
module top;
  typedef enum {RED, GREEN, BLUE, WHITE} color_t;
  typedef color_t coll_t[int];
  function automatic coll_t f(); coll_t c; c[10] = RED; c[20] = GREEN; c[30] = BLUE; return c; endfunction
  initial begin
      coll_t v;
      string a, b;
      color_t m0, m1, m2, miss;
      m0 = RED; m1 = GREEN; m2 = BLUE; miss = WHITE;
      v = f();
      a = $sformatf("%p", f());
      b = $sformatf("%p", v);
      if (a == b) $display("PASS M-A-enum p"); else $display("FAIL M-A-enum p: call prints %s, variable prints %s", a, b);
      if ((m0 inside {f()}) && (m1 inside {f()}) && (m2 inside {f()})) $display("PASS M-A-enum in-hit");
      else $display("FAIL M-A-enum in-hit");
      if (!(miss inside {f()})) $display("PASS M-A-enum in-miss"); else $display("FAIL M-A-enum in-miss");
    $finish;
  end
endmodule
"##,
        ),
        (
            "M-D-logic4",
            1,
            r##"
module top;

  typedef logic [3:0] coll_t[];
  function automatic coll_t f(); coll_t c; c = new[3]; c[0] = 4'b1010; c[1] = 4'h3; c[2] = 4'hf; return c; endfunction
  initial begin
      coll_t v;
      string a, b;
      logic [3:0] m0, m1, m2, miss;
      m0 = 4'b1010; m1 = 4'h3; m2 = 4'hf; miss = 4'h7;
      v = f();
      a = $sformatf("%p", f());
      b = $sformatf("%p", v);
      if (a == b) $display("PASS M-D-logic4 p"); else $display("FAIL M-D-logic4 p: call prints %s, variable prints %s", a, b);
    $finish;
  end
endmodule
"##,
        ),
        (
            "M-F-real",
            1,
            r##"
module top;

  typedef real coll_t[3];
  function automatic coll_t f(); coll_t c; c[0] = 1.5; c[1] = 2.25; c[2] = -3.0; return c; endfunction
  initial begin
      coll_t v;
      string a, b;
      real m0, m1, m2, miss;
      m0 = 1.5; m1 = 2.25; m2 = -3.0; miss = 9.5;
      v = f();
      a = $sformatf("%p", f());
      b = $sformatf("%p", v);
      if (a == b) $display("PASS M-F-real p"); else $display("FAIL M-F-real p: call prints %s, variable prints %s", a, b);
    $finish;
  end
endmodule
"##,
        ),
        (
            "M-N-string",
            3,
            r##"
module top;

  typedef string coll_t[string];
  function automatic coll_t f(); coll_t c; c["k1"] = "x"; c["k2"] = "yz"; c["k3"] = "abc"; return c; endfunction
  initial begin
      coll_t v;
      string a, b;
      string m0, m1, m2, miss;
      m0 = "x"; m1 = "yz"; m2 = "abc"; miss = "none";
      v = f();
      a = $sformatf("%p", f());
      b = $sformatf("%p", v);
      if (a == b) $display("PASS M-N-string p"); else $display("FAIL M-N-string p: call prints %s, variable prints %s", a, b);
      if ((m0 inside {f()}) && (m1 inside {f()}) && (m2 inside {f()})) $display("PASS M-N-string in-hit");
      else $display("FAIL M-N-string in-hit");
      if (!(miss inside {f()})) $display("PASS M-N-string in-miss"); else $display("FAIL M-N-string in-miss");
    $finish;
  end
endmodule
"##,
        ),
        (
            "M-Q-pstruct",
            3,
            r##"
module top;
  typedef struct packed {bit [3:0] hi; bit [3:0] lo;} nib_t;
  typedef nib_t coll_t[$];
  function automatic coll_t f(); coll_t c; c.push_back(8'h1f); c.push_back(8'ha5); c.push_back(8'h05); return c; endfunction
  initial begin
      coll_t v;
      string a, b;
      nib_t m0, m1, m2, miss;
      m0 = 8'h1f; m1 = 8'ha5; m2 = 8'h05; miss = 8'h77;
      v = f();
      a = $sformatf("%p", f());
      b = $sformatf("%p", v);
      if (a == b) $display("PASS M-Q-pstruct p"); else $display("FAIL M-Q-pstruct p: call prints %s, variable prints %s", a, b);
      if ((m0 inside {f()}) && (m1 inside {f()}) && (m2 inside {f()})) $display("PASS M-Q-pstruct in-hit");
      else $display("FAIL M-Q-pstruct in-hit");
      if (!(miss inside {f()})) $display("PASS M-Q-pstruct in-miss"); else $display("FAIL M-Q-pstruct in-miss");
    $finish;
  end
endmodule
"##,
        ),
        (
            "P-N-int",
            3,
            r##"
package pk;

  typedef int coll_t[string];
  function automatic coll_t f(); coll_t c; c["k1"] = 11; c["k2"] = 22; c["k3"] = 33; return c; endfunction
endpackage
module top;
  initial begin
      pk::coll_t v;
      string a, b;
      int m0, m1, m2, miss;
      m0 = 11; m1 = 22; m2 = 33; miss = 99;
      v = pk::f();
      a = $sformatf("%p", pk::f());
      b = $sformatf("%p", v);
      if (a == b) $display("PASS P-N-int p"); else $display("FAIL P-N-int p: call prints %s, variable prints %s", a, b);
      if ((m0 inside {pk::f()}) && (m1 inside {pk::f()}) && (m2 inside {pk::f()})) $display("PASS P-N-int in-hit");
      else $display("FAIL P-N-int in-hit");
      if (!(miss inside {pk::f()})) $display("PASS P-N-int in-miss"); else $display("FAIL P-N-int in-miss");
    $finish;
  end
endmodule
"##,
        ),
        (
            "S-Q-int",
            3,
            r##"
module top;

  typedef int coll_t[$];
  class k;
    static function coll_t f(); coll_t c; c.push_back(11); c.push_back(22); c.push_back(33); return c; endfunction
  endclass
  k h = new;
  initial begin
      coll_t v;
      string a, b;
      int m0, m1, m2, miss;
      m0 = 11; m1 = 22; m2 = 33; miss = 99;
      v = k::f();
      a = $sformatf("%p", k::f());
      b = $sformatf("%p", v);
      if (a == b) $display("PASS S-Q-int p"); else $display("FAIL S-Q-int p: call prints %s, variable prints %s", a, b);
      if ((m0 inside {k::f()}) && (m1 inside {k::f()}) && (m2 inside {k::f()})) $display("PASS S-Q-int in-hit");
      else $display("FAIL S-Q-int in-hit");
      if (!(miss inside {k::f()})) $display("PASS S-Q-int in-miss"); else $display("FAIL S-Q-int in-miss");
    $finish;
  end
endmodule
"##,
        ),
        (
            "T-D-int",
            3,
            r##"
module top;

  typedef int coll_t[];
  class k;
    function coll_t f(); coll_t c; c = new[3]; c[0] = 11; c[1] = 22; c[2] = 33; return c; endfunction
    function void run();
      coll_t v;
      string a, b;
      int m0, m1, m2, miss;
      m0 = 11; m1 = 22; m2 = 33; miss = 99;
      v = this.f();
      a = $sformatf("%p", this.f());
      b = $sformatf("%p", v);
      if (a == b) $display("PASS T-D-int p"); else $display("FAIL T-D-int p: call prints %s, variable prints %s", a, b);
      if ((m0 inside {this.f()}) && (m1 inside {this.f()}) && (m2 inside {this.f()})) $display("PASS T-D-int in-hit");
      else $display("FAIL T-D-int in-hit");
      if (!(miss inside {this.f()})) $display("PASS T-D-int in-miss"); else $display("FAIL T-D-int in-miss");
    endfunction
  endclass
  k h = new;
  initial begin
    h.run();
    $finish;
  end
endmodule
"##,
        ),
        (
            "V-F-int",
            3,
            r##"
module top;

  typedef int coll_t[3];
  virtual class base;
    pure virtual function coll_t f();
  endclass
  class k extends base;
    virtual function coll_t f(); coll_t c; c[0] = 11; c[1] = 22; c[2] = 33; return c; endfunction
  endclass
  k impl = new;
  base h = impl;
  initial begin
      coll_t v;
      string a, b;
      int m0, m1, m2, miss;
      m0 = 11; m1 = 22; m2 = 33; miss = 99;
      v = h.f();
      a = $sformatf("%p", h.f());
      b = $sformatf("%p", v);
      if (a == b) $display("PASS V-F-int p"); else $display("FAIL V-F-int p: call prints %s, variable prints %s", a, b);
      if ((m0 inside {h.f()}) && (m1 inside {h.f()}) && (m2 inside {h.f()})) $display("PASS V-F-int in-hit");
      else $display("FAIL V-F-int in-hit");
      if (!(miss inside {h.f()})) $display("PASS V-F-int in-miss"); else $display("FAIL V-F-int in-miss");
    $finish;
  end
endmodule
"##,
        ),
        (
            "W-A-int",
            3,
            r##"
package pk;

  typedef int coll_t[int];
  function automatic coll_t f(); coll_t c; c[10] = 11; c[20] = 22; c[30] = 33; return c; endfunction
endpackage
module top;
  import pk::*;
  initial begin
      coll_t v;
      string a, b;
      int m0, m1, m2, miss;
      m0 = 11; m1 = 22; m2 = 33; miss = 99;
      v = f();
      a = $sformatf("%p", f());
      b = $sformatf("%p", v);
      if (a == b) $display("PASS W-A-int p"); else $display("FAIL W-A-int p: call prints %s, variable prints %s", a, b);
      if ((m0 inside {f()}) && (m1 inside {f()}) && (m2 inside {f()})) $display("PASS W-A-int in-hit");
      else $display("FAIL W-A-int in-hit");
      if (!(miss inside {f()})) $display("PASS W-A-int in-miss"); else $display("FAIL W-A-int in-miss");
    $finish;
  end
endmodule
"##,
        ),
        (
            "Y-Q-int",
            3,
            r##"
module top;

  typedef int coll_t[$];
  class k;
    extern function coll_t f();
    function void run();
      coll_t v;
      string a, b;
      int m0, m1, m2, miss;
      m0 = 11; m1 = 22; m2 = 33; miss = 99;
      v = f();
      a = $sformatf("%p", f());
      b = $sformatf("%p", v);
      if (a == b) $display("PASS Y-Q-int p"); else $display("FAIL Y-Q-int p: call prints %s, variable prints %s", a, b);
      if ((m0 inside {f()}) && (m1 inside {f()}) && (m2 inside {f()})) $display("PASS Y-Q-int in-hit");
      else $display("FAIL Y-Q-int in-hit");
      if (!(miss inside {f()})) $display("PASS Y-Q-int in-miss"); else $display("FAIL Y-Q-int in-miss");
    endfunction
  endclass
  function coll_t k::f(); coll_t c; c.push_back(11); c.push_back(22); c.push_back(33); return c; endfunction
  k h = new;
  initial begin h.run(); $finish; end
endmodule
"##,
        ),
    ];
    for (id, want, src) in shapes {
        let msgs = lines(src);
        let pass = msgs
            .iter()
            .filter(|m| m.starts_with(&format!("PASS {id}")))
            .count();
        assert_eq!(pass, *want, "{id}: {msgs:#?}");
        assert!(
            !msgs.iter().any(|m| m.starts_with("FAIL")),
            "{id}: {msgs:#?}"
        );
    }
}

/// A returned array in an unpacked array concatenation, `$size`, `case
/// inside` and parentheses.
#[test]
fn returned_array_in_other_whole_value_uses() {
    let msgs = lines(
        r##"
module top;
  typedef int q_t[$];
  function automatic q_t f(); return '{1, 2, 4}; endfunction
  initial begin
    q_t q;
    int n;
    q = {f(), 5};
    $display("concat %p", q);
    $display("size %0d sum %0d", $size(f()), f().sum());
    case (4) inside
      f(): $display("case inside hit");
      default: $display("case inside miss");
    endcase
    $display("paren %p", (f()));
    $display("p d %p %0d", f(), 7);
    n = 0; foreach (q[i]) if (q[i] inside {f()}) n++;
    $display("n %0d", n);
  end
endmodule
"##,
    );
    expect(
        &msgs,
        &[
            "concat '{1, 2, 4, 5}",
            "size 3 sum 7",
            "case inside hit",
            "paren '{1, 2, 4}",
            "p d '{1, 2, 4} 7",
            "n 3",
        ],
    );
}

/// `pkg::f()`, a static `C::f()` and a `pure virtual` method through its base
/// handle: `size()`, `inside`, `%p` and `case inside`.
#[test]
fn package_static_and_prototype_calls() {
    let msgs = lines(
        r##"
package pk;
  typedef int q_t[$];
  function automatic q_t f(); return '{1, 2, 4}; endfunction
endpackage
module top;
  import pk::q_t;
  virtual class shape;
    pure virtual function q_t sides();
  endclass
  class square extends shape;
    virtual function q_t sides(); return '{1, 1, 1, 1}; endfunction
  endclass
  class k;
    static function q_t g(); return '{7, 8}; endfunction
  endclass
  initial begin
    square sq = new;
    shape s = sq;
    int aa[string];
    $display("sizes %0d %0d %0d", pk::f().size(), k::g().size(), s.sides().size());
    $display("in %0d %0d %0d", 4 inside {pk::f()}, 8 inside {k::g()}, 1 inside {s.sides()});
    $display("p %p %p", k::g(), s.sides());
    case (8) inside k::g(): $display("case hit"); default: $display("case miss"); endcase
    aa["x"] = 3;
    case (3) inside aa: $display("case aa hit"); default: $display("case aa miss"); endcase
  end
endmodule
"##,
    );
    expect(
        &msgs,
        &[
            "sizes 3 2 4",
            "in 1 1 1",
            "p '{7, 8} '{1, 1, 1, 1}",
            "case hit",
            "case aa hit",
        ],
    );
}

/// §11.4.13: an associative array named by itself in an `inside` set
/// contributes its elements (it read as a scalar).
#[test]
fn associative_array_variable_in_inside_set() {
    let msgs = lines(
        r##"
module top;
  int ma[string];
  int mi[int];
  bit r1, r2;
  initial begin
    ma["y"] = 5; mi[7] = 6;
    r1 = 5 inside {ma}; r2 = 6 inside {mi};
    $display("in %0d %0d", r1, r2);
    $display("in2 %0d %0d", 5 inside {ma}, 6 inside {mi});
  end
endmodule
"##,
    );
    expect(&msgs, &["in 1 1", "in2 1 1"]);
}
