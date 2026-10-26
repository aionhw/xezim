//! §10.8 / §10.10: a concatenation passed to a fixed-size unpacked array
//! input formal is an unpacked array concatenation. An array operand
//! contributes all of its elements, leftmost item to the leftmost index,
//! including for a descending range (`int r[3:0]`).
#[test]
fn concat_actual_fills_fixed_array_formal() {
    let result = xezim::simulate(
        r#"
class cfg_c;
  byte g[3] = '{10, 11, 12};
  byte s = 5;
endclass
class show_c;
  function string show(byte r[6]);
    string s = "";
    foreach (r[i]) s = {s, $sformatf("%0d ", r[i])};
    return s;
  endfunction
endclass
function automatic string down(int r[3:0]);
  return $sformatf("%0d %0d %0d %0d", r[3], r[2], r[1], r[0]);
endfunction
module t;
  cfg_c c;
  show_c p;
  byte loc[2] = '{1, 2};
  int il[2] = '{1, 2};
  int bad;
  initial begin
    c = new();
    p = new();
    $display("%s", p.show({c.g, c.s, loc}));
    $display("%s", p.show({loc, c.s, c.g}));
    $display("%s", down({il, 3, 4}));
    for (int i = 0; i < 200; i++)
      bad += p.show({loc, c.s, c.g}) != "1 2 5 10 11 12 ";
    $display("bad %0d", bad);
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
        ["10 11 12 5 1 2 ", "1 2 5 10 11 12 ", "1 2 3 4", "bad 0"]
    );
}
