//! IEEE 1800-2023 §§8.3, 13.3, 13.4 and 23.6: shared callable syntax
//! must preserve per-instance static state, formal widths and class receivers.
use xezim::simulate;

#[test]
fn shared_callable_bodies_keep_instance_state_and_widths() {
    let source = r#"
module leaf #(parameter W=8)(output int result, output int bits_seen, output int method_result);
  function static int count();
    static int n=0;
    n++;
    return n;
  endfunction
  function automatic logic [W-1:0] add(input logic [W-1:0] x);
    return x+1;
  endfunction
  task automatic inspect(input logic [W-1:0] x, output int b);
    b=$bits(x);
  endtask
  class C;
    int n;
    function new(input int seed); n=seed; endfunction
    function int step(); n++; return n; endfunction
  endclass
  C obj;
  initial begin
    obj=new(W);
    result=count()*100+count()*10+int'(add('1));
    inspect('1,bits_seen);
    method_result=obj.step();
  end
endmodule
module top;
  int ra,rb,rc,wa,wb,wc,ma,mb,mc;
  leaf #(8) a(ra,wa,ma);
  leaf #(16) b(rb,wb,mb);
  leaf #(8) c(rc,wc,mc);
  initial begin
    #1;
    $display("T|results=%0d,%0d,%0d widths=%0d,%0d,%0d methods=%0d,%0d,%0d",ra,rb,rc,wa,wb,wc,ma,mb,mc);
    $finish;
  end
endmodule
"#;
    let result = simulate(source, 1000).expect("simulate shared callable bodies");
    let messages: Vec<_> = result
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(
        messages,
        ["T|results=120,120,120 widths=8,16,8 methods=9,17,9"]
    );
}

/// §8.25/§13.5: the existing per-origin method cache must retain dynamic
/// specialization widths even when its private body enables memoization.
#[test]
fn cached_generic_class_methods_keep_specialization_widths() {
    let result = simulate(r#"
class C #(parameter int W=8);
  bit [W-1:0] value;
  function void put(input bit [W-1:0] x); value=x; endfunction
  function int width_of(input bit [W-1:0] x); return $bits(x); endfunction
  function int high_bit(); return value >> (W-1); endfunction
endclass
module top;
  C#(8) a;
  C#(16) b;
  initial begin
    a=new; b=new;
    a.put(8'h81); b.put(16'h8001);
    $display("T|widths=%0d,%0d,%0d high=%0d,%0d,%0d", a.width_of('1),b.width_of('1),a.width_of('1),a.high_bit(),b.high_bit(),a.high_bit());
    $finish;
  end
endmodule
"#, 1000).expect("simulate generic method caches");
    let messages: Vec<_> = result
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(messages, ["T|widths=8,16,8 high=1,1,1"]);
}
