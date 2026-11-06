//! IEEE 1800-2023 §22.5.1: a macro invoked with more actual arguments than
//! it has formals is an error naming the macro (#888, the
//! `` `uvm_component_param_utils(T) `` shape with a second actual). A comma
//! inside parentheses, braces or a string is part of one actual, so
//! `` `m(cls#(8, int)) `` passes one argument, and an empty actual at a
//! comma is a supplied position. The reference simulator rejects the first
//! design and prints the others' lines.

use xezim::simulate;

fn out(src: &str) -> Vec<String> {
    let sim = simulate(src, 10).unwrap_or_else(|e| panic!("{e}\n{src}"));
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn an_excess_macro_actual_is_an_error_naming_the_macro() {
    let src = r#"
`define reg_utils(T) typedef T T``_t;
module top;
  `reg_utils(logic, extra_arg)
  initial $display("T|done");
endmodule
"#;
    let e = match simulate(src, 10) {
        Err(e) => e,
        Ok(_) => panic!("accepted two actuals for one formal"),
    };
    assert!(e.contains("reg_utils"), "{e}");
    assert!(
        e.contains("invoked with 2 arguments but only 1 are declared"),
        "{e}"
    );
}

#[test]
fn commas_inside_parentheses_and_braces_stay_in_one_actual() {
    let o = out(r#"
`define show(T) initial $display("T|%s", `"T`");
`define two(A, B) initial $display("T|a=%s b=%s", `"A`", `"B`");
class ag #(int W = 8, type I = int); endclass
module top;
  `show(ag#(8, int))
  `show({a, b})
  `two(x,)
  `two(,y)
endmodule
"#);
    assert_eq!(o, ["T|ag#(8, int)", "T|{a, b}", "T|a=x b=", "T|a= b=y"]);
}
