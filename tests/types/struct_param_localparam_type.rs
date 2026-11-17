//! §6.20.3 / §11.5.1: an inlined localparam type folds packed-struct
//! parameter members before function formals and casts use its range.
use xezim::simulate;

const DECLARATIONS: &str = r#"
package cfg_pkg;
  typedef struct packed { int unsigned W; int unsigned N; } cfg_t;
  function automatic cfg_t build(int unsigned x);
    cfg_t c; c.W = x; c.N = 128; return c;
  endfunction
  parameter cfg_t CFG = build(64);
endpackage
module child #(parameter cfg_pkg::cfg_t Cfg = cfg_pkg::build(64));
  localparam type word_t = logic [Cfg.W-1:0];
  function automatic int sel(word_t word);
    $display("left=%0d s70=%h b6=%b", $left(word), word[7:0], word[6]);
    sel = int'(word[2:0]);
  endfunction
  initial if (sel(word_t'(11'hc1)) == 1) $display("PASS");
          else $display("FAIL");
endmodule
"#;

fn output(top: &str) -> Vec<String> {
    simulate(
        &format!("{DECLARATIONS}\nmodule top; {top} endmodule"),
        1000,
    )
    .expect("simulate")
    .output
    .iter()
    .map(|line| line.message.clone())
    .collect()
}

#[test]
fn struct_parameter_member_sizes_localparam_type_formals() {
    for instance in [
        "child #(.Cfg(cfg_pkg::CFG)) u();",
        "child #(.Cfg(cfg_pkg::cfg_t'{W:64, N:128})) u();",
        "child #(.Cfg(cfg_pkg::build(64))) u();",
        "child u();",
    ] {
        assert_eq!(
            output(instance),
            ["left=63 s70=c1 b6=1", "PASS"],
            "{instance}"
        );
    }
}

#[test]
fn separate_instances_keep_their_own_type_bounds() {
    let mut rows =
        output("child #(.Cfg(cfg_pkg::build(9))) lo(); child #(.Cfg(cfg_pkg::CFG)) hi();");
    rows.sort();
    assert_eq!(
        rows,
        ["PASS", "PASS", "left=63 s70=c1 b6=1", "left=8 s70=c1 b6=1"]
    );
}
