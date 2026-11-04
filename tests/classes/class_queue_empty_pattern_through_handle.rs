//! `o.q = '{}` through an object handle leaves a queue or dynamic array
//! property with no elements, own or inherited, of any element type, as it
//! does inside a method. `'{}` is an extension that a reference simulator also
//! accepts; IEEE 1800-2017 spells the empty value `{}` (§7.10).

#[test]
fn empty_pattern_through_a_handle_clears_the_property() {
    let out: Vec<String> = xezim::simulate(
        r#"
class b;
  string sq[$] = {"base"};
  int iq[$] = {7, 8};
endclass
class d extends b;
  string own[$] = {"own"};
  int oq[$] = {1, 2, 3};
  int arr[] = '{4, 5};
endclass
module t;
  initial begin
    d o = new();
    o.sq = '{}; o.iq = '{}; o.own = '{}; o.oq = '{}; o.arr = '{};
    $display("sq=%0d iq=%0d own=%0d oq=%0d arr=%0d",
      o.sq.size(), o.iq.size(), o.own.size(), o.oq.size(), o.arr.size());
    o.oq = '{9};
    $display("oq=%0d:%0d", o.oq.size(), o.oq[0]);
  end
endmodule
"#,
        100,
    )
    .expect("simulate failed")
    .output
    .iter()
    .map(|o| o.message.clone())
    .collect();
    for want in ["sq=0 iq=0 own=0 oq=0 arr=0", "oq=1:9"] {
        assert!(out.iter().any(|m| m == want), "missing {want:?}: {out:?}");
    }
}
