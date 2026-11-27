//! IEEE 1800-2023 §6.16.6 and §8.6 (#1639).

#[test]
fn indexed_class_compare_is_distinct_from_string_compare() {
    let src = r#"
class item;
  int id;
  function bit compare(item rhs); return rhs.id == id; endfunction
endclass
class child extends item; endclass
module top;
  initial begin
    child fixed_items[2];
    item dynamic_items[];
    item queue_items[$];
    string words[2];
    int index = 0;
    fixed_items[0] = new;
    fixed_items[1] = new;
    fixed_items[0].id = 3;
    fixed_items[1].id = 4;
    dynamic_items = new[2];
    dynamic_items[0] = fixed_items[0];
    dynamic_items[1] = fixed_items[1];
    queue_items.push_back(fixed_items[0]);
    queue_items.push_back(fixed_items[1]);
    words[0] = "a";
    words[1] = "b";
    $display("R %0b %0b %0b %0b %0b %0b %0b",
       fixed_items[index++].compare(fixed_items[0]), index == 1,
       dynamic_items[0].compare(dynamic_items[1]), dynamic_items[0].compare(dynamic_items[0]),
       queue_items[0].compare(queue_items[1]), queue_items[0].compare(queue_items[0]),
       words[0].compare(words[1]) < 0);
  end
endmodule
"#;
    let sim = xezim::simulate(src, 100).unwrap();
    assert!(
        sim.output
            .iter()
            .any(|o| o.message.trim() == "R 1 1 0 1 0 1 1"),
        "{:?}",
        sim.output.iter().map(|o| &o.message).collect::<Vec<_>>()
    );
}
