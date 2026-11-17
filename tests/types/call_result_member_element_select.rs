//! #984 (G): §13.4.1, §7.4.1: an element select of a packed-array member of
//! a call result (`set_td(a, i, j, k).f0[1][1][1]`, `mk().f0[1]`) selects
//! elements by the member's declared dimensions, through its typedef too.
//! Every index was a bit select of the member (`5` read `x`), in `initial`,
//! `always` and `always_comb` code. Expected lines are the reference
//! simulator's.

use xezim::simulate;

/// §7.2.1, §6.16, §7.10.2.5: a projected member retains its declared
/// string, signed integer or real type, including a queue pop result.
#[test]
fn unpacked_call_and_queue_members_keep_their_types() {
    let src = r#"
module top;
  typedef struct { string label; int count; real weight; } row_t;
  row_t q[$];
  function automatic row_t make();
    return '{ "item", -7, 1.5 };
  endfunction
  initial begin
    $display("fn|%0d|%0d|%0.1f", make().label.len(), make().count < 0,
             make().weight + 0.5);
    q.push_back(make());
    $display("q|%0d|%0d", q.pop_front().label.len(), q.size());
    q.push_back(make());
    $display("n|%0d|%0d", q.pop_back().count < 0, q.size());
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    let out: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert_eq!(out, ["fn|4|1|2.0", "q|4|0", "n|1|0"]);
}

#[test]
fn call_result_member_selects_elements() {
    let src = r#"
typedef logic [2:0] t3_t;
typedef struct packed { t3_t [1:0][3:0][7:0] f0; } td_t;
typedef struct packed { logic [1:0][3:0][7:0][2:0] f0; logic [7:0] g; } ti_t;
function automatic td_t set_td(input td_t a, input logic [1:0] i, j, input logic [2:0] k);
  td_t c; c = a; c.f0[i][j][k] = 3'd5; return c;
endfunction
function automatic ti_t mk(); ti_t c; c = '0; c.f0[1][1][1] = 3'd5; c.f0[0][2] = 24'h81; c.g = 8'h33; return c; endfunction
module top;
  logic clk = 0;
  logic [2:0] p1, p2, c1; logic [23:0] p3; logic [95:0] p4; logic p5; logic [7:0] p6;
  always @(posedge clk) begin
    p1 = mk().f0[1][1][1]; p2 = set_td(td_t'(0), 1, 2, 3).f0[1][2][3];
    p3 = mk().f0[1][1]; p4 = mk().f0[1]; p5 = mk().f0[0][2][0]; p6 = mk().g;
  end
  always_comb c1 = set_td(td_t'(0), 0, 3, 7).f0[0][3][7];
  initial begin
    #1 clk = 1;
    #1 $display("proc %0d %0d %h %h %0d %h", p1, p2, p3, p4, p5, p6);
    $display("comb %0d", c1);
    $display("init %0d %0d %h %0d", mk().f0[1][1][1], set_td(td_t'(0), 1, 0, 2).f0[1][0][2], mk().f0[1][1], mk().f0[0][2][0]);
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_eq!(
        out,
        [
            "proc 5 5 000028 000000000000000028000000 1 33",
            "comb 5",
            "init 5 5 000028 1",
        ]
    );
}
