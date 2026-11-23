//! §13.4: a compiled method body storing a collection-returning call into a
//! bare local queue / dynamic array (`q = f(..)`), and a compiled callee
//! handing back a `return '{...}` literal. Each test runs at tier 0 (a
//! child process compiles every method on its first call) and again at the
//! default tier, where the 1500-iteration loops cross the compile threshold.
//! Expected values come from the reference simulator.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 10).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

/// A compiled callee's queue literal must reach both an interpreted caller
/// and a compiled caller, including the empty literal.
#[test]
fn compiled_literal_return_reaches_callers() {
    let _ = super::compiled_method_test_env::eager();
    let src = r#"
module top;
class S;
  typedef int q_t[$];
  int base;
  function q_t mk(int a); return '{a, a + base, a + 2*base}; endfunction
  function q_t none(); return '{}; endfunction
  function int use_mk(int a);
    automatic q_t q;
    automatic int s;
    q = mk(a);
    s = q.size() * 1000;
    foreach (q[i]) s += q[i];
    q = none();
    return s + q.size();
  endfunction
endclass
function automatic S::q_t ff(int a); return '{a, a*2}; endfunction
function automatic int use_ff(int a);
  automatic S::q_t q;
  q = ff(a);
  return q.size() * 1000 + q[0] + q[1];
endfunction
initial begin
  automatic S s = new;
  automatic S::q_t r;
  automatic int acc1 = 0, acc2 = 0, acc3 = 0, acc4 = 0;
  s.base = 10;
  for (int i = 0; i < 1500; i++) begin
    r = s.mk(i);
    acc1 += r.size() + r[2];
    acc2 += s.use_mk(i);
    r = ff(i);
    acc3 += r.size() + r[1];
    acc4 += use_ff(i);
    if (i == 1200) $display("T|i=%0d r=%p mk=%0d", i, r, s.use_mk(i));
  end
  $display("T|acc1=%0d acc2=%0d acc3=%0d acc4=%0d", acc1, acc2, acc3, acc4);
end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "T|i=1200 r='{1200, 2400} mk=6630",
            "T|acc1=1158750 acc2=7917750 acc3=2251500 acc4=6372750",
        ]
    );
}

/// A compiled caller storing a compiled callee's result keeps running the
/// statements after the store.
#[test]
fn compiled_store_runs_rest_of_body() {
    let _ = super::compiled_method_test_env::eager();
    let src = r#"
module top;
class S;
  typedef int q_t[$];
  int m_q[$];
  int after;
  function q_t lit(int a); return '{a, a + 1, a + 2}; endfunction
  function q_t getq(); return m_q; endfunction
  function int f4(int a);
    automatic int q[$];
    q = lit(a);
    after++;
    return q.size() * 100 + q[q.size()-1];
  endfunction
  function int f5(int a);
    automatic int q[$];
    q = getq();
    after++;
    q = lit(a);
    after++;
    return q.size() * 100 + q[0];
  endfunction
  function int g2(S h);
    automatic int d[];
    d = h.getq();
    after++;
    return d.size() * 100 + d[d.size()-1];
  endfunction
endclass
initial begin
  automatic S s = new;
  automatic longint a4 = 0, a5 = 0, a6 = 0;
  s.m_q = '{1, 3, 5};
  for (int i = 0; i < 1500; i++) begin
    a4 += s.f4(i); a5 += s.f5(i); a6 += s.g2(s);
    if (i == 1400) $display("T|i=%0d f4=%0d f5=%0d after=%0d", i, s.f4(i), s.f5(i), s.after);
  end
  $display("T|a4=%0d a5=%0d a6=%0d after=%0d", a4, a5, a6, s.after);
end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "T|i=1400 f4=1702 f5=1700 after=5607",
            "T|a4=1577250 a5=1574250 a6=457500 after=6003",
        ]
    );
}

#[test]
fn return_pattern_width_follows_the_current_function_and_method() {
    if !super::compiled_method_test_env::policies(&[("0", "1000"), ("1", "0")]) {
        return;
    }
    let src = r#"
module top;
  typedef byte byte_q[$];
  typedef int int_q[$];
  function automatic byte_q bytes(); return '{8'h34}; endfunction
  function automatic int_q words(); return '{32'h1234, 1'b1 + 1'b1}; endfunction
  function automatic int first_byte(); byte_q q; q = {bytes()}; return q[0]; endfunction
  function automatic int_q nested(); return '{32'h3456, first_byte(), 32'h789a}; endfunction
  class C;
    function int_q more(); return '{32'h5678, 1'b1 + 1'b1}; endfunction
  endclass
  initial begin
    C c = new;
    byte b[$];
    int w[$];
    b = {bytes()};
    w = {words()};
    $display("T|free=%h,%0d", w[0],w[1]);
    b = {bytes()};
    w = {c.more()};
    $display("T|method=%h,%0d", w[0],w[1]);
    w = {nested()};
    $display("T|nested=%h,%0d,%h", w[0],w[1],w[2]);
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "T|free=00001234,2",
            "T|method=00005678,2",
            "T|nested=00003456,52,0000789a"
        ]
    );
}

#[test]
fn returned_concatenations_convert_each_element_before_the_caller_consumes_it() {
    if !super::compiled_method_test_env::policies(&[("0", "1000"), ("1", "0")]) {
        return;
    }
    let src = r#"
module top;
  typedef bit [7:0] q8[$];
  typedef byte sq8[$];
  typedef int q32[$];
  function automatic q8 trunc(); return {32'h1234, 8'hxx}; endfunction
  function automatic sq8 sign_ext(); return {32'hffff00ff}; endfunction
  function automatic q32 full(); return '{32'h3456}; endfunction
  initial begin
    int q[$];
    for (int i=0; i<2; i++) begin
      q={full(),trunc(),sign_ext()};
      $display("T|rows=%h,%h,%h,%h",q[0],q[1],q[2],q[3]);
    end
  end
endmodule
"#;
    let expected = "T|rows=00003456,00000034,00000000,ffffffff";
    assert_eq!(lines(src), [expected, expected]);
}
