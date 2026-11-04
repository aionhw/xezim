//! IEEE 1800-2023 §13.5.1, §11.6.1: a compiled `super.m(...)` evaluates
//! each argument at the PARENT's formal width, as the call goes to the
//! parent's `m`; it took the widths of the derived `m` that hides it, so
//! `(a + b) >> 1` on 8-bit operands gave 150 (#593, reported by akashlevy,
//! with the patch applied here).
use xezim::simulate;

#[test]
fn compiled_super_call_takes_the_parents_widths() {
    let out: Vec<String> = simulate(
        r#"class P;
  function int m(logic [7:0] v);
    return v;
  endfunction
endclass
class D extends P;
  function int m(logic [31:0] v);
    return v;
  endfunction
  function int call(logic [7:0] a, logic [7:0] b);
    return super.m((a + b) >> 1);
  endfunction
endclass
module top;
  initial begin
    D d = new;
    int r;
    for (int i = 0; i < 1500; i++) r = d.call(200, 100);
    $display("r=%0d expect 22", r);
    $finish;
  end
endmodule

"#,
        1000,
    )
    .expect("simulate")
    .output
    .iter()
    .map(|l| l.message.clone())
    .filter(|m| m.contains("expect"))
    .collect();
    assert_eq!(out, ["r=22 expect 22"]);
}
