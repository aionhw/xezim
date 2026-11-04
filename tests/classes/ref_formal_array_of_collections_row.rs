//! §13.5.2: a `ref` dynamic-array formal bound to one row of a class
//! property declared as a fixed array of dynamic arrays (`item_c rows[2][]`,
//! passed as `rows[1]`) shares that row with the caller, so sizing and
//! filling it in the callee is visible after the call.
#[test]
fn ref_formal_bound_to_row_of_array_of_collections() {
    let result = xezim::simulate(
        r#"
class item_c;
  string nm;
  function new(string n = "");
    nm = n;
  endfunction
endclass
class gen_c;
  item_c rows[2][];
  function void fill(ref item_c sp[], input int n);
    sp = new[n];
    foreach (sp[i]) sp[i] = new($sformatf("e%0d", i));
    $display("callee %0d %s", sp.size(), sp[n - 1].nm);
  endfunction
  function void run();
    fill(rows[1], 3);
  endfunction
endclass
module t;
  gen_c g;
  initial begin
    g = new();
    g.run();
    $display("caller %0d %0d %s", g.rows[0].size(), g.rows[1].size(), g.rows[1][2].nm);
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
    assert_eq!(lines, ["callee 3 e2", "caller 0 3 e2"]);
}
