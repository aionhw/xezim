//! IEEE 1800-2017 §26.2, §6.6, §6.7: a net declared in a package is a net:
//! an undriven one reads z, a supply net its constant, a wired net resolves
//! its drivers (a `wand` driven by `assign pkg::w` and by a port connected to
//! `pkg::w`), and a force overrides it (#610, reported by git-hevangel; it
//! extends #602).
use xezim::simulate;

fn shown(src: &str, f: impl Fn(&str) -> bool) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| f(m))
        .collect()
}

#[test]
fn reported_testbench() {
    let out = shown(
        r#"// IEEE 1800-2017 26.2: a package may hold net declarations (net_declaration is
// one of the package_or_generate_item_declaration items), and 26.3 lets code
// refer to a package item by name, `pkg::name`, or through an import. A net
// declared in a package is one net for the whole design: an undriven one reads
// z at its declared width, a supply net reads its constant, a continuous
// assignment drives it from anywhere, and a force overrides it.
//
// Every line should print PASS.
// With the bug the file does not compile (`wire [consts::W-1:0] ...`).
package consts;
  parameter int W = 8;
endpackage

package nets;
  wire [consts::W-1:0] floating;        // range written with another package's constant
  wire [3:0]           driven;
  wire logic [3:0]     typed;           // explicit data type (6.7)
  wire signed [3:0]    sgn;
  supply1 [1:0]        vdd;
  supply0              gnd;
  wand [1:0]           both;
endpackage

module drv_a(output wire [1:0] o);
  assign o = 2'b10;
endmodule

module t;
  import nets::*;
  int fails = 0;

  task automatic check(string label, bit ok);
    if (ok) $display("PASS %s", label);
    else begin $display("FAIL %s", label); fails++; end
  endtask

  // continuous assignments by qualified name, from a module, and from an output port
  assign nets::driven = 4'b1010;
  assign nets::sgn    = 4'b1000;        // -8 when the net is signed
  assign nets::both   = 2'b11;
  drv_a u_a (.o(nets::both));           // wand: 2'b11 & 2'b10

  initial begin
    #1;
    check("range uses a constant of another package", $bits(nets::floating) == 8);
    check("undriven package net reads z",             nets::floating === 8'bzzzzzzzz);
    check("undriven typed net reads z",               nets::typed === 4'bzzzz);
    check("supply1 reads 1",                          nets::vdd === 2'b11);
    check("supply0 reads 0",                          nets::gnd === 1'b0);
    check("continuous assignment by qualified name",  nets::driven === 4'b1010);
    check("bare name through import nets::*",         driven === 4'b1010);
    check("signed package net is signed",             nets::sgn < 0);
    check("wand resolves two drivers",                nets::both === 2'b10);

    force nets::floating = 8'h5a;
    #1 check("force overrides a package net",         nets::floating === 8'h5a);

    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#,
        |m| m.contains("PASS") || m.contains("FAIL"),
    );
    assert!(out.iter().any(|m| m == "ALL PASS"), "{out:?}");
    assert!(!out.iter().any(|m| m.contains("FAIL")), "{out:?}");
}

#[test]
fn wand_package_net_resolves_its_drivers() {
    assert_eq!(
        shown(
            r#"package nets; wand [1:0] both; endpackage
module drv(output logic [1:0] o); assign o = 2'b10; endmodule
module t;
  wand [1:0] lw;
  assign lw = 2'b11; assign lw = 2'b10;
  assign nets::both = 2'b11;
  drv u (.o(nets::both));
  initial #1 $display("R %b %b", nets::both, lw);
endmodule
"#,
            |m| m.starts_with("R ")
        ),
        ["R 10 10"]
    );
}
