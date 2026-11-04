//! IEEE 1800-2023 §26.3: a `$unit` wildcard import makes a name potentially
//! locally visible only after it, and the first reference that finds the
//! name through one imports it into the compilation unit. With two packages
//! declaring `mode_t` imported in different files of one unit, every module
//! names the package the first reference found; both imports before a
//! reference are still an error (#620, reported by git-hevangel).
use xezim::simulate_multi;

const PKGS: &str = r#"// Two packages that declare a type with the same name but a different width.
package pkg_one;
  typedef enum logic [1:0] { ONE_A, ONE_B, ONE_C } mode_t;
endpackage

package pkg_two;
  typedef enum logic [3:0] { TWO_A, TWO_B, TWO_C, TWO_D } mode_t;
endpackage
"#;
const UNIT_A: &str = r#"// The first wildcard import in the compilation unit; the first reference to
// mode_t follows it, so it names pkg_one::mode_t (2 bits).
import pkg_one::*;
module probe_a #(parameter mode_t INIT = ONE_B) ();
  localparam int BITS = $bits(mode_t);
endmodule
"#;
const UNIT_B: &str = r#"// A later wildcard import of the other package. mode_t was already made
// locally visible in the compilation unit by the reference in unit_a.sv, so
// it still names pkg_one::mode_t here.
import pkg_two::*;
module probe_b ();
  localparam int BITS = $bits(mode_t);
endmodule
"#;
const UNIT_C: &str = r#"// No import of its own: the name is found in the compilation unit.
module probe_c ();
  localparam int BITS = $bits(mode_t);
endmodule
"#;
const TOP: &str = r#"// IEEE 1800-2023 26.3: a wildcard import makes a name potentially locally
// visible only for references after it. The first reference that finds the
// name there imports it into the scope, where it is locally visible, and a
// later search finds that before it looks at any wildcard import. So with the
// files below compiled in this order as one compilation unit, mode_t is
// pkg_one::mode_t (2 bits) in all three modules, although pkg_two also declares
// it and is wildcard imported before the later two references.
//
//   xezim --simulate -s t --no-cache pkgs.sv unit_a.sv unit_b.sv unit_c.sv top.sv
//
// Every line should print PASS. With the bug elaboration stops with: type
// 'mode_t' is declared by more than one wildcard-imported package.
module t;
  probe_a a();
  probe_b b();
  probe_c c();
  int fails = 0;

  task automatic check(string name, int got, int want);
    if (got == want) $display("PASS %s", name);
    else begin $display("FAIL %s: got %0d, expected %0d", name, got, want); fails++; end
  endtask

  initial begin
    check("probe_a_uses_pkg_one", a.BITS, 2);
    check("probe_b_still_pkg_one", b.BITS, 2);
    check("probe_c_still_pkg_one", c.BITS, 2);
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#;

fn run(files: &[&str]) -> Result<Vec<String>, String> {
    let sources: Vec<String> = files.iter().map(|s| s.to_string()).collect();
    let sim = simulate_multi(
        &sources,
        100,
        Some("t"),
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &[],
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
    )?;
    Ok(sim
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.contains("PASS") || m.contains("FAIL"))
        .collect())
}

#[test]
fn first_reference_binds_the_name_for_the_unit() {
    let out = run(&[PKGS, UNIT_A, UNIT_B, UNIT_C, TOP]).expect("simulate");
    assert!(out.iter().any(|m| m == "ALL PASS"), "{out:?}");
}

/// The other order binds `pkg_two::mode_t` (4 bits) everywhere.
#[test]
fn the_order_of_the_imports_decides() {
    let out = run(&[PKGS, UNIT_B, UNIT_A, UNIT_C, TOP]).expect("simulate");
    assert_eq!(
        out.iter()
            .filter(|m| m.contains("got 4, expected 2"))
            .count(),
        3,
        "{out:?}"
    );
}

#[test]
fn both_imports_before_the_reference_conflict() {
    let src = "import pkg_one::*;\nimport pkg_two::*;\n\
               module t; mode_t m; initial $display(\"%0d\", $bits(m)); endmodule\n";
    let err = run(&[PKGS, src]).err().expect("ambiguous");
    assert!(
        err.contains("more than one wildcard-imported package"),
        "{err}"
    );
}
