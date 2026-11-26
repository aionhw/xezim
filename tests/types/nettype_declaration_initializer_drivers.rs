//! IEEE 1800-2023 §6.6.7, §10.3.1: the declaration initializer of a
//! user-defined nettype net is one of the net's continuous drivers, seen by
//! the resolution function and re-evaluated when it reads a variable that
//! changes, whether the initializer is constant or not and whether the net is
//! declared at top level or in an instantiated module (#1204). Expected values
//! are the reference simulator's; the resolver adds 100 per driver.
#[test]
fn nettype_declaration_initializers_are_drivers() {
    let sim = xezim::simulate(SRC, 100).expect("simulate nettype declaration initializers");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|t1 tm=101 tv=103 tw=204 cm=101 cv=102",
            "T|t2 tm=101 tv=105 tw=206 cm=101 cv=102",
        ]
    );
}

const SRC: &str = r#"`timescale 1ns/1ps
// IEEE 1800-2023 §6.6.7, §10.3.1: a nettype net's declaration initializer is one
// of its continuous drivers, constant or not, at top level and in a child (#1204).
function automatic real cnt(input real d[]);
  cnt = 100.0 * d.size();
  foreach (d[i]) cnt += d[i];
endfunction
nettype real C with cnt;
module child(output real o1, output real o2);
  real src = 2.0;
  C cm = 1.0;           // constant initializer in a child
  C cv = src;           // non-constant initializer in a child
  assign o1 = cm;
  assign o2 = cv;
endmodule
module top;
  real a = 3.0, c1, c2;
  C tm = 1.0;           // constant initializer, no other driver
  C tv = a;             // non-constant initializer
  C tw = 1.0;           // constant initializer plus a driver
  assign tw = a;
  child u(c1, c2);
  initial begin
    #1 $display("T|t1 tm=%g tv=%g tw=%g cm=%g cv=%g", tm, tv, tw, c1, c2);
    a = 5.0;
    #1 $display("T|t2 tm=%g tv=%g tw=%g cm=%g cv=%g", tm, tv, tw, c1, c2);
  end
endmodule
"#;
