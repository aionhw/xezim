//! §31.7: the comparisons of a conditioned timing check are deterministic
//! (none, `~`, `===`, `!==`) or nondeterministic (`==`, `!=`). An x or z on
//! the conditioning signal disables a deterministic one and enables a
//! nondeterministic one, whatever the comparison evaluates to: `en == 1'b1`
//! with `en` at x enables the check and `en !== 1'b1` does not. The
//! violations are the reference simulator's. Both used to follow the value
//! of the comparison, so an x disabled every condition but `!==` (#1246).

use xezim::simulate;

#[test]
fn x_and_z_conditions_of_each_comparison() {
    let src = r#"
`timescale 1ns/1ps
module tchk (input wire ck, input wire d, input wire en, input wire s);
  reg n1, n2, n3, n4, n5, n6, n7, n8;
  specify
    $setup(d, posedge ck &&& en, 2, n1);
    $setup(d, posedge ck &&& ~en, 2, n2);
    $setup(d, posedge ck &&& (en == 1'b1), 2, n3);
    $hold(posedge ck &&& (en != 1'b1), d, 2, n4);
    $hold(posedge ck &&& (en === 1'b0), d, 2, n5);
    $hold(posedge ck &&& (en !== 1'b1), d, 2, n6);
    $width(posedge ck &&& (s == 1), 3, 0, n7);
    $setup(d &&& (s !== 'b0), posedge ck, 2, n8);
  endspecify
endmodule
module tb;
  reg ck = 0, d = 0, en = 0, s = 0;
  tchk u (.ck(ck), .d(d), .en(en), .s(s));
  task automatic pulse(input e, input ss);
    en = e; s = ss;
    #5 d = ~d;
    #1 ck = 1;
    #1 d = ~d;
    #1 ck = 0;
    #2;
  endtask
  initial begin
    #10;
    pulse(1'b0, 1'b0);
    pulse(1'b1, 1'b1);
    pulse(1'bx, 1'b0);
    pulse(1'bz, 1'bx);
    pulse(1'b1, 1'bz);
    pulse(1'b0, 1'b1);
    $display("T|done");
    $finish;
  end
endmodule
"#;
    let res = simulate(src, 1_000_000).expect("simulate failed");
    let got: Vec<String> = res
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("** Error:"))
        .map(|l| {
            let at = l.split(" at time ").nth(1).unwrap_or("");
            let at = at.split(" (").next().unwrap_or(at);
            format!("{} @ {}", l.split(" violation in ").next().unwrap(), at)
        })
        .collect();
    assert_eq!(
        got,
        [
            "** Error: $setup( d:15 ns, posedge ck &&& ~en:16 ns, 2 ns ) @ 16 ns",
            "** Error: $hold( posedge ck &&& (en != 1'b1):16 ns, d:17 ns, 2 ns ) @ 17 ns",
            "** Error: $hold( posedge ck &&& (en === 1'b0):16 ns, d:17 ns, 2 ns ) @ 17 ns",
            "** Error: $hold( posedge ck &&& (en !== 1'b1):16 ns, d:17 ns, 2 ns ) @ 17 ns",
            "** Error: $setup( d:25 ns, posedge ck &&& en:26 ns, 2 ns ) @ 26 ns",
            "** Error: $setup( d:25 ns, posedge ck &&& (en == 1'b1):26 ns, 2 ns ) @ 26 ns",
            "** Error: $setup( d &&& (s !== 'b0):25 ns, posedge ck:26 ns, 2 ns ) @ 26 ns",
            "** Error: $width( posedge ck &&& (s == 1):26 ns, negedge ck:28 ns, 3 ns ) @ 28 ns",
            "** Error: $setup( d:35 ns, posedge ck &&& (en == 1'b1):36 ns, 2 ns ) @ 36 ns",
            "** Error: $hold( posedge ck &&& (en != 1'b1):36 ns, d:37 ns, 2 ns ) @ 37 ns",
            "** Error: $setup( d:45 ns, posedge ck &&& (en == 1'b1):46 ns, 2 ns ) @ 46 ns",
            "** Error: $hold( posedge ck &&& (en != 1'b1):46 ns, d:47 ns, 2 ns ) @ 47 ns",
            "** Error: $width( posedge ck &&& (s == 1):46 ns, negedge ck:48 ns, 3 ns ) @ 48 ns",
            "** Error: $setup( d:55 ns, posedge ck &&& en:56 ns, 2 ns ) @ 56 ns",
            "** Error: $setup( d:55 ns, posedge ck &&& (en == 1'b1):56 ns, 2 ns ) @ 56 ns",
            "** Error: $width( posedge ck &&& (s == 1):56 ns, negedge ck:58 ns, 3 ns ) @ 58 ns",
            "** Error: $setup( d:65 ns, posedge ck &&& ~en:66 ns, 2 ns ) @ 66 ns",
            "** Error: $setup( d &&& (s !== 'b0):65 ns, posedge ck:66 ns, 2 ns ) @ 66 ns",
            "** Error: $hold( posedge ck &&& (en != 1'b1):66 ns, d:67 ns, 2 ns ) @ 67 ns",
            "** Error: $hold( posedge ck &&& (en === 1'b0):66 ns, d:67 ns, 2 ns ) @ 67 ns",
            "** Error: $hold( posedge ck &&& (en !== 1'b1):66 ns, d:67 ns, 2 ns ) @ 67 ns",
            "** Error: $width( posedge ck &&& (s == 1):66 ns, negedge ck:68 ns, 3 ns ) @ 68 ns",
        ]
    );
}
