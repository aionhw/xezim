//! IEEE 1800-2017 §27.6: unnamed generate blocks get implicit scope names
//! `genblk<N>` (N = the construct's ordinal among ALL generate constructs in
//! its enclosing scope — named ones consume a number too), named blocks use
//! their label, and generate-for iterations append `[i]`. These scopes must
//! appear in instance hierarchical paths — commercial simulators report
//! `u_mod.genblk1.u_ff`, and a customer diffing xezim logs against theirs
//! found xezim silently omitting the scope.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

const LEAF: &str = r#"
module leaf (input a, output y);
  buf b0 (y, a);
  initial $display("LEAF=%m");
endmodule
"#;

#[test]
fn unnamed_if_block_is_genblk1() {
    let src = format!(
        r#"{LEAF}
module mid (input a, output y);
  if (1) begin
    leaf u_ff (.a(a), .y(y));
  end
endmodule
module top;
  reg a = 1; wire y;
  mid u_mid (.a(a), .y(y));
  initial #2 $finish;
endmodule
"#
    );
    let sim = simulate(&src, 100).expect("sim");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "LEAF=top.u_mid.genblk1.u_ff"),
        "expected top.u_mid.genblk1.u_ff; output: {:?}",
        msgs
    );
}

#[test]
fn ordinals_count_named_constructs_and_nest() {
    // Construct #1 is NAMED (consumes ordinal 1) -> second, unnamed construct
    // is genblk2. A nested unnamed block restarts numbering in its own scope.
    let src = format!(
        r#"{LEAF}
module mid (input a, output w1, output w2, output w3);
  if (1) begin : ublk
    leaf u_named (.a(a), .y(w1));
  end
  if (1) begin
    leaf u_un (.a(a), .y(w2));
  end
  if (1) begin
    if (1) begin
      leaf u_nest (.a(a), .y(w3));
    end
  end
endmodule
module top;
  reg a = 1; wire w1, w2, w3;
  mid u_mid (.a(a), .w1(w1), .w2(w2), .w3(w3));
  initial #2 $finish;
endmodule
"#
    );
    let sim = simulate(&src, 100).expect("sim");
    let msgs = messages(&sim);
    for want in [
        "LEAF=top.u_mid.ublk.u_named",
        "LEAF=top.u_mid.genblk2.u_un",
        "LEAF=top.u_mid.genblk3.genblk1.u_nest",
    ] {
        assert!(
            msgs.iter().any(|m| m == want),
            "missing {}; output: {:?}",
            want,
            msgs
        );
    }
}

#[test]
fn generate_for_iterations_get_indexed_scopes() {
    // Each iteration is scope[i]; the instance name itself stays clean
    // (`genblk1[0].u_ff`, not a mangled `u_ff__gf_i_0_`).
    let src = format!(
        r#"{LEAF}
module top;
  reg a = 1; wire y0, y1;
  wire [1:0] ys;
  genvar i;
  for (i = 0; i < 2; i = i + 1) begin
    leaf u_ff (.a(a), .y(ys[i]));
  end
  for (i = 0; i < 2; i = i + 1) begin : lp
    leaf u_lp (.a(a), .y());
  end
  initial #2 $finish;
endmodule
"#
    );
    let sim = simulate(&src, 100).expect("sim");
    let msgs = messages(&sim);
    for want in [
        "LEAF=top.genblk1[0].u_ff",
        "LEAF=top.genblk1[1].u_ff",
        "LEAF=top.lp[0].u_lp",
        "LEAF=top.lp[1].u_lp",
    ] {
        assert!(
            msgs.iter().any(|m| m == want),
            "missing {}; output: {:?}",
            want,
            msgs
        );
    }
}

#[test]
fn generate_case_arm_label_used() {
    let src = format!(
        r#"{LEAF}
module top;
  reg a = 1; wire y;
  generate
    case (1)
      1: begin : carm
        leaf u_case (.a(a), .y(y));
      end
    endcase
  endgenerate
  initial #2 $finish;
endmodule
"#
    );
    let sim = simulate(&src, 100).expect("sim");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "LEAF=top.carm.u_case"),
        "expected top.carm.u_case; output: {:?}",
        msgs
    );
}

#[test]
fn hierarchical_reference_through_genblk_resolves() {
    // §27.6 also makes `u_mid.genblk1.u_ff.y`-style references legal — the
    // scope is a real hierarchy level.
    let src = format!(
        r#"{LEAF}
module mid (input a);
  if (1) begin
    wire y;
    leaf u_ff (.a(a), .y(y));
  end
endmodule
module top;
  reg a = 1;
  mid u_mid (.a(a));
  initial begin
    #1 $display("HREF=%b", u_mid.genblk1.u_ff.y);
    $finish;
  end
endmodule
"#
    );
    let sim = simulate(&src, 100).expect("sim");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "HREF=1"),
        "hierarchical ref through genblk1 failed; output: {:?}",
        msgs
    );
}
