//! Issues #143/#144/#145 — the delayed-write dimension of the #142 elision
//! class: every compare that decides whether a delayed write is scheduled,
//! merged, or elided must run against the PENDING delayed value when one
//! exists, and the indexed part-select forms must convert base+width like
//! everything else. All expectations reference-simulator verified.

use xezim::simulate;

fn msgs(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn delayed_ca_indexed_part_select_lands_and_slices_merge() {
    // #143: `assign #1 d[64 +: 32]` was silently dropped (Constant-only
    // helper), and two slice drivers on one base must keep each other's
    // bits — the whole-signal inertial replace used to clobber.
    let out = msgs(
        r#"
module top;
  wire [95:0] d;
  logic [31:0] src = 32'hCAFEBABE;
  assign #1 d[64 +: 32] = src;
  assign #1 d[0  +: 32] = 32'h11111111;
  wire [95:0] e;
  assign #1 e[95 -: 32] = 32'hFEEDFACE;   // the -: spelling
  initial begin
    #3 $display("A_%h", d);
    $display("B_%h", e);
  end
endmodule
"#,
    );
    assert!(out.contains(&"A_cafebabezzzzzzzz11111111".to_string()), "{out:?}");
    assert!(out.contains(&"B_feedfacezzzzzzzzzzzzzzzz".to_string()), "{out:?}");
}

#[test]
fn udp_instance_delay_swallows_glitch_and_bare_hash_parses() {
    // #144: both facets — `#2` without parens must parse, and a pulse
    // narrower than the instance delay must be SWALLOWED (§29.7 inertial),
    // which requires comparing against the PENDING delayed transition
    // rather than the current value.
    let out = msgs(
        r#"
primitive ubuf (o, i);
  output o; input i;
  table
    0 : 0 ;
    1 : 1 ;
  endtable
endprimitive
module top;
  reg i = 0; wire o;
  reg j = 0; wire p;
  ubuf #2 u (o, i);      // bare-literal delay form
  ubuf #(2) v (p, j);    // parenthesized form
  initial begin
    #10 i = 1; j = 1;
    #1  i = 0; j = 0;    // 1-unit pulse into a 2-unit delay
    #10 $display("C_%b_%b", o, p);
    i = 1; j = 1;        // a real transition still propagates
    #5 $display("D_%b_%b", o, p);
  end
endmodule
"#,
    );
    assert!(out.contains(&"C_0_0".to_string()), "{out:?}");
    assert!(out.contains(&"D_1_1".to_string()), "{out:?}");
}

#[test]
fn delayed_nba_of_current_value_is_elided_at_schedule() {
    // #145: the reference elides `q <= #5 v` when v equals the CURRENT value
    // at schedule time — a blocking write in the window makes it observable.
    // The elision must NOT fire when a delayed NBA is already pending for
    // the target (the pending entry, not the register, is the truth then).
    let out = msgs(
        r#"
module top;
  logic q, r;
  initial begin
    q = 0;
    q <= #5 0;          // == current at schedule: never lands
    #1 q = 1;
    #5 $display("E_%b", q);
    r = 0;
    r <= #3 1;          // real change: pending
    r <= #5 0;          // == current, but a pending entry exists: must land
    #6 $display("F_%b", r);
  end
endmodule
"#,
    );
    assert!(out.contains(&"E_1".to_string()), "{out:?}");
    assert!(out.contains(&"F_0".to_string()), "{out:?}");
}
