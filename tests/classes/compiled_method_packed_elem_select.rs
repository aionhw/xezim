/// A compiled class method misread a select INTO an element of a member
/// collection whose element is a multi-dimensional packed array
/// (`rand bit [8:0][7:0] arr[3]`): the bare element read `arr[j]` lowers
/// to LoadCollElem, but a further select `arr[j][2]` fell through to the
/// generic Index tail and took ONE BIT (BitSelectConst) of the loaded
/// 72-bit element instead of the 8-bit packed element [23:16] (§7.4.1).
/// The self-determined compare context (`arr[i][2] != i` in a `foreach`)
/// additionally narrowed the slice to 1 bit through `expr_max_width`.
///
/// Expected output verified byte-for-byte against the reference simulator.
use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn compiled_method_packed_elem_select() {
    let src = include_str!("compiled_method_packed_elem_select.sv");
    assert_eq!(
        t_lines(src),
        [
            "T| rand r=1 ok=1",
            "T| seeded ref b2=0,1,2 b1=8,7,6",
            "T| seeded ok4=1",
        ]
    );
}

/// §7.4.5, §7.10: the outer index selects a row, the inner one an entire
/// integer. Accumulation must read the same cells as a direct display.
#[test]
fn method_queue_of_unpacked_rows() {
    assert_eq!(t_lines(r#"
module top;
  typedef int pair_t[2];
  class K;
    pair_t q[$];
    function int sum_foreach();
      foreach (q[j]) sum_foreach += q[j][1];
    endfunction
    function int sum_local();
      int s = 0;
      for (int j = 0; j < q.size(); j++) s += q[j][1];
      return s;
    endfunction
  endclass
  initial begin
    automatic K k = new;
    k.q.push_back('{1, 2}); k.q.push_back('{3, 4});
    $display("T|%0d %0d", k.sum_foreach(), k.sum_local());
    $display("T|%0d %0d", k.q[0][1], k.q[1][1]);
  end
endmodule
"#), ["T|6 6", "T|2 4"]);
}
