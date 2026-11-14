//! §10.4.2, §6.12, §8.25: a nonblocking update of a `real`, `shortreal` or
//! `realtime` class property keeps its real value, through a handle, inside a
//! method, as a static and as a type parameter the object binds to `real`; an
//! integral right-hand side is self-determined for it (#1050). Expected
//! values are the reference simulator's.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).unwrap_or_else(|e| panic!("rejected: {e}"));
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[test]
fn real_class_property_targets_of_nonblocking_updates() {
    assert_eq!(
        lines(
            r#"class P #(type T = int); T v; endclass
class R;
  real r; shortreal sr; realtime rtm; logic [63:0] w64; int unsigned u;
  static real sr0;
  task nb(); r <= -2.25; this.sr <= 0.5; w64 <= 64'hFFFF_0000_1234_5678; endtask
endclass
module t;
  R h; P #(real) pr; P #(int) pi; int unsigned u = 1;
  initial begin
    h = new; pr = new; pi = new;
    h.r <= -1.5; h.sr <= 2.5; h.rtm <= 3.75;
    pr.v <= -7.5; pi.v <= -7.5;
    R::sr0 <= 1.25;
    h.w64 <= u - 2;
    h.u <= u - 2;
    #1;
    $display("A r=%f sr=%f rtm=%f pr=%f pi=%0d sr0=%f", h.r, h.sr, h.rtm, pr.v, pi.v, R::sr0);
    $display("B w64=%h u=%h", h.w64, h.u);
    h.nb();
    #1 $display("C r=%f sr=%f w64=%h", h.r, h.sr, h.w64);
    h.r <= u - 2;
    pr.v <= u - 2;
    #1 $display("D r=%f pr=%f", h.r, pr.v);
    $finish;
  end
endmodule
"#
        ),
        [
            r#"A r=-1.500000 sr=2.500000 rtm=3.750000 pr=-7.500000 pi=-8 sr0=1.250000"#,
            r#"B w64=ffffffffffffffff u=ffffffff"#,
            r#"C r=-2.250000 sr=0.500000 w64=ffff000012345678"#,
            r#"D r=4294967295.000000 pr=4294967295.000000"#,
        ]
    );
}
