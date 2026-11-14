//! Loop jumps before a delay and `disable` of blocks (IEEE 1800-2023
//! §9.6.2, §12.8), found reviewing #1068. Every expected line is the
//! reference simulator's output.

use xezim::simulate;

/// Every line `src` prints, in output order.
fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).unwrap_or_else(|e| panic!("rejected: {e}"));
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .collect()
}

/// §12.8: `break` and `continue` ahead of the delay in `while`, `repeat`,
/// `do`-`while`, `foreach` and `for` bodies, and in a `forever` whose body
/// waits on an event or a `wait`; nested `forever` loops; a `return` and a
/// `disable` of the task from a `forever` in a task; a jump inside nested
/// blocks and a `case`; `disable` of fork branches and of a labelled loop.
#[test]
fn jumps_before_the_delay_in_every_loop_kind() {
    let got = lines(
        r#"
module top;
  int n, x, y, i, q[$];
  int arr[4] = '{1,2,3,4};
  event ev; logic clk = 0;
  always #1 clk = ~clk;
  task automatic tk(output int r);
    r = 0;
    forever begin r++; if (r == 3) return; #1; end
  endtask
  task tk2; forever begin : tb y++; if (y == 4) disable tk2; #1; end endtask
  function automatic int fn(); int k = 0; forever begin k++; if (k == 7) return k; end endfunction
  initial begin
    // while with jump before delay
    n = 0; while (1) begin n++; if (n == 3) break; #1; end
    $display("A while-break n=%0d t=%0t", n, $time);
    n = 0; x = 0; while (n < 6) begin n++; if (n < 3) continue; x++; #1; end
    $display("B while-continue n=%0d x=%0d t=%0t", n, x, $time);
    n = 0; repeat (10) begin n++; if (n == 4) break; #1; end
    $display("C repeat-break n=%0d t=%0t", n, $time);
    n = 0; x = 0; repeat (6) begin n++; if (n % 2) continue; x++; #1; end
    $display("D repeat-continue n=%0d x=%0d t=%0t", n, x, $time);
    n = 0; do begin n++; if (n == 2) break; #1; end while (n < 10);
    $display("E do-break n=%0d t=%0t", n, $time);
    n = 0; x = 0; do begin n++; if (n < 4) continue; x++; #1; end while (n < 6);
    $display("F do-continue n=%0d x=%0d t=%0t", n, x, $time);
    n = 0; foreach (arr[k]) begin n += arr[k]; if (k == 2) break; #1; end
    $display("G foreach-break n=%0d t=%0t", n, $time);
    n = 0; foreach (arr[k]) begin if (k == 1) continue; n += arr[k]; #1; end
    $display("H foreach-continue n=%0d t=%0t", n, $time);
    n = 0; for (i = 0; i < 10; i++) begin n++; if (i == 3) break; #1; end
    $display("I for-break n=%0d i=%0d t=%0t", n, i, $time);
    n = 0; for (i = 0; i < 6; i++) begin if (i[0]) continue; n++; #1; end
    $display("J for-continue n=%0d i=%0d t=%0t", n, i, $time);
    // forever with event control later
    n = 0; forever begin n++; if (n == 3) break; @(posedge clk); end
    $display("K forever-break-event n=%0d t=%0t", n, $time);
    n = 0; x = 0; forever begin n++; if (n == 6) break; if (n < 3) continue; x++; @(posedge clk); end
    $display("L forever-cont-event n=%0d x=%0d t=%0t", n, x, $time);
    // forever with wait
    n = 0; forever begin n++; if (n == 3) break; wait (clk); #1; end
    $display("M forever-break-wait n=%0d t=%0t", n, $time);
    // nested forever
    n = 0; x = 0;
    forever begin
      n++;
      forever begin x++; if (x % 3 == 0) break; #1; end
      if (n == 3) break;
      #1;
    end
    $display("N nested n=%0d x=%0d t=%0t", n, x, $time);
    // disable named forever body before delay
    n = 0; x = 0;
    // return from task
    tk(n); $display("P task-return n=%0d t=%0t", n, $time);
    y = 0; tk2; $display("Q task-disable y=%0d t=%0t", y, $time);
    $display("R fn=%0d", fn());
    // break inside nested if/begin before delay
    n = 0; forever begin n++; begin if (n > 2) begin if (n == 4) break; end end #1; end
    $display("S nested-break n=%0d t=%0t", n, $time);
    // break in case
    n = 0; forever begin n++; case (n) 3: break; default: ; endcase #1; end
    $display("T case-break n=%0d t=%0t", n, $time);
    // disable outer block from nested forever in fork
    n = 0; x = 0;
    fork : fk
      begin : b1 forever begin n++; if (n == 3) disable b1; #1; end end
      begin : b2 forever begin x++; if (x == 5) disable b2; #1; end end
    join
    $display("U fork-disable n=%0d x=%0d t=%0t", n, x, $time);
    n = 0;
    fork
      forever begin n++; if (n == 4) disable fork_done; #1; end
    join_none
    begin : fork_done #20; end
    $display("V n=%0d t=%0t", n, $time);
    // labeled statement loop
    n = 0;
    lp: forever begin n++; if (n == 3) disable lp; #1; end
    $display("W labeled-forever n=%0d t=%0t", n, $time);
    $finish;
  end
  initial #500 begin $display("TIMEOUT"); $finish; end
endmodule
"#,
    );
    assert_eq!(
        got,
        [
            "A while-break n=3 t=2",
            "B while-continue n=6 x=4 t=6",
            "C repeat-break n=4 t=9",
            "D repeat-continue n=6 x=3 t=12",
            "E do-break n=2 t=13",
            "F do-continue n=6 x=3 t=16",
            "G foreach-break n=6 t=18",
            "H foreach-continue n=8 t=21",
            "I for-break n=4 i=3 t=24",
            "J for-continue n=3 i=6 t=27",
            "K forever-break-event n=3 t=31",
            "L forever-cont-event n=6 x=3 t=37",
            "M forever-break-wait n=3 t=40",
            "N nested n=3 x=9 t=48",
            "P task-return n=3 t=50",
            "Q task-disable y=4 t=53",
            "R fn=7",
            "S nested-break n=4 t=56",
            "T case-break n=3 t=58",
            "U fork-disable n=3 x=5 t=62",
            "V n=4 t=65",
            "W labeled-forever n=3 t=66",
        ]
    );
}

