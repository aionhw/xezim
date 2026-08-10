//! §8.4 — reading an instance property through a null class handle in a
//! process body is a runtime fatal: the error is reported and the sim
//! terminates (the reference simulator aborts). Reference-validated (I17).

use xezim::simulate;

#[test]
fn null_property_read_is_fatal() {
    let src = r#"
class C;
  int v = 7;
endclass
module tb;
  C c; // null
  initial begin
    $display("T|pre");
    $display("T|v=%0d", c.v); // fatal here
    $display("T|post");
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(msgs.iter().any(|m| m.contains("null object dereference")), "error reported");
    assert!(msgs.iter().any(|m| m == "T|pre"));
    assert!(!msgs.iter().any(|m| m == "T|post"), "sim must stop at the deref");
}
