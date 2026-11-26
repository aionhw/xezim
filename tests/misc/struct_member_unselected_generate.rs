//! §27.5/§7.2: the struct-member lint must not look into a conditional
//! generate branch that no instance selects. An unselected branch is not
//! elaborated, so its names are never resolved: Caliptra's `kmac.sv`
//! (`EnFullKmac = 0`) and `entropy_src.sv` (`Stub = 0`) name members that
//! exist only in the configuration the branch serves. xezim rejected both
//! with "not a member of the struct or union". A branch some instance does
//! select is still checked.

use xezim::simulate;

const PKG: &str = r#"
package p;
  typedef struct packed { logic [7:0] a; } s_t;
endpackage
"#;

fn design(body: &str, top: &str) -> String {
    format!("{PKG}{body}{top}")
}

const IF_CHILD: &str = r#"
module m #(parameter bit Full = 1'b1) (output logic [7:0] o);
  import p::*;
  s_t r;
  assign r.a = 8'h5a;
  if (Full) begin : gen_full
    assign o = r.missing;
  end else begin : gen_lite
    assign o = r.a;
  end
endmodule
"#;

const CASE_CHILD: &str = r#"
module m #(parameter int Mode = 2) (output logic [7:0] o);
  import p::*;
  s_t r;
  assign r.a = 8'h5a;
  case (Mode)
    0, 1: begin : gen_lo
      assign o = r.a;
    end
    2: begin : gen_full
      assign o = r.missing;
    end
    default: begin : gen_other
      assign o = r.also_missing;
    end
  endcase
endmodule
"#;

fn top(params: &[&str]) -> String {
    let mut s = String::from("module t;\n");
    for (i, p) in params.iter().enumerate() {
        s.push_str(&format!(
            "  logic [7:0] o{i};\n  m #({p}) u{i} (.o(o{i}));\n"
        ));
    }
    s.push_str("  initial begin\n    #1;\n");
    for i in 0..params.len() {
        s.push_str(&format!("    $display(\"O{i}|%h\", o{i});\n"));
    }
    s.push_str("  end\nendmodule\n");
    s
}

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 10).expect("simulate");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn member_in_unselected_generate_if_branch_is_not_checked() {
    let o = lines(&design(IF_CHILD, &top(&[".Full(1'b0)"])));
    assert!(o.iter().any(|l| l == "O0|5a"), "{o:?}");
}

#[test]
fn member_in_unselected_generate_case_arm_is_not_checked() {
    let o = lines(&design(CASE_CHILD, &top(&[".Mode(1)", ".Mode(0)"])));
    assert!(o.iter().any(|l| l == "O0|5a"), "{o:?}");
    assert!(o.iter().any(|l| l == "O1|5a"), "{o:?}");
}

#[test]
fn member_in_unselected_branch_of_the_top_is_not_checked() {
    let src = format!(
        "{PKG}{}",
        r#"
module t;
  import p::*;
  localparam bit Full = 1'b0;
  s_t r;
  logic [7:0] o;
  assign r.a = 8'h5a;
  if (Full) begin : gen_full
    assign o = r.missing;
  end else begin : gen_lite
    if (!Full) begin : gen_inner
      assign o = r.a;
    end else begin : gen_inner_full
      assign o = r.missing;
    end
  end
  initial #1 $display("O|%h", o);
endmodule
"#
    );
    let o = lines(&src);
    assert!(o.iter().any(|l| l == "O|5a"), "{o:?}");
}

/// A nested construct is decided for the instances that reach its
/// enclosing branch only: `Mode = 0` reaches the outer branch but not the
/// inner one, and `Mode = 1` selects the inner condition but never reaches
/// it.
#[test]
fn member_in_a_nested_branch_no_reaching_instance_selects_is_not_checked() {
    let child = r#"
module m #(parameter int Mode = 0) (output logic [7:0] o);
  import p::*;
  s_t r;
  assign r.a = 8'h5a;
  if (Mode == 0) begin : gen_outer
    if (Mode == 1) begin : gen_inner
      assign o = r.missing;
    end else begin : gen_inner_ok
      assign o = r.a;
    end
  end else begin : gen_other
    case (Mode)
      0: begin : gen_never
        assign o = r.also_missing;
      end
      default: begin : gen_case_ok
        assign o = r.a;
      end
    endcase
  end
endmodule
"#;
    let o = lines(&design(child, &top(&[".Mode(0)", ".Mode(1)"])));
    assert!(o.iter().any(|l| l == "O0|5a"), "{o:?}");
    assert!(o.iter().any(|l| l == "O1|5a"), "{o:?}");
}

#[test]
fn member_in_a_selected_branch_is_still_rejected() {
    let err = simulate(&design(IF_CHILD, &top(&[".Full(1'b1)"])), 10)
        .err()
        .expect("a missing member in an elaborated branch must be an error");
    assert!(err.contains("'missing' is not a member"), "{err}");
    // One instance selecting the branch is enough.
    assert!(simulate(&design(IF_CHILD, &top(&[".Full(1'b0)", ".Full(1'b1)"])), 10).is_err());
    let err = simulate(&design(CASE_CHILD, &top(&[".Mode(7)"])), 10)
        .err()
        .expect("a missing member in the elaborated default arm must be an error");
    assert!(err.contains("'also_missing' is not a member"), "{err}");
}
