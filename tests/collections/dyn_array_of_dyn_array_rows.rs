//! §7.5 / §7.4.2: `d[i] = new[n]` sizes row `d[i]` and `d[i][j] = v` writes one
//! element of that row for a class property and for a local.

/// §7.4.6 / §11.4.1: selectors run once, including a discarded row sizing.
#[test]
fn dynamic_rows_evaluate_selectors_once() {
    let sim = xezim::simulate(
        r#"
module top;
 int calls=0;
 function int index_value(); calls++; return 0; endfunction
 class rows;
  typedef bit [7:0] octet;
  octet data[][];
 endclass
 rows r;
 int sizes=0;
 function int size_value(); sizes++; return 3; endfunction
 initial begin
  r=new(); r.data=new[1];
  r.data[index_value()]=new[size_value()];
  r.data[index_value()][index_value()]=8'hac;
  r.data[4]=new[size_value()];
  $display("rows=%0d value=%h calls=%0d sizes=%0d",r.data[0].size(),r.data[0][0],calls,sizes);
 end
endmodule
"#,
        10,
    )
    .expect("dynamic rows");
    let out = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(out.contains("rows=3 value=ac calls=3 sizes=2"), "{out}");
}

#[test]
fn dyn_array_of_dyn_array_rows_size_and_write() {
    let result = xezim::simulate(
        r#"
class item;
  int n;
  function new(int n); this.n = n; endfunction
endclass
class c;
  static int ctor_calls;
  bit v[][];
  int d[][];
  item objs[];
  function new();
    ctor_calls++;
  endfunction
  function void build(int n);
    v = new[2];
    foreach (v[i]) v[i] = new[n];
    v[1][n - 1] = 1;
    objs = new[2];
    foreach (objs[i]) objs[i] = new(i + 7);
  endfunction
endclass
module t;
  c o;
  int bad;
  task automatic local_rows();
    bit w[][];
    w = new[3];
    foreach (w[i]) w[i] = new[i + 2];
    $display("local %0d %0d %0d %0d", w.size(), w[0].size(), w[1].size(), w[2].size());
  endtask
  initial begin
    o = new();
    o.d = new[1];
    o.d[0] = new[1];
    o.d[0][0] = 1.5;
    $display("real %0d", o.d[0][0]);
    for (int k = 1; k <= 200; k++) begin
      o.build(k);
      bad += o.v[0].size() != k || o.v[1].size() != k || o.v[1][k - 1] != 1 || o.v[0][k - 1] != 0;
    end
    $display("ctor %0d bad %0d rows %0d %0d objs %0d %0d", c::ctor_calls, bad, o.v.size(),
             o.v[1].size(), o.objs[0].n, o.objs[1].n);
    local_rows();
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
            "real 2",
            "ctor 1 bad 0 rows 2 200 objs 7 8",
            "local 3 2 3 4"
        ]
    );
}
