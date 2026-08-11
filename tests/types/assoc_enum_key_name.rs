//! §6.19.6: a `foreach` index over an associative array whose KEY is a named
//! enum type has that type, so `key.name()` must resolve against THAT enum.
//! With no type binding the lookup scanned every enum and picked the largest,
//! printing a member of an unrelated enum — UVM's report summary listed
//! `UVM_NORADIX` and `UVM_PHASE_DORMANT` where severities belonged
//! (GitHub issue #109). Reference-validated.

use xezim::simulate;

fn msgs(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// Two enums share the values 0/1/2; the key must use its own.
#[test]
fn foreach_key_name_uses_the_key_enum_not_a_same_valued_one() {
    let src = r#"
package p;
  typedef enum int { SEV_INFO = 0, SEV_WARN = 1, SEV_ERR = 2 } sev_e;
  typedef enum int { RADIX_NONE = 0, RADIX_BIN = 1, RADIX_HEX = 2 } radix_e;
endpackage
module top;
  import p::*;
  int cnt [sev_e];
  initial begin
    cnt[SEV_INFO] = 5;
    cnt[SEV_WARN] = 7;
    cnt[SEV_ERR]  = 9;
    foreach (cnt[s]) $display("T|key=%s val=%0d", s.name(), cnt[s]);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let got = msgs(&sim);
    for want in [
        "T|key=SEV_INFO val=5",
        "T|key=SEV_WARN val=7",
        "T|key=SEV_ERR val=9",
    ] {
        assert!(got.iter().any(|m| m == want), "missing {want:?} in {got:?}");
    }
}

/// A plain (non-key) enum variable was always fine — pin it so the fix can't
/// regress the ordinary path.
#[test]
fn plain_enum_variable_name_still_resolves() {
    let src = r#"
package p;
  typedef enum int { A0 = 0, A1 = 1 } a_e;
  typedef enum int { B0 = 0, B1 = 1 } b_e;
endpackage
module top;
  import p::*;
  initial begin
    a_e a = A1;
    b_e b = B0;
    $display("T|%s %s", a.name(), b.name());
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert!(msgs(&sim).iter().any(|m| m == "T|A1 B0"), "got {:?}", msgs(&sim));
}
