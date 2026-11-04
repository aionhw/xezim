//! §8.27 / §6.16: when a class is first named through `typedef class C;`, a
//! `string` property read through an element of a `C` queue is still a
//! string, so `q[i].label == ""` compares text and returns 1 or 0.
#[test]
fn forward_class_string_member_compares_as_text() {
    let result = xezim::simulate(
        r#"
typedef class item_c;
class list_c;
  string label = "own";
  item_c items[$];
  function void run();
    item_c a = new, b = new;
    b.label = "L1";
    items.push_back(a);
    items.push_back(b);
    foreach (items[i])
      $display("m %0d eq=%0d ne=%0d", i, items[i].label == "", items[i].label != "");
    foreach (items[i])
      if (items[i].label == "") items[i].has_label = 1'b0;
    $display("has %0d %0d", items[0].has_label, items[1].has_label);
  endfunction
  task later();
    #1;
    $display("t eq=%0d", this.items[0].label == "");
  endtask
endclass
class item_c;
  bit has_label = 1'b1;
  string label;
endclass
module t;
  initial begin
    list_c l = new;
    l.run();
    l.later();
    $display("top eq=%0d %0d", l.items[0].label == "", l.items[1].label == "L1");
    $finish;
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
    assert_eq!(
        lines,
        [
            "m 0 eq=1 ne=0",
            "m 1 eq=0 ne=1",
            "has 0 1",
            "t eq=1",
            "top eq=1 1"
        ]
    );
}
