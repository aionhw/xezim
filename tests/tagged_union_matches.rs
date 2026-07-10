//! Tagged unions (IEEE 1800-2017 §7.3.2) with pattern matching (§12.6.1).
//!
//! `case (expr) matches` used to parse each item's pattern and then throw it
//! away (`CaseItem.patterns` was left empty), so no item could ever match and
//! the statement silently did nothing. Patterns are now modelled and tested at
//! run time, including `.v` bindings and the `&&& guard`.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  typedef union tagged { void Invalid; int Valid; } VInt;
  VInt v;
  int  hits_invalid = 0;
  int  hits_small   = 0;
  int  hits_big     = 0;
  int  hits_default = 0;
  int  seen_n       = 0;

  task automatic classify();
    case (v) matches
      tagged Invalid                : hits_invalid = hits_invalid + 1;
      tagged Valid .n &&& (n > 100) : begin hits_big = hits_big + 1; seen_n = n; end
      tagged Valid .n               : begin hits_small = hits_small + 1; seen_n = n; end
      default                       : hits_default = hits_default + 1;
    endcase
  endtask

  initial begin
    v = tagged Valid (23);  classify();   // -> small, seen_n = 23
    v = tagged Valid (500); classify();   // -> big,   seen_n = 500
    v = tagged Invalid;     classify();   // -> invalid
  end
endmodule
"#;

fn get(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("tb.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} not u64-able", name))
}

#[test]
fn case_matches_binds_and_guards_tagged_union() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // The void-tag arm fired exactly once.
    assert_eq!(get(&sim, "hits_invalid"), 1, "tagged Invalid did not match");
    // The guarded arm took the 500 case, the unguarded arm took the 23 case.
    assert_eq!(get(&sim, "hits_big"), 1, "`&&& (n > 100)` guard did not select the big arm");
    assert_eq!(get(&sim, "hits_small"), 1, "unguarded `tagged Valid .n` arm did not match");
    // No arm should have fallen through to default.
    assert_eq!(get(&sim, "hits_default"), 0, "an item fell through to default");
    // The `.n` binding carried the member payload (last write was Invalid, so
    // seen_n retains the 500 from the big arm).
    assert_eq!(get(&sim, "seen_n") & 0xFFFF_FFFF, 500, "`.n` binding did not bind the payload");
}