/// §9.6.2: a named `always` body disabled before its delay, from a nested
/// block and from another process; a named block inside an unnamed `always`
/// body; a process's top-level block disabled with a fork child running, and
/// from another process; `%m` in a top-level block and a named `forever`
/// body; a named block under an event control.
#[test]
fn disable_of_always_and_top_level_blocks() {
    let got = lines(
        r#"
module top;
  int n, x, y, z, a, b, c, d;
  // always with disable/jump before delay
  always begin : ab n++; if (n % 2) disable ab; x++; #1; end
  always begin : ac #1 y++; begin : inner if (y == 2) disable ac; end z++; end
  // unnamed always + named inner
  always begin begin : in2 a++; if (a < 3) disable in2; b++; end #1; end
  // disable from another process
  always begin : ad #3 c++; #3 d++; end
  initial begin #4 disable ad; #6 disable ad; end
  // top-level block label with fork children
  int f1, f2, f3;
  initial begin : blk
    fork #5 f1 = 1; join_none
    #1 f2 = 1;
    disable blk;
    f3 = 1;
  end
  // disable top-level from another process
  int g1, g2;
  initial begin : gb #2 g1 = 1; #10 g2 = 1; end
  initial #5 disable gb;
  // %m in named forever body and top-level
  initial begin : mb $display("m1=%m"); begin : mi $display("m2=%m"); end end
  initial begin forever begin : fm #7 $display("m3=%m"); break; end end
  // final/always_ff named
  logic clk = 0; always #1 clk = ~clk;
  int h1, h2;
  always @(posedge clk) begin : pe h1++; if (h1 % 2) disable pe; h2++; end
  // disable own top-level from nested task
  // local variable in named always body
  // automatic variable in named forever body
  initial #20 begin
    $display("ab n=%0d x=%0d", n, x);
    $display("ac y=%0d z=%0d", y, z);
    $display("in2 a=%0d b=%0d", a, b);
    $display("ad c=%0d d=%0d", c, d);
    $display("blk f1=%0d f2=%0d f3=%0d", f1, f2, f3);
    $display("gb g1=%0d g2=%0d", g1, g2);
    $display("pe h1=%0d h2=%0d", h1, h2);
    $finish;
  end
endmodule
"#,
    );
    assert_eq!(
        got,
        [
            "m1=top.mb",
            "m2=top.mb.mi",
            "m3=top.fm",
            "ab n=40 x=20",
            "ac y=19 z=18",
            "in2 a=20 b=18",
            "ad c=4 d=1",
            "blk f1=0 f2=1 f3=0",
            "gb g1=1 g2=0",
            "pe h1=10 h2=5",
        ]
    );
}
