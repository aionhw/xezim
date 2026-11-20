//! The delays of a UDP instance count the time unit of the module that
//! holds the instance, like the delays of a gate (IEEE 1800-2023 §3.14.2,
//! §28.16, §29.8): `udp #(4) u(q, a);` under `timescale 1ns/1ps` waits 4 ns,
//! not 4 ticks. Expected values are the reference simulator's, except the
//! instance in the 10 ns module: the reference scales that delay by the time
//! unit of the primitive's own definition; the delay is written in the
//! instantiating module, so its unit applies (as it does for `buf #(2)`
//! there on the reference too).

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let mut v: Vec<String> = simulate(src, 1_000_000)
        .expect("simulate failed")
        .output
        .iter()
        .filter_map(|o| o.message.trim().strip_prefix("CHECK ").map(str::to_string))
        // An undelayed output may or may not show its initial x to an
        // `always @` that starts in the same time step.
        .filter(|l| !l.contains(" x at 0.000"))
        .collect();
    v.sort();
    v
}

const SRC: &str = r#"
`timescale 1ns/1ps
primitive udpbuf(q, a);
  output q; input a;
  table 0 : 0; 1 : 1; endtable
endprimitive
primitive udplat(q, d, en);
  output q; reg q; input d, en;
  table
    0 1 : ? : 0;
    1 1 : ? : 1;
    ? 0 : ? : -;
  endtable
endprimitive
package pk; localparam int D = 4; localparam real RD = 2.5; endpackage
module leaf #(parameter int PD = 3) (input a, output q);
  udpbuf #(PD) u(q, a);
endmodule
`timescale 10ns/1ns
module slow(input a, output q, output qb);
  udpbuf #(2) u(q, a);
  buf #(2) b(qb, a);
endmodule
`timescale 1ns/1ps
module top;
  reg a; reg en = 1;
  localparam LD = 4;
  wire q1, q2, q3, q4, q5, q6, q7, q8, q9, q10, q11, q12, q12b, q13, q14;
  wire [1:0] qa;
  udpbuf #(4) u1(q1, a);
  udpbuf #(4,6) u2(q2, a);
  udpbuf #(2.5) u3(q3, a);
  udpbuf #(4ns) u4(q4, a);
  udpbuf #(pk::D + 1) u5(q5, a);
  udpbuf #(LD) u6(q6, a);
  udpbuf #(0) u7(q7, a);
  udpbuf u8(q8, a);
  udpbuf #(pk::RD) u9(q9, a);
  udplat #(3) u10(q10, a, en);
  leaf #(.PD(7)) u11(a, q11);
  slow u12(a, q12, q12b);
  udpbuf #(1.2345) u13(q13, a);
  udpbuf #(1500ps, 2) u14(q14, a);
  udpbuf #(3) ua0(qa[0], a), ua1(qa[1], a);
  generate if (1) begin : g
    wire gq; udpbuf #(LD*2) ug(gq, a);
  end endgenerate
  `define MON(n, s) always @(s) $display("CHECK %s %b at %0.3f", n, s, $realtime);
  `MON("q01", q1) `MON("q02", q2) `MON("q03", q3) `MON("q04", q4) `MON("q05", q5)
  `MON("q06", q6) `MON("q07", q7) `MON("q08", q8) `MON("q09", q9) `MON("q10", q10)
  `MON("q11", q11) `MON("q12", q12) `MON("q12b", q12b) `MON("q13", q13) `MON("q14", q14)
  `MON("qa", qa) `MON("gq", g.gq)
  initial begin
    a = 0;
    #100 a = 1;
    #100 a = 0;
    #100 $finish;
  end
endmodule
"#;

#[test]
fn udp_instance_delays_count_the_holding_modules_time_unit() {
    let mut expected: Vec<String> = Vec::new();
    // (name, rise delay, fall delay) in ns; the first change is the fall
    // from x at time 0.
    for (name, rise, fall) in [
        ("q01", 4.0, 4.0),
        ("q02", 4.0, 6.0),
        ("q03", 2.5, 2.5),
        ("q04", 4.0, 4.0),
        ("q05", 5.0, 5.0),
        ("q06", 4.0, 4.0),
        ("q07", 0.0, 0.0),
        ("q08", 0.0, 0.0),
        ("q09", 2.5, 2.5),
        ("q10", 3.0, 3.0),
        ("q11", 7.0, 7.0),
        ("q12", 20.0, 20.0),
        ("q12b", 20.0, 20.0),
        ("q13", 1.235, 1.235),
        ("q14", 1.5, 2.0),
        ("gq", 8.0, 8.0),
    ] {
        expected.push(format!("{name} 0 at {:.3}", fall));
        expected.push(format!("{name} 1 at {:.3}", 100.0 + rise));
        expected.push(format!("{name} 0 at {:.3}", 200.0 + fall));
    }
    expected.push("qa 00 at 3.000".into());
    expected.push("qa 11 at 103.000".into());
    expected.push("qa 00 at 203.000".into());
    expected.sort();
    assert_eq!(lines(SRC), expected);
}

/// The same instance delays under a 1 ns precision keep their values: a
/// time unit that is the tick needs no scaling.
#[test]
fn udp_instance_delays_at_tick_time_unit() {
    let src = SRC
        .replace("`timescale 1ns/1ps", "`timescale 1ns/1ns")
        .replace("udpbuf #(2.5) u3(q3, a);", "udpbuf #(3) u3(q3, a);")
        .replace("udpbuf #(pk::RD) u9(q9, a);", "udpbuf #(3) u9(q9, a);")
        .replace("udpbuf #(1.2345) u13(q13, a);", "udpbuf #(1) u13(q13, a);")
        .replace(
            "udpbuf #(1500ps, 2) u14(q14, a);",
            "udpbuf #(1, 2) u14(q14, a);",
        );
    let got = lines(&src);
    for want in [
        "q01 1 at 104.000",
        "q02 0 at 206.000",
        "q03 1 at 103.000",
        "q05 1 at 105.000",
        "q11 1 at 107.000",
        "q12 1 at 120.000",
        "q12b 1 at 120.000",
        "q14 0 at 202.000",
        "gq 1 at 108.000",
    ] {
        assert!(got.iter().any(|l| l == want), "missing {want}: {got:?}");
    }
}
