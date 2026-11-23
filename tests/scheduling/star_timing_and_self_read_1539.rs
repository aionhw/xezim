//! IEEE 1800-2023 §9.4.2.2: `@*` / `@(*)` infers sensitivity through timing,
//! wait, loop, case-guard, and procedural-continuous statements, excludes
//! delay/event/wait control expressions and block-local variables, and retains
//! self-written module-scope reads when the body suspends or has no non-written
//! external reads (`always @(*) #10 clk <= ~clk;` and self-referential `@*`).

use xezim::simulate;

fn lines_of(src: &str, max_time: u64) -> Vec<String> {
    simulate(src, max_time)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .collect()
}

#[test]
fn star_clock_divider_nba_and_blocking_toggle_correctly() {
    let src = r#"
`timescale 1ns/1ns
module t;
  reg clk1, clk2;
  integer n1 = 0, n2 = 0;
  initial begin #5 clk1 = 1'b0; clk2 = 1'b0; end
  always @(*) #10 clk1 <= ~clk1;
  always @(*) #10 clk2 = ~clk2;
  always @(posedge clk1) n1++;
  always @(posedge clk2) n2++;
  initial begin
    #2000;
    $display("CLKS n1=%0d n2=%0d", n1, n2);
    $finish;
  end
endmodule
"#;
    assert_eq!(lines_of(src, 3000), ["CLKS n1=100 n2=1"]);
}

#[test]
fn star_descends_through_controls_without_adding_control_expressions() {
    let src = r#"
`timescale 1ns/1ns
module t;
  reg [7:0] da = 0, ev_a = 8'd10, wa = 8'd10, d = 1, rn = 0;
  reg en = 0, trig = 0;
  reg [7:0] y_del = 0, w_wait = 0, ev_y = 0;
  reg ry = 0;
  int d_cnt = 0, w_cnt = 0, ev_cnt = 0, r_cnt = 0;

  always @* #(d) begin
    y_del = da;
    d_cnt = d_cnt + 1;
  end
  always @* wait (en) begin
    w_wait = wa;
    w_cnt = w_cnt + 1;
  end
  always @* @(posedge trig) begin
    ev_y = ev_a;
    ev_cnt = ev_cnt + 1;
  end
  always @* repeat (rn) #1 ry = ~ry;
  always @* repeat (rn) r_cnt = r_cnt + 1;

  initial begin
    #2; d = 2; #10;
    $display("DEL d_only=%0d", d_cnt);
    #2; da = 8'd7; #10;
    $display("DEL da=%0d y_del=%0d", d_cnt, y_del);

    #2; trig = 1; #2;
    $display("EV trig_only=%0d ev_y=%0d", ev_cnt, ev_y);
    trig = 0; #1; ev_a = 8'd42; #1; trig = 1; #2;
    $display("EV a=%0d ev_y=%0d", ev_cnt, ev_y);

    #2; en = 1; #2;
    $display("WAIT en_only=%0d w_wait=%0d", w_cnt, w_wait);
    en = 0; #1; wa = 8'd9; #1; en = 1; #2;
    $display("WAIT a=%0d w_wait=%0d", w_cnt, w_wait);

    #2; rn = 8'd2; #5;
    $display("REP rn=%0d ry=%b", r_cnt, ry);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        lines_of(src, 500),
        [
            "DEL d_only=0",
            "DEL da=1 y_del=7",
            "EV trig_only=0 ev_y=0",
            "EV a=1 ev_y=42",
            "WAIT en_only=0 w_wait=0",
            "WAIT a=1 w_wait=9",
            "REP rn=2 ry=0",
        ]
    );
}

#[test]
fn star_keeps_self_read_in_suspending_and_self_ref_blocks_and_ignores_empty() {
    let src = r#"
`timescale 1ns/1ns
module t;
  int x_blk = 0, cnt_blk = 0;
  int x_pre = 0, cnt_pre = 0;
  int sref = 0;
  int empty_hits = 0;
  logic csel = 1'b0, cclk = 1'b0;
  int ccnt = 0;

  always @* begin #5; x_blk = x_blk + 1; cnt_blk = cnt_blk + 1; end
  always @* begin x_pre = x_pre + 1; #5; cnt_pre = cnt_pre + 1; end
  always @* sref = sref + 1;
  always @* #5 empty_hits = empty_hits + 1;
  always @* if (csel) begin #5 cclk <= ~cclk; ccnt = ccnt + 1; end

  initial begin
    #1; x_blk = 16; x_pre = 16; sref = 10;
    #20;
    $display("BLK x_blk=%0d cnt_blk=%0d x_pre=%0d cnt_pre=%0d",
             x_blk, cnt_blk, x_pre, cnt_pre);
    $display("SREF sref=%0d EMPTY=%0d", sref, empty_hits);
    csel = 1'b1;
    #25;
    csel = 1'b0;
    #20;
    $display("COND ccnt=%0d cclk=%b", ccnt, cclk);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        lines_of(src, 500),
        [
            "BLK x_blk=17 cnt_blk=1 x_pre=17 cnt_pre=1",
            "SREF sref=11 EMPTY=0",
            "COND ccnt=5 cclk=1",
        ]
    );
}
