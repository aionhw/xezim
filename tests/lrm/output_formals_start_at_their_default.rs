//! §13.4.2, §13.5.1: an output formal is not copied in. In an automatic
//! subroutine, which includes every class method, it starts each call at its
//! type's default; a static subroutine's formal keeps its previous value.

use xezim::simulate;

/// §13.4.2: overlapping invocations observe the same static output cell.
#[test]
fn recursive_static_function_shares_its_output_during_the_call() {
    assert_eq!(
        lines(
            r#"
module top;
  int outer_value, inner_value, seen;
  function static void f(input bit inner, output int value);
    if (inner) begin
      seen = value;
      value = 9;
    end else begin
      value = 7;
      f(1, inner_value);
    end
  endfunction
  initial begin
    f(0, outer_value);
    $display("seen=%0d outer=%0d inner=%0d", seen, outer_value, inner_value);
  end
endmodule
"#
        ),
        ["seen=7 outer=9 inner=9"]
    );
}

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn function_and_method_outputs_start_at_their_default() {
    let src = r#"package p;
  function automatic void pkg_out(output bit [3:0] o, input bit set); if (set) o = 9; endfunction
endpackage
class C #(type T = bit);
  function void f(output bit [3:0] o, output logic [3:0] l, output T t, input bit set);
    if (set) begin o = 9; l = 9; t = 1; end
  endfunction
  static function void sf(output bit [3:0] o, input bit set); if (set) o = 9; endfunction
  task tk(output bit [3:0] o, input bit set); if (set) o = 9; endtask
  function void hit(int n, output bit h); if (n == 0) h = 1; endfunction
  function void scan(output bit [3:0] flags);
    bit u = 0;
    for (int i = 0; i < 4; i++) begin
      hit(i % 3, u);
      flags[i] = u;
    end
  endfunction
endclass
module top;
  function automatic void auto_out(output bit [3:0] o, input bit set); if (set) o = 9; endfunction
  function void static_out(output bit [3:0] o, input bit set); if (set) o = 9; endfunction
  function void count(output int o); o++; endfunction
  C #(logic) c = new;
  bit [3:0] a, b;
  int n;
  logic [3:0] l;
  logic t;
  initial begin
    a = 5; auto_out(a, 0);
    b = 5; p::pkg_out(b, 0);
    $display("function %0d %0d", a, b);
    a = 5; l = 5; t = 1; c.f(a, l, t, 0);
    $display("method %0d %b %b", a, l, t);
    a = 5; c.sf(a, 0);
    b = 5; c.tk(b, 0);
    $display("static method %0d task %0d", a, b);
    c.scan(a);
    $display("loop %b", a);
    a = 0; static_out(a, 1);
    b = 0; static_out(b, 0);
    for (int i = 0; i < 1500; i++) begin n = 0; count(n); end
    $display("static %0d %0d %0d", a, b, n);
    $finish;
  end
endmodule"#;
    assert_eq!(
        lines(src),
        [
            "function 0 0",
            "method 0 xxxx x",
            "static method 0 task 0",
            "loop 1001",
            "static 9 9 1500"
        ]
    );
}
