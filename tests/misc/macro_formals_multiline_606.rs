//! IEEE 1800-2017 §22.5.1, §5.2: a macro's formal argument list may span
//! lines; white space includes newlines and no backslash is needed inside
//! the list. The definition's header ended at its first line, so the rest of
//! the list was read as code (#606, reported by amal-khailtash).
use xezim::simulate;

fn shown(src: &str, prefixes: &[&str]) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| prefixes.iter().any(|p| m.starts_with(p)))
        .collect()
}

/// Defaults, three lines, and `` `__LINE__ `` after the definitions.
#[test]
fn formals_over_several_lines() {
    let src = r#"`define ADD(a,
            b) (a + b)
`define MUL(x = 2,
            y = 3,
            z
           ) (x * y * z)
`define SHOW(msg,
             val) $display("%s %0d", msg, val);
module top;
  initial begin
    `SHOW("A", `ADD(1, 2))
    `SHOW("B", `MUL(, , 4))
    `SHOW("C", `MUL(1, 1, 5))
    $display("L %0d", `__LINE__);
  end
endmodule
"#;
    assert_eq!(
        shown(src, &["A ", "B ", "C ", "L "]),
        ["A 3", "B 24", "C 5", "L 14"]
    );
}

/// The report's bind macro.
#[test]
fn reported_bind_macro() {
    let src = r#"`define ADD_PROBE(target,
                  probe_name) \
  bind target observer probe_name(.flag(flag));
module dut; logic flag = 1; endmodule
module observer(input logic flag); initial #1 $display("OBS %m %b", flag); endmodule
module top; dut u0(); endmodule
`ADD_PROBE(top.u0, obs)

module chk; initial #1 $display("BOUND %m"); endmodule
"#;
    assert_eq!(shown(src, &["OBS "]), ["OBS top.u0.obs 1"]);
}
