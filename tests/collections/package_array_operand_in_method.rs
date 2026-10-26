//! §11.4.13: an unpacked array in an `inside` set contributes its elements,
//! and §10.10: an unpacked array operand of an unpacked array concatenation
//! contributes all of its elements. Inside a method, `pkg::arr` is that
//! package array, not one packed value.
#[test]
fn package_array_operand_in_method() {
    let result = xezim::simulate(
        r#"
package p;
  typedef enum bit [1:0] {U = 2'b00, S = 2'b01, M = 2'b11} mode_t;
  mode_t modes[] = {M, U};
  int iq[$] = {3, 7};
endpackage
class gen;
  function string f();
    int cat[$];
    cat = {p::iq, 9};
    return $sformatf("%0d%0d%0d %0d%0d %p", p::M inside {p::modes}, p::S inside {p::modes},
                     p::U inside {p::modes}, 7 inside {p::iq}, 8 inside {1, p::iq}, cat);
  endfunction
endclass
module t;
  gen g;
  initial begin
    g = new();
    $display("%s", g.f());
  end
endmodule
"#,
        100_000,
    )
    .unwrap();
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(lines, ["101 10 '{3, 7, 9}"]);
}
