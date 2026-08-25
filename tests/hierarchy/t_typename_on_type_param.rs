//! `$typename(T)` where `T` is a TYPE PARAMETER of the current class
//! specialization must return the CONCRETE bound type, not `logic`.
//!
//! Inside a parameterized class's method, `$typename(T)` was falling
//! through the `$typename` argument dispatch: `T` is not a builtin keyword,
//! not a class key, not a specialization node, and not a signal — so the
//! handler's final fallback returned the literal `"logic"` for every bound
//! (09callbacks/90Mantis/6033 logged `Test: logic` for all three of its
//! `tester#(...)` specializations). Resolve the bare name as a type
//! parameter of the active specialization and format its concrete binding:
//! a builtin bare (`string`, `int`) or a class as `class <name>` /
//! `class <name> #(<args>)`. Verified byte-for-byte against a commercial
//! simulator: `typename=string / int / class Base`.

use xezim::simulate_multi;

#[test]
fn t_typename_on_type_param() {
    let src = r#"
class Base;
endclass
class Inner #(int N=0);
endclass
class UsesType #(type T=Base, type U=Inner#(7));
  static task show();
    $display("TN:%s:%s", $typename(T), $typename(U));
  endtask
endclass
module top;
  initial begin
    UsesType#(string)::show();              // T=string
    UsesType#(int, Inner#(5))::show();      // T=int, U=Inner#(5)
    UsesType#(Base)::show();                // T=class Base
    $finish;
  end
endmodule
"#;
    let out: Vec<String> = simulate_multi(
        &[src.to_string()],
        1000,
        Some("top"),
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &[],
        1,
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
        None,
    )
    .expect("sim")
    .output
    .iter()
    .map(|o| o.message.clone())
    .collect();
    let text = out.join("\n");
    // Explicit type-param bindings must render concretely (not `logic`):
    assert!(text.contains("TN:string:"), "T=string got: {:?}", out);
    assert!(text.contains("TN:int:class Inner #(5)"), "U=Inner#(5) got: {:?}", out);
    assert!(text.contains("TN:class Base:"), "T=class Base got: {:?}", out);
    // The old defect printed `logic` for every type param:
    assert!(!text.contains("TN:logic"), "type param rendered as logic: {:?}", out);
}