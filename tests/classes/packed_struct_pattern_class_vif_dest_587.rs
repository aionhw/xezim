//! Packed-struct assignment patterns into class properties and vif
//! array elements.
//!
//! Regression test for the layout fix on this branch: the generic
//! ExprKind::AssignmentPattern evaluation concatenated the pattern
//! items at their own literal widths and zero-extended to the target
//! (157 bits into a 242-bit struct) while every read extracted members
//! at their declared offsets -- so a '{...} literal written into a
//! packed-struct CLASS property (obj.prop, this.prop, or a bare
//! property inside a method) or a whole element of a vif FIXED array
//! scrambled the upper fields, and the corrupted struct then made the
//! downstream randomize pre-solve UNSAT in production.
//!
//! Cases: the issue #587 (Struct-Literal Assignment Mishandled) matrix
//! -- P1 positional literal into a class property, P3 literal into a
//! vif fixed-array element, P4 the production whole-struct copy out of
//! a literal-filled element, controls K1-K7 (plain-variable literal,
//! vif packed/unpacked copy, per-field workaround, per-field fill
//! sanity, local var / local unpacked array) -- extended with P2/P2b
//! literal into a bare/this property inside a method, P5 re-assign of
//! the same literal (a pattern equal to the current contents must
//! still count as handled), and P6/P7 named-member + default-key
//! patterns (unlisted members get the default, sized to the member
//! width).
use xezim::simulate;

#[test]
fn packed_struct_pattern_class_vif_dest() {
    let sim = simulate(
        include_str!("packed_struct_pattern_class_vif_dest_587.sv"),
        100,
    )
    .expect("simulate failed");
    let failures = sim
        .get_signal("failures")
        .or_else(|| sim.get_signal("tb_top.failures"))
        .unwrap_or_else(|| panic!("signal not found: failures"))
        .to_u64()
        .unwrap_or_else(|| panic!("failures not u64-able"));
    assert_eq!(
        failures, 0,
        "packed-struct pattern sweep: every case must hold (see FAIL lines above)"
    );
    assert!(
        sim.output.iter().any(|o| o.message.trim() == "TEST_PASS"),
        "SVTEST_PASSFAIL must print TEST_PASS"
    );
}

/// §8.14/§8.15: a pattern written to a property a derived class redeclares
/// lands in the cell its spelling selects: `this.p` in a base method and
/// `super.p` the base's, a bare `p` in the derived class its own, `h.p`
/// through a base handle the base's.
#[test]
fn pattern_into_a_shadowed_property() {
    let out: Vec<String> = xezim::simulate(
        r#"typedef struct packed { logic [7:0] a; logic [99:0] b; logic [3:0] c; } st_t;
class base;
  st_t p;
  function void set_base(); this.p = '{a: 8'h11, b: 100'h22, c: 4'h3}; endfunction
  function st_t get_base(); return p; endfunction
endclass
class derived extends base;
  st_t p;
  function void set_super(); super.p = '{a: 8'h44, b: 100'h55, c: 4'h6}; endfunction
  function void set_own(); p = '{a: 8'h77, b: 100'h88, c: 4'h9}; endfunction
endclass
module t;
  initial begin
    derived d = new; base h;
    h = d;
    d.set_base();
    $display("R1 %h %h %h", d.get_base().a, d.get_base().b, d.p.a);
    d.set_super();
    $display("R2 %h %h %h", d.get_base().a, d.get_base().c, d.p.a);
    d.set_own();
    $display("R3 %h %h %h", d.get_base().a, d.p.a, d.p.c);
    h.p = '{a: 8'hAA, b: 100'hBB, c: 4'hC};
    $display("R4 %h %h %h", d.get_base().a, d.get_base().b, d.p.a);
  end
endmodule
"#,
        100,
    )
    .expect("simulate")
    .output
    .iter()
    .map(|l| l.message.clone())
    .filter(|m| m.starts_with('R'))
    .collect();
    assert_eq!(
        out,
        [
            "R1 11 0000000000000000000000022 xx",
            "R2 44 6 xx",
            "R3 44 77 9",
            "R4 aa 00000000000000000000000bb 77",
        ]
    );
}
