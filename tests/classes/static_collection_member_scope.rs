//! IEEE 1800-2023 §7.8, §7.10, §8.9, §8.23 (#1685).

#[test]
fn static_collection_elements_share_storage_through_class_and_handle() {
    let src = r#"
module top;
  typedef struct { string text; int count; } item_t;
  class registry;
    static item_t q[$];
    static item_t array[];
    static item_t lookup[string];
  endclass
  class derived extends registry; endclass
  task automatic inc(ref int n); n++; endtask
  initial begin
    registry obj = new;
    item_t item = '{"first", 1};
    registry::q.push_back(item);
    registry::q.push_back(item);
    registry::array = new[1];
    registry::array[0] = item;
    registry::lookup["key"] = item;
    derived::q[$].count += 6;
    inc(registry::q[0].count);
    registry::q[0].text = "other";
    registry::array[0].count = 9;
    registry::lookup["key"].text = "again";
    $display("R %s %0d %0d %0d %s", obj.q[0].text, obj.q[0].count,
             obj.q[1].count, obj.array[0].count, obj.lookup["key"].text);
  end
endmodule
"#;
    let sim = xezim::simulate(src, 100).unwrap();
    assert!(
        sim.output
            .iter()
            .any(|o| o.message.trim() == "R other 2 7 9 again"),
        "{:?}",
        sim.output.iter().map(|o| &o.message).collect::<Vec<_>>()
    );
}
