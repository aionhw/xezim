//! §18.5.5: a `unique` group that lists a whole rand fixed array, or a
//! scalar together with an array, makes the scalars and every leaf element
//! pairwise distinct. Each class is satisfiable, so randomize() succeeds
//! every time (§18.3) with no two members equal.
#[test]
fn unique_over_scalars_and_fixed_arrays() {
    let result = xezim::simulate(
        r#"
class sf_c;
  rand bit [3:0] x; rand bit [3:0] a[3];
  constraint k { unique {x, a}; }
  function void vals(ref int v[$]); v = {x}; foreach (a[i]) v.push_back(a[i]); endfunction
endclass
class sd_c;
  rand bit [3:0] x, y; rand bit [3:0] a[];
  constraint k { a.size() == 3; unique {x, y, a}; }
  function void vals(ref int v[$]); v = {x, y}; foreach (a[i]) v.push_back(a[i]); endfunction
endclass
class ff_c;
  rand bit [3:0] a[3]; rand bit [3:0] b[3];
  constraint k { unique {a, b}; }
  function void vals(ref int v[$]);
    v.delete(); foreach (a[i]) v.push_back(a[i]); foreach (b[i]) v.push_back(b[i]);
  endfunction
endclass
class fd_c;
  rand bit [3:0] a[3]; rand bit [3:0] b[];
  constraint k { b.size() == 2; unique {a, b}; }
  function void vals(ref int v[$]);
    v.delete(); foreach (a[i]) v.push_back(a[i]); foreach (b[i]) v.push_back(b[i]);
  endfunction
endclass
class tight_c;
  rand bit [1:0] x; rand bit [1:0] a[3];
  constraint k { unique {x, a}; }
  function void vals(ref int v[$]); v = {x}; foreach (a[i]) v.push_back(a[i]); endfunction
endclass
module t;
  function automatic bit dup(int v[$]);
    foreach (v[i]) for (int j = i + 1; j < v.size(); j++) if (v[i] == v[j]) return 1;
    return 0;
  endfunction
  initial begin
    sf_c sf = new(); sd_c sd = new(); ff_c ff = new(); fd_c fd = new(); tight_c tt = new();
    int ok[5], bad[5];
    int v[$];
    repeat (10) begin
      if (sf.randomize()) begin ok[0]++; sf.vals(v); bad[0] += dup(v); end
      if (sd.randomize()) begin ok[1]++; sd.vals(v); bad[1] += dup(v) || v.size() != 5; end
      if (ff.randomize()) begin ok[2]++; ff.vals(v); bad[2] += dup(v); end
      if (fd.randomize()) begin ok[3]++; fd.vals(v); bad[3] += dup(v) || v.size() != 5; end
      if (tt.randomize()) begin ok[4]++; tt.vals(v); bad[4] += dup(v); end
    end
    $display("ok %0d %0d %0d %0d %0d", ok[0], ok[1], ok[2], ok[3], ok[4]);
    $display("bad %0d %0d %0d %0d %0d", bad[0], bad[1], bad[2], bad[3], bad[4]);
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
    assert_eq!(lines, ["ok 10 10 10 10 10", "bad 0 0 0 0 0"]);
}
