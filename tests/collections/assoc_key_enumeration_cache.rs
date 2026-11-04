//! §7.9.2-§7.9.8: `num`, `first`, `next`, `last` and `prev` on an associative
//! array see the CURRENT key set. The sorted key list is cached between
//! calls; every insert or delete (including of a class property's array, of a
//! multi-index array's row, and of a key re-added after deletion) must be
//! seen by the next traversal.

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn traversal_sees_inserts_and_deletes_between_calls() {
    let got = lines(
        r#"
class store;
  int aa[int];
  function string walk();
    int k; string s = "";
    if (aa.first(k)) do s = {s, $sformatf("%0d,", k)}; while (aa.next(k));
    return s;
  endfunction
endclass
module top;
  int aa[int];
  int mi[int][string];
  function automatic string walk_aa();
    int k; string s = "";
    if (aa.first(k)) do s = {s, $sformatf("%0d,", k)}; while (aa.next(k));
    return s;
  endfunction
  initial begin
    store h = new();
    int k; string s;
    aa[5] = 1; aa[1] = 1; aa[9] = 1;
    $display("T|a %0d %s", aa.num(), walk_aa());
    aa.delete(5); aa[3] = 1;
    $display("T|b %0d %s", aa.num(), walk_aa());
    // delete during traversal: next() from a removed key ends the walk
    k = 1; void'(aa.next(k)); aa.delete(3);
    $display("T|c %0d %0d %s", k, aa.num(), walk_aa());
    aa.delete(); aa[7] = 1;
    $display("T|d %0d %s", aa.num(), walk_aa());
    void'(aa.last(k)); $display("T|e last=%0d", k);
    aa[2] = 1; void'(aa.prev(k)); $display("T|f prev=%0d", k);
    // through a handle
    h.aa[4] = 1; h.aa[8] = 1;
    $display("T|g %0d %s", h.aa.num(), h.walk());
    h.aa.delete(4); h.aa[6] = 1;
    $display("T|h %0d %s", h.aa.num(), h.walk());
    // rows of a multi-index array
    mi[2]["x"] = 1; mi[1]["y"] = 1;
    $display("T|i %0d", mi.num());
    mi[3]["z"] = 1; mi.delete(1);
    s = ""; if (mi.first(k)) do s = {s, $sformatf("%0d,", k)}; while (mi.next(k));
    $display("T|j %0d %s", mi.num(), s);
  end
endmodule
"#,
    );
    assert_eq!(
        got,
        [
            "T|a 3 1,5,9,",
            "T|b 3 1,3,9,",
            "T|c 3 2 1,9,",
            "T|d 1 7,",
            "T|e last=7",
            "T|f prev=2",
            "T|g 2 4,8,",
            "T|h 2 6,8,",
            "T|i 2",
            "T|j 2 2,3,",
        ]
    );
}
