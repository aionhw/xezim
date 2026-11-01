//! Review regression of #798: an integral value stored into a `real`
//! element of an associative array is self-determined before conversion,
//! not evaluated in the element's 64 bits (§11.6.1, §6.12.1), for module,
//! class-member and method-local arrays. Integral elements keep their width
//! as context. Expected values are the reference simulator's.

use xezim::simulate;

#[test]
fn a_real_element_store_converts_the_self_determined_value() {
    let src = r#"
class H;
  bit [71:0] ma[int];
  real mr[int];
  bit [7:0] x = 200, y = 100;
  function void f();
    real lr[int];
    ma[1] = (1 << 69) - 1;
    mr[1] = x + y;
    lr[1] = x + y;
    $display("T|ma %h mr %0.1f lr %0.1f", ma[1], mr[1], lr[1]);
  endfunction
endclass
module top;
  real ra[int];
  shortreal sra[int];
  bit [7:0] x = 200, y = 100;
  H h = new;
  initial begin
    ra[1] = x + y;
    ra[2] = 1 << 40;
    sra[1] = x + y;
    $display("T|ra %0.1f %0.1f sra %0.1f", ra[1], ra[2], sra[1]);
    h.f();
  end
endmodule
"#;
    let lines: Vec<String> = simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|ra 44.0 0.0 sra 44.0",
            "T|ma 1fffffffffffffffff mr 44.0 lr 44.0"
        ]
    );
}
