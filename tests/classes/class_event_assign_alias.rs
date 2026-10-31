//! §15.5.5: an event variable is a handle to a synchronization object, so
//! assigning one class-member event to another makes both names one event.
//! `other = done` in a method of the class, and `p2.e = p1.e` through
//! handles, left each slot with its own event: a trigger of one never woke
//! `@` or `.triggered` on the other. A re-bound slot keeps the object it was
//! given (`t.x = t.y; t.y = t.z;` leaves x holding y's first event).
//! Expected values are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
`timescale 1ns/1ns
class watcher;
  event done, other;
  int a, b;
  task automatic watch();
    other = done;
    fork
      begin wait (other.triggered); a++; end
      begin @(other); b++; end
    join_none
  endtask
endclass
class peer;
  event e;
endclass
class trio;
  event x, y, z;
endclass
module top;
  watcher w = new();
  peer p1 = new(), p2 = new();
  trio t = new();
  int c, d, f, hx, hy, hz;
  initial begin
    w.watch();
    p2.e = p1.e;
    fork
      begin @(p2.e); c++; end
      begin wait (p1.e.triggered); d++; end
    join_none
    #5 -> w.done;
    $display("T|t=%0t other.triggered=%0d done.triggered=%0d", $time, w.other.triggered, w.done.triggered);
    #0 $display("T|t=%0t a=%0d b=%0d", $time, w.a, w.b);
    #5 -> p2.e;
    #0 $display("T|t=%0t c=%0d d=%0d", $time, c, d);
    #5 w.other = null;
    -> w.done;
    #0 $display("T|t=%0t after null a=%0d b=%0d same=%0d", $time, w.a, w.b, w.other == null);
    t.x = t.y;
    t.y = t.z;
    fork
      forever begin @(t.x); hx++; end
      forever begin @(t.y); hy++; end
      forever begin @(t.z); hz++; end
    join_none
    #1 -> t.z;
    #0 $display("T|t=%0t z fired hx=%0d hy=%0d hz=%0d", $time, hx, hy, hz);
    #1 -> t.x;
    #0 $display("T|t=%0t x fired hx=%0d hy=%0d hz=%0d", $time, hx, hy, hz);
    #1 $finish;
  end
endmodule
"#;

#[test]
fn assigned_class_events_share_one_object() {
    let _ = super::compiled_method_test_env::policies(&[("1", "0"), ("0", "0")]);
    let sim = simulate(SRC, 1_000_000).expect("simulate failed");
    let out: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| m.starts_with("T|"))
        .collect();
    assert_eq!(
        out,
        [
            "T|t=5 other.triggered=1 done.triggered=1",
            "T|t=5 a=1 b=1",
            "T|t=10 c=1 d=1",
            "T|t=15 after null a=1 b=1 same=1",
            "T|t=16 z fired hx=0 hy=1 hz=1",
            "T|t=17 x fired hx=1 hy=1 hz=1",
        ]
    );
}
