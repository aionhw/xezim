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

/// A fixed array operand is flattened over its declared indexes, left to
/// right, whatever its bounds.
#[test]
fn concat_actual_flattens_array_operand_bounds() {
    let result = xezim::simulate(
        r#"
function automatic string show(int r[3]);
  return $sformatf("%0d %0d %0d", r[0], r[1], r[2]);
endfunction
function automatic string show2(int r[2]);
  return $sformatf("%0d %0d", r[0], r[1]);
endfunction
module t;
  int down[1:0] = '{10, 20};
  int up[4:5] = '{7, 9};
  initial begin
    $display("%s", show({down, 30}));
    $display("%s", show2({up}));
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
    assert_eq!(lines, ["10 20 30", "7 9"]);
}

/// §13.5: an input array formal is the subroutine's own variable, so binding
/// a pattern or concatenation actual to a formal named like a module array
/// leaves the module array alone.
#[test]
fn array_formal_binding_leaves_same_named_module_array() {
    let result = xezim::simulate(
        r#"
module t;
  int r[2] = '{9, 8};
  function automatic int f(input int r[2]);
    return r[0] * 10 + r[1];
  endfunction
  function int fs(input int r[2]);
    return r[0] * 10 + r[1];
  endfunction
  initial begin
    int a, b, s;
    a = 1; b = 2;
    s = f('{a, b});
    $display("pattern f=%0d r=%0d %0d", s, r[0], r[1]);
    s = f({a, b});
    $display("concat f=%0d r=%0d %0d", s, r[0], r[1]);
    s = fs('{a, b});
    $display("static pattern f=%0d r=%0d %0d", s, r[0], r[1]);
    s = fs({a, b});
    $display("static concat f=%0d r=%0d %0d", s, r[0], r[1]);
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
            "pattern f=12 r=9 8",
            "concat f=12 r=9 8",
            "static pattern f=12 r=9 8",
            "static concat f=12 r=9 8",
        ]
    );
}

/// A whole-array actual, a default and a body write all bind or change the
/// formal's own storage, never the same-named module array.
#[test]
fn array_formal_storage_is_per_call() {
    let result = xezim::simulate(
        r#"
module t;
  int r[2] = '{9, 8};
  int o[2] = '{5, 6};
  function automatic int f(input int r[2]);
    return r[0] * 10 + r[1];
  endfunction
  function automatic int fw(input int r[2]);
    r[0] = 77;
    return r[0] + r[1];
  endfunction
  function automatic int fd(input int r[2] = '{3, 4});
    return r[0] * 10 + r[1];
  endfunction
  function automatic int rec(input int r[2], int n);
    int s;
    if (n == 0) return r[0] + r[1];
    s = rec('{r[1], r[0]}, n - 1);
    return s * 100 + r[0];
  endfunction
  initial begin
    int a, b, s;
    a = 1; b = 2;
    s = f(o);
    $display("whole f=%0d r=%0d %0d", s, r[0], r[1]);
    s = fw(o);
    $display("write f=%0d r=%0d %0d o=%0d %0d", s, r[0], r[1], o[0], o[1]);
    s = fd();
    $display("default f=%0d r=%0d %0d", s, r[0], r[1]);
    s = rec('{a, b}, 2);
    $display("recursion f=%0d r=%0d %0d", s, r[0], r[1]);
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
            "whole f=56 r=9 8",
            "write f=83 r=9 8 o=5 6",
            "default f=34 r=9 8",
            "recursion f=30201 r=9 8",
        ]
    );
}

/// §13.5: an output, inout or slice-bound array formal copies back to its
/// actual on return and touches nothing else, including a module array that
/// shares the formal's name.
#[test]
fn array_formal_copy_back_leaves_same_named_module_array() {
    let result = xezim::simulate(
        r#"
module t;
  int r[2] = '{9, 8};
  int o[2] = '{5, 6};
  int big[4] = '{1, 2, 3, 4};
  function automatic void outf(output int r[2]);
    r[0] = 41; r[1] = 42;
  endfunction
  function automatic void inoutf(inout int r[2]);
    r[0] = r[0] + 1; r[1] = r[1] + 1;
  endfunction
  function automatic int fsl(input int r[2]);
    return r[0] * 10 + r[1];
  endfunction
  task automatic tk(input int r[2]);
    r[0] = 55;
    $display("task r=%0d %0d", r[0], r[1]);
  endtask
  initial begin
    int a, b, s;
    int x[2];
    a = 1; b = 2;
    outf(x);
    $display("output x=%0d %0d r=%0d %0d", x[0], x[1], r[0], r[1]);
    x = '{10, 20};
    inoutf(x);
    $display("inout x=%0d %0d r=%0d %0d", x[0], x[1], r[0], r[1]);
    s = fsl(big[1:2]);
    $display("slice f=%0d r=%0d %0d", s, r[0], r[1]);
    tk(o);
    $display("task r=%0d %0d o=%0d %0d", r[0], r[1], o[0], o[1]);
    tk('{a, b});
    $display("task pattern r=%0d %0d", r[0], r[1]);
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
            "output x=41 42 r=9 8",
            "inout x=11 21 r=9 8",
            "slice f=23 r=9 8",
            "task r=55 6",
            "task r=9 8 o=5 6",
            "task r=55 2",
            "task pattern r=9 8",
        ]
    );
}

/// A formal passed on to another subroutine, a `ref` or `inout` formal bound
/// to the module array of the same name or to another array, and a class
/// method formal named like a class property each get their own storage.
#[test]
fn array_formal_nested_ref_and_method_calls_keep_own_storage() {
    let result = xezim::simulate(
        r#"
class C;
  int r[2] = '{9, 8};
  function int m(input int r[2]);
    return r[0] * 10 + r[1];
  endfunction
  function int mc();
    int a = 1, b = 2;
    return m({a, b});
  endfunction
  function void bumpm(ref int r[2]); r[0] = r[0] + 1; endfunction
endclass
module t;
  int r[2] = '{9, 8};
  function automatic int g(input int r[2]);
    return r[0] + r[1];
  endfunction
  function automatic int f(input int r[2]);
    return g(r) * 10 + r[0];
  endfunction
  task automatic bump(ref int r[2]);
    r[0] = r[0] + 1;
  endtask
  task automatic bump_io(inout int r[2]);
    r[1] = r[1] + 5;
  endtask
  initial begin
    C c = new();
    int a = 1, b = 2, s;
    int o[2] = '{5, 6};
    s = f(o);
    $display("nested f=%0d r=%0d %0d", s, r[0], r[1]);
    s = f({a, b});
    $display("nested concat f=%0d r=%0d %0d", s, r[0], r[1]);
    bump(r);
    $display("ref identity r=%0d %0d", r[0], r[1]);
    bump(o);
    $display("ref other r=%0d %0d o=%0d %0d", r[0], r[1], o[0], o[1]);
    bump_io(o);
    $display("inout other r=%0d %0d o=%0d %0d", r[0], r[1], o[0], o[1]);
    bump_io(r);
    $display("inout identity r=%0d %0d", r[0], r[1]);
    s = c.m({a, b});
    $display("method f=%0d c.r=%0d %0d", s, c.r[0], c.r[1]);
    s = c.mc();
    $display("method concat f=%0d c.r=%0d %0d", s, c.r[0], c.r[1]);
    c.bumpm(c.r);
    $display("method ref c.r=%0d %0d", c.r[0], c.r[1]);
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
            "nested f=115 r=9 8",
            "nested concat f=31 r=9 8",
            "ref identity r=10 8",
            "ref other r=10 8 o=6 6",
            "inout other r=10 8 o=6 11",
            "inout identity r=10 13",
            "method f=12 c.r=9 8",
            "method concat f=12 c.r=9 8",
            "method ref c.r=10 8",
        ]
    );
}
