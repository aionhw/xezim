//! IEEE 1800-2023 §§8.8, 8.25, 13.5: constructor actuals use the caller's scope.

use xezim::simulate;

fn check(src: &str, expected: &[&str]) {
    let sim = simulate(src, 10).expect("simulation");
    let output: Vec<_> = sim.output.iter().map(|o| o.message.trim()).collect();
    for line in expected {
        assert!(output.contains(line), "missing {line:?} in {output:?}");
    }
}

#[test]
fn actuals_use_caller_and_defaults_use_target() {
    if !super::compiled_method_test_env::policies(&[("0", "0"), ("1", "0")]) {
        return;
    }
    check(
        r#"class Target #(int N=2);
 int value;
 int default_value;
 int initialized=N;
 int body_value;
 static function int default_n(); return N; endfunction
 function new(int v=N, int d=default_n());
  value=v; default_value=d; body_value=N;
 endfunction
endclass
class Maker #(int N=7);
 typedef Target#(3) target_t;
 static function int make();
  target_t t;
  target_t d;
  Target plain;
  target_t direct;
  target_t i=target_t::new(N);
  t=target_t::new(N);
  d=target_t::new;
  plain=Target::new(N);
  direct=Target#(3)::new(N);
  $display("T|direct=%0d/%0d",direct.value,direct.default_value);
  $display("T|plain=%0d/%0d/%0d/%0d",plain.value,plain.default_value,plain.initialized,plain.body_value);
  $display("T|caller=%0d init=%0d default=%0d field=%0d body=%0d bare=%0d", t.value,i.value,t.default_value,t.initialized,t.body_value,d.value);
  return N;
 endfunction
endclass
module top;
 typedef Target#(5) module_t;
 module_t m;
 initial begin
  int restored;
  restored=Maker#(7)::make();
  m=module_t::new();
  $display("T|restored=%0d module=%0d/%0d/%0d",restored,m.value,m.initialized,m.body_value);
 end
endmodule
"#,
        &[
            "T|caller=7 init=7 default=3 field=3 body=3 bare=3",
            "T|restored=7 module=5/5/5",
            "T|plain=7/2/2/2",
            "T|direct=7/3",
        ],
    );
}

#[test]
fn actuals_keep_nested_calls_width_and_reference_writeback() {
    if !super::compiled_method_test_env::policies(&[("0", "0"), ("1", "0")]) {
        return;
    }
    check(
        r#"class Target #(int N=3);
 int value;
 bit [63:0] wide;
 function new(int v, bit [63:0] w, ref int count);
  value=v; wide=w; count+=N;
 endfunction
endclass
class Maker #(int N=7);
 typedef Target#(3) target_t;
 static int calls;
 static function int next_value(); calls++; return N; endfunction
 static function void run();
  target_t t;
  int count=10;
  t=target_t::new(N+next_value(), '1, count);
  $display("T|value=%0d wide=%016h count=%0d calls=%0d caller=%0d",t.value,t.wide,count,calls,N);
 endfunction
endclass
module top;
 initial Maker#(7)::run();
endmodule
"#,
        &["T|value=14 wide=ffffffffffffffff count=13 calls=1 caller=7"],
    );
}

#[test]
fn qualified_constructors_keep_their_class() {
    if !super::compiled_method_test_env::policies(&[("0", "0"), ("1", "0")]) {
        return;
    }
    check(
        r#"
package p;
 class C; int id=11; endclass
endpackage
package q;
 class C; int id=22; endclass
endpackage
class Base;
 virtual function int id(); return 1; endfunction
endclass
class Outer;
 class Inner extends Base;
  virtual function int id(); return 31; endfunction
 endclass
endclass
class Maker;
 static function void run();
  p::C a=p::C::new;
  q::C b=q::C::new();
  Base c;
  c=Outer::Inner::new();
  $display("T|qualified=%0d/%0d/%0d",a.id,b.id,c.id());
 endfunction
endclass
module top;
 initial Maker::run();
endmodule
"#,
        &["T|qualified=11/22/31"],
    );
}
