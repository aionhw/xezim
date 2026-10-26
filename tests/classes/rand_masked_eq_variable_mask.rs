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
