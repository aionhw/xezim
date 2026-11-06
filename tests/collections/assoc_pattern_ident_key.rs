//! §10.9.1 — the index of a keyed array assignment pattern may be a bare
//! identifier (`'{ PM_PAGE : v }`), and on an ARRAY target it is an index
//! expression, not a struct member designator.
//!
//! The parser cannot tell the two apart without the target type, so a bare
//! identifier key arrives as `AssignmentPatternItem::Named`. That is correct
//! for a struct and wrong for an array: the item was dropped and the array
//! came out empty, which is how a UVM scoreboard's `const cmd_info_s
//! CMD_DB [pmbus_cmd_e] = '{ PM_PAGE : ... }` ended up with `num() == 0`.

use xezim::simulate;

fn msgs(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn assoc_pattern_accepts_bare_enum_and_const_ident_keys() {
    let src = r#"
package p;
  typedef enum int { PM_PAGE = 0, PM_OPERATION = 1, PM_VOUT = 2 } cmd_e;
  typedef struct packed { int lo; int hi; } info_s;
endpackage

module top;
  import p::*;
  localparam int LP_A = 10;
  localparam int LP_B = 20;

  // Enum key written as a bare identifier — a struct member designator to the
  // parser, an index expression to an array target.
  const info_s DB [cmd_e] = '{
      PM_PAGE      : '{1, 1}
    , PM_OPERATION : '{2, 4}
    , PM_VOUT      : '{3, 9}
  };

  // Same shape with an integral key type.
  int TAB [int] = '{ LP_A : 100, LP_B : 200 };

  initial begin
    $display("T|enum_num=%0d", DB.num());
    $display("T|page=%0d/%0d", DB[PM_PAGE].lo, DB[PM_PAGE].hi);
    $display("T|vout=%0d/%0d", DB[PM_VOUT].lo, DB[PM_VOUT].hi);
    $display("T|int_num=%0d", TAB.num());
    $display("T|tab=%0d/%0d", TAB[LP_A], TAB[LP_B]);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let got = msgs(&sim);
    for want in [
        "T|enum_num=3",
        "T|page=1/1",
        "T|vout=3/9",
        "T|int_num=2",
        "T|tab=100/200",
    ] {
        assert!(got.iter().any(|m| m == want), "missing {want:?} in {got:?}");
    }
}

#[test]
fn fixed_array_pattern_accepts_bare_const_ident_keys() {
    let src = r#"
module top;
  localparam int IDX = 2;
  int fa [4] = '{ IDX : 9 };
  initial begin
    int local_fa [4];
    local_fa = '{ IDX : 7, 0 : 5 };
    $display("T|fa=%0d/%0d/%0d/%0d", fa[0], fa[1], fa[2], fa[3]);
    $display("T|lfa=%0d/%0d", local_fa[0], local_fa[IDX]);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let got = msgs(&sim);
    for want in ["T|fa=0/0/9/0", "T|lfa=5/7"] {
        assert!(got.iter().any(|m| m == want), "missing {want:?} in {got:?}");
    }
}

/// A `default:` item alongside bare-ident keys still fills the rest.
#[test]
fn assoc_pattern_ident_keys_with_default() {
    let src = r#"
module top;
  typedef enum int { K0 = 0, K1 = 1 } k_e;
  initial begin
    int q [k_e];
    q = '{ K1 : 4, default : 1 };
    $display("T|k0=%0d k1=%0d", q[K0], q[K1]);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|k0=1 k1=4"),
        "got {:?}",
        msgs(&sim)
    );
}

/// The procedural form goes through a different path than the `const`
/// declaration initialiser, so pin it separately.
#[test]
fn procedural_assoc_pattern_accepts_bare_ident_keys() {
    let src = r#"
package p;
  typedef enum int { A_ONE = 1, A_TWO = 2 } a_e;
endpackage

module top;
  import p::*;
  initial begin
    int q [a_e];
    q = '{ A_ONE : 11, A_TWO : 22 };
    $display("T|num=%0d", q.num());
    $display("T|vals=%0d/%0d", q[A_ONE], q[A_TWO]);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let got = msgs(&sim);
    for want in ["T|num=2", "T|vals=11/22"] {
        assert!(got.iter().any(|m| m == want), "missing {want:?} in {got:?}");
    }
}

/// A struct target must keep reading a bare identifier as a member name —
/// that is the whole reason the parser makes the distinction.
#[test]
fn struct_pattern_still_treats_bare_ident_as_member_name() {
    let src = r#"
module top;
  typedef struct packed { int lo; int hi; } pair_s;
  initial begin
    pair_s s = '{ lo : 7, hi : 8 };
    $display("T|s=%0d/%0d", s.lo, s.hi);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|s=7/8"),
        "got {:?}",
        msgs(&sim)
    );
}
