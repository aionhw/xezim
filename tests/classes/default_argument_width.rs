use xezim::simulate;

fn check(source: &str) {
    let sim = simulate(source, 100).expect("simulation");
    let out = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(out.contains("PASS") && !out.contains("Fatal"), "{out}");
}

#[test]
fn constructor_default_bound_uses_formal_width() {
    for range in ["63:0", "64-1:0"] {
        check(&format!(
            r#"
class memory_t;
 bit [63:0] max_addr;
 extern function new(bit [{range}] bound=((1<<64)-1));
endclass
function memory_t::new(bit [{range}] bound=((1<<64)-1));
 max_addr=bound;
endfunction
module top;
 initial begin
  memory_t m=new();
  if(m.max_addr!==64'hffffffffffffffff) $fatal(1,"default bound=%h",m.max_addr);
  $display("PASS"); $finish;
 end
endmodule
"#
        ));
    }
}

#[test]
fn function_and_task_defaults_preserve_carry() {
    check(
        r#"
class C;
 function bit [8:0] f(bit [7:0] a=255, bit [8:0] b=a+8'd1);
  return b;
 endfunction
 task t(bit [63:0] bound=(1<<64)-1);
  if(bound!==64'hffffffffffffffff) $fatal(1,"method task bound");
 endtask
endclass
module top;
 function automatic bit [8:0] f(bit [7:0] a=255, bit [8:0] b=a+8'd1);
  return b;
 endfunction
 task automatic t(bit [63:0] bound=(1<<64)-1);
  if(bound!==64'hffffffffffffffff) $fatal(1,"task bound");
 endtask
 initial begin
  C c=new();
  if(f()!=256 || c.f()!=256) $fatal(1,"default carry");
  t(); c.t();
  $display("PASS"); $finish;
 end
endmodule
"#,
    );
}

#[test]
fn parameterized_constructor_default_uses_callee_width() {
    check(
        r#"
class memory_t #(int W=64);
 bit [W-1:0] max_addr;
 function new(bit [W-1:0] bound=((1<<W)-1));
  max_addr=bound;
 endfunction
endclass
module top;
 initial begin
  memory_t #(64) wide=new();
  memory_t #(40) narrow=new();
  if(wide.max_addr!==64'hffffffffffffffff) $fatal(1,"wide default");
  if(narrow.max_addr!==40'hffffffffff) $fatal(1,"narrow default");
  $display("PASS"); $finish;
 end
endmodule
"#,
    );
}

#[test]
fn explicit_constructor_bound_uses_constant_expression_formal_width() {
    check(
        r#"
class cfg_t;
 int addr_width=64;
endclass
class memory_t;
 bit [63:0] max_addr;
 function new(bit [64-1:0] bound);
  max_addr=bound;
 endfunction
endclass
module top;
 initial begin
  cfg_t cfg=new();
  memory_t mem=new((1<<cfg.addr_width)-1);
  if(mem.max_addr!==64'hffffffffffffffff) $fatal(1,"explicit bound=%h",mem.max_addr);
  $display("PASS"); $finish;
 end
endmodule
"#,
    );
}

/// §8.25, §13.5: a class parameter named like a package parameter sizes the
/// formal by the class's own value, for explicit and default actuals.
#[test]
fn class_parameter_bound_ignores_same_named_package_parameter() {
    check(include_str!("../fixtures/class_param_formal_name_clash.sv"));
}

#[test]
fn wide_128_bit_constructor_arguments() {
    for range in ["127:0", "128-1:0"] {
        check(&format!(
            r#"
class C;
 bit [127:0] bound;
 function new(bit [{range}] value=(1<<128)-1);
  bound=value;
 endfunction
endclass
module top;
 initial begin
  C defaults=new();
  C explicit_arg=new((1<<128)-1);
  C high_bit=new(1<<100);
  if(defaults.bound!==128'hffffffffffffffffffffffffffffffff) $fatal(1,"default width");
  if(explicit_arg.bound!==128'hffffffffffffffffffffffffffffffff) $fatal(1,"actual width");
  if(high_bit.bound!==(128'b1<<100)) $fatal(1,"high bit lost");
  $display("PASS"); $finish;
 end
endmodule
"#
        ));
    }
}
