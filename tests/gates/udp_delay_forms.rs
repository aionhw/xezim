//! IEEE 1800-2017 §29.3 / §28.16 / A.2.2.3: the delay of a UDP instance may
//! be a parameter name (`#D`, with or without an instance name), a
//! `min:typ:max` value, or a `(rise, fall)` pair. `#D` was taken for the
//! instance NAME (`buf1 #D (z, a)` became an undelayed instance called `D`,
//! with no message), `#(1:2:3)` was a parse error, and the fall value of a
//! pair was ignored (every change waited the rise delay). Expected values
//! from the reference simulator.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 1_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with('t'))
        .collect()
}

#[test]
fn udp_delay_from_a_parameter_and_min_typ_max() {
    let got = lines(
        "`timescale 1ns/1ns
primitive buf1 (z, a); output z; input a; table 0 : 0; 1 : 1; endtable endprimitive
module t;
  localparam int D = 2;
  reg a;
  wire z_id, z_idn, z_mtm;
  buf1 #D        (z_id, a);
  buf1 #D  u_idn (z_idn, a);
  buf1 #(1:2:3)  (z_mtm, a);
  initial begin
    a = 0; #10 a = 1;
    #1 $display(\"t11 %b %b %b\", z_id, z_idn, z_mtm);
    #2 $display(\"t13 %b %b %b\", z_id, z_idn, z_mtm);
    $finish;
  end
endmodule
",
    );
    assert_eq!(got, ["t11 0 0 0", "t13 1 1 1"]);
}

#[test]
fn udp_rise_and_fall_delays() {
    let got = lines(
        "`timescale 1ns/1ns
primitive buf1 (z, a); output z; input a; table 0 : 0; 1 : 1; x : x; endtable endprimitive
module t;
  reg a; wire z, zu;
  buf1 #(2, 5) u (z, a);
  buf1 #(2, 5)   (zu, a);
  initial begin
    a = 0; #10 a = 1; #3 $display(\"t13 %b %b\", z, zu);
    a = 0; #4 $display(\"t17 %b %b\", z, zu);
    #2 $display(\"t19 %b %b\", z, zu);
    a = 1'bx; #3 $display(\"t22 %b %b\", z, zu);
    $finish;
  end
endmodule
",
    );
    assert_eq!(got, ["t13 1 1", "t17 1 1", "t19 0 0", "t22 x x"]);
}
