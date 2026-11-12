//! §18.3, §18.6.1: `(x & M) == C` with a mask that depends on another rand
//! variable is satisfiable whenever `C` lies inside `M`, and so is every
//! entry below (the guard can always be left false). The solver guessed
//! `addr[i]` against the masked equality, so randomize() returned 0 on most
//! calls once an entry took the guarded branch with a wide mask.
#[test]
fn masked_equality_with_variable_mask_solves() {
    let result = xezim::simulate(
        r#"
class pc;
  rand bit [1:0] a[];
  rand int m[];
  rand bit [31:0] addr[];
  function new(); a = new[16]; m = new[16]; addr = new[16]; endfunction
  constraint m_c { foreach (a[i]) { m[i] inside {[0:29]}; } }
  constraint o_c { foreach (a[i]) { solve a[i] before addr[i]; solve m[i] before addr[i]; } }
  constraint up_c { foreach (a[i]) {
    if (i > 0 && a[i] == 1 && m[i] > 0) { addr[i] > addr[i-1]; }
    addr[i][31:29] == '0; } }
  constraint mask_c { foreach (a[i]) { if (a[i] == 3) {
    (addr[i] & ((1 << m[i]) - 1)) == ((1 << m[i]) - 1);
    (addr[i] & (1 << m[i])) == 0; } } }
endclass
module t;
  initial begin
    pc p = new;
    int ok = 0, bad = 0;
    for (int k = 0; k < 10; k++) begin
      if (!p.randomize()) continue;
      ok++;
      foreach (p.a[i]) begin
        bit [31:0] mask = (1 << p.m[i]) - 1;
        if (p.addr[i][31:29] != 0) bad++;
        if (p.a[i] == 3 && ((p.addr[i] & mask) != mask || p.addr[i][p.m[i]])) bad++;
        if (i > 0 && p.a[i] == 1 && p.m[i] > 0 && p.addr[i] <= p.addr[i-1]) bad++;
      end
    end
    $display("ok %0d bad %0d", ok, bad);
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
    assert_eq!(lines, ["ok 10 bad 0"]);
}

#[test]
fn masked_equality_regressions() {
    let result = xezim::simulate(
        r#"
class d;
  rand bit [31:0] x, m;
  constraint c { solve m before x; m == -1; ((x & m) == 0) || (x != 0); }
endclass
class u; rand int unsigned x, m; constraint c { x inside {[0:100]}; m == 1; (x & m) == 0; } endclass
module t; initial begin
  d a; u b; int ok, v, sum, hist[101];
  a = new; b = new; ok = 0; sum = 0;
  a.srandom(31);
  repeat (200) if (a.randomize() && a.x != 0) ok++;
  foreach (hist[i]) hist[i] = 0;
  b.srandom(29);
  repeat (5100) if (b.randomize() && b.x <= 100) hist[b.x]++;
  for (v = 0; v <= 100; v += 2) sum += hist[v];
  $display("disj %0d", ok);
  $display("uni %0d %0d", hist[100], sum / 51);
  $finish;
end endmodule
"#,
        100_000,
    )
    .unwrap();
    let lines: Vec<_> = result.output.iter().map(|e| e.message.as_str()).collect();
    let d: Vec<&str> = lines[0].split_whitespace().collect();
    let ok = d[1].parse::<i32>().unwrap();
    assert!(ok >= 190, "a disjunct's mask forced x: {}", lines[0]);
    let u: Vec<&str> = lines[1].split_whitespace().collect();
    let (c100, mean) = (u[1].parse::<i32>().unwrap(), u[2].parse::<i32>().unwrap());
    assert!(c100 * 20 >= mean * 17, "{}", lines[1]);
}

#[test]
fn masked_equality_wide_domain_draws() {
    let result = xezim::simulate(
        r#"
class c; rand bit [63:0] x, m; constraint c { solve m before x; m == 1; (x & m) == 0; } endclass
module t; initial begin
  c a = new; int ok = 0;
  repeat (5) if (a.randomize()) ok++;
  $display("ok %0d", ok);
  $finish;
end endmodule
"#,
        30_000,
    )
    .unwrap();
    let lines: Vec<_> = result.output.iter().map(|e| e.message.as_str()).collect();
    let n: i32 = lines[0].split_whitespace().nth(1).unwrap().parse().unwrap();
    assert_eq!(n, 5, "{}", lines[0]);
}

#[test]
fn masked_equality_soft_enabled_hint() {
    let result = xezim::simulate(
        r#"
class c; rand bit [31:0] x, m; constraint c { solve m before x; m == -1; soft ((x & m) == 0); } endclass
module t; initial begin
  c a = new; int bad = 0;
  repeat (100) if (!a.randomize()) bad++;
  $display("bad %0d", bad);
  $finish;
end endmodule
"#,
        120_000,
    )
    .unwrap();
    let lines: Vec<_> = result.output.iter().map(|e| e.message.as_str()).collect();
    let n: i32 = lines[0].split_whitespace().nth(1).unwrap().parse().unwrap();
    assert_eq!(n, 0, "{}", lines[0]);
}
