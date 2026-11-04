//! IEEE 1800-2017 §8.25: nested actuals belong to the lexical specialization.
use xezim::simulate;

#[test]
fn static_nested_construction_keeps_lexical_values_and_instance_arguments() {
    let source = r#"
class parent #(int P=0);
 int seen;
 function new();seen=P;endfunction
endclass
class child #(int N=1) extends parent #(N); endclass
class helper #(int N=7);
 static function int get();
  child #(N) x; child #(7) expected;
  x=new;
  return 1000*$cast(expected,x)+x.seen;
 endfunction
endclass
class caller #(int N=194);
 function int run();return helper#(7)::get();endfunction
 function int own();child #(N) x; x=new;return x.seen;endfunction
endclass
module top;
 caller#(194) c;
 initial begin
  c=new;
  $display("T|static|%0d|%0d|%0d",helper#(7)::get(),c.run(),c.own());
 end
endmodule
"#;
    let sim = simulate(source, 20).expect("nested specialization elaborates");
    let actual: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(actual, ["T|static|1007|1007|194"]);
}

#[test]
fn inherited_method_uses_its_declaring_specialization_when_leaf_reuses_parameter_name() {
    let source = r#"
class parent #(int P=0); int seen; function new();seen=P;endfunction endclass
class child #(int N=1) extends parent #(N); endclass
class base #(int N=7);
 function int get();child#(N) x;x=new;return x.seen;endfunction
endclass
class derived #(int N=194) extends base#(7); endclass
module top;
 derived#(194) d;
 initial begin d=new;$display("T|inherited|%0d",d.get());end
endmodule
"#;
    let sim = simulate(source, 20).expect("inherited specialization elaborates");
    let actual: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(actual, ["T|inherited|7"]);
}
