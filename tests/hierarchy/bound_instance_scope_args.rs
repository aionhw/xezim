//! IEEE 1800-2023 §23.11, §20.12, §21.7.1.2: a module may name an instance
//! that a `bind` adds to it as the scope argument of an assertion-control or
//! dump task (aionhw/xezim#721).
use xezim::simulate_multi;

fn run(source: &str) -> Result<Vec<String>, String> {
    let sim = simulate_multi(
        &[source.to_string()],
        1000,
        Some("top"),
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
    let mut lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|line| line.starts_with("T|"))
        .collect();
    lines.sort();
    Ok(lines)
}

const CHK: &str = r###"module chk(input logic clk, input logic a);
  ap: assert property (@(posedge clk) a) else $display("T| CHK FAIL %m t=%0t", $time);
endmodule
"###;

/// `body` is the item list of the bind target `dut`; `binds` follows the
/// definitions, and `in_top` is written inside the top module.
fn design(body: &str, binds: &str, in_top: &str) -> String {
    format!(
        r###"{CHK}module dut(input logic clk, input logic a);
{body}
endmodule
{binds}
module top; logic clk=0, a=1; always #5 clk=~clk; dut d0(.*);
{in_top}
  initial begin #30 $display("T| done"); $finish; end
endmodule
"###
    )
}

const UNIT_BIND: &str = "bind dut chk u_chk(.clk(clk), .a(a));";

#[test]
fn controls_act_on_each_instances_bound_checker() {
    // Expected lines are the reference simulator's, sorted.
    let lines = run(r###"module chk(input logic clk, input logic a);
  ap: assert property (@(posedge clk) a) else $display("T| CHK FAIL %m t=%0t", $time);
endmodule
module dut(input logic clk, input logic a);
  ap_own: assert property (@(posedge clk) a) else $display("T| DUT FAIL %m t=%0t", $time);
  initial begin
    $assertoff(0, u_chk);
    #22 $asserton(0, u_chk);
    #20 $assertkill(0, u_chk);
    #20 $asserton(0, u_chk);
  end
endmodule
bind dut chk u_chk(.clk(clk), .a(a));
module top; logic clk=0, a=0; always #5 clk=~clk; dut d0(.*); dut d1(.*);
  initial begin #85 $display("T| done"); $finish; end
endmodule
"###)
    .expect("assertion controls on a bound instance");
    let expected = [
        "T| CHK FAIL top.d0.u_chk.ap t=25",
        "T| CHK FAIL top.d0.u_chk.ap t=35",
        "T| CHK FAIL top.d0.u_chk.ap t=65",
        "T| CHK FAIL top.d0.u_chk.ap t=75",
        "T| CHK FAIL top.d1.u_chk.ap t=25",
        "T| CHK FAIL top.d1.u_chk.ap t=35",
        "T| CHK FAIL top.d1.u_chk.ap t=65",
        "T| CHK FAIL top.d1.u_chk.ap t=75",
        "T| DUT FAIL top.d0.ap_own t=15",
        "T| DUT FAIL top.d0.ap_own t=25",
        "T| DUT FAIL top.d0.ap_own t=35",
        "T| DUT FAIL top.d0.ap_own t=45",
        "T| DUT FAIL top.d0.ap_own t=5",
        "T| DUT FAIL top.d0.ap_own t=55",
        "T| DUT FAIL top.d0.ap_own t=65",
        "T| DUT FAIL top.d0.ap_own t=75",
        "T| DUT FAIL top.d1.ap_own t=15",
        "T| DUT FAIL top.d1.ap_own t=25",
        "T| DUT FAIL top.d1.ap_own t=35",
        "T| DUT FAIL top.d1.ap_own t=45",
        "T| DUT FAIL top.d1.ap_own t=5",
        "T| DUT FAIL top.d1.ap_own t=55",
        "T| DUT FAIL top.d1.ap_own t=65",
        "T| DUT FAIL top.d1.ap_own t=75",
        "T| done",
    ];
    assert_eq!(lines, expected);
}

#[test]
fn every_bind_form_and_process_kind_elaborates() {
    let restart = "$assertkill(0, u_chk); $asserton(0, u_chk);";
    let cases: [(&str, String); 9] = [
        ("unit bind, initial", design(&format!("  initial begin {restart} end"), UNIT_BIND, "")),
        ("unit bind, always", design(&format!("  always @(posedge clk) begin {restart} end"), UNIT_BIND, "")),
        ("unit bind, task", design(&format!("  task automatic t(); {restart} endtask\n  initial t();"), UNIT_BIND, "")),
        ("unit bind, function", design("  function void f(); $asserton(0, u_chk); endfunction\n  initial f();", UNIT_BIND, "")),
        ("unit bind, fork", design("  initial fork $assertkill(0, u_chk); join_none", UNIT_BIND, "")),
        ("unit bind, dump scope", design("  initial $dumpvars(0, u_chk);", UNIT_BIND, "")),
        ("unit bind, other controls", design("  initial begin $assertoff(0, u_chk); $assertcontrol(3, 15, 7, 0, u_chk); $assertpassoff(0, u_chk); $assertfailon(0, u_chk); end", UNIT_BIND, "")),
        ("bind to an instance path", design(&format!("  initial begin {restart} end"), "", "") + "bind top.d0 chk u_chk(.clk(clk), .a(a));\n"),
        ("bind inside the top module", design(&format!("  initial begin {restart} end"), "", "  bind dut chk u_chk(.clk(clk), .a(a));")),
    ];
    for (what, source) in cases {
        let lines = run(&source).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert_eq!(lines, ["T| done"], "{what}");
    }
}

#[test]
fn macro_branch_passing_a_bound_instance_as_scope() {
    let source = format!(
        r###"`define ASSERT_ON
`ifdef ASSERT_ON
 `define ASSERT_RESTART(scope) begin $assertkill(0, scope); $asserton(0, scope); end
`else
 `define ASSERT_RESTART(scope) begin end
`endif
{CHK}module dut(input logic clk, input logic a, input logic rst);
  always @(negedge rst) `ASSERT_RESTART(u_chk)
endmodule
module top; logic clk=0, a=1, rst=1; always #5 clk=~clk; dut d0(.*);
  bind dut chk u_chk(.clk(clk), .a(a));
  initial begin #12 rst=0; #30 $display("T| done"); $finish; end
endmodule
"###
    );
    assert_eq!(run(&source).expect("macro scope argument"), ["T| done"]);
}

#[test]
fn unknown_scope_and_instance_as_value_stay_errors() {
    let unknown = design("  initial $asserton(0, no_such_thing);", UNIT_BIND, "");
    let err = run(&unknown).expect_err("a scope nothing declares");
    assert!(err.contains("Undeclared identifier 'no_such_thing'"), "{err}");
    let value = design("  initial $display(\"%0d\", u_chk);", UNIT_BIND, "");
    let err = run(&value).expect_err("an instance is not a value");
    assert!(err.contains("Undeclared identifier 'u_chk'"), "{err}");
}
