//! §7.2.1, §7.4.1, §11.5.1, §12.7.3 (#1046): element and part-selects of a
//! packed-array member of a packed struct address that member's ELEMENTS in
//! their declared index order, through every receiver: an element of an
//! unpacked-array class property (`arr[i].lanes[1]`), an ascending
//! packed-array member of a packed-array struct element
//! (`g_nest.pairs[0].pair[0]`), a constant part-select (`lanes[1:2]`, read
//! and written), a `super.` write and a `foreach` over the member. Expected
//! values are from the reference simulator.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 1_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect()
}

fn expect(msgs: &[String], want: &[&str]) {
    for w in want {
        assert!(msgs.iter().any(|m| m == w), "missing {w:?} in {msgs:#?}");
    }
}

#[test]
fn packed_array_member_selects_through_every_receiver() {
    let msgs = lines(
        r#"
typedef logic [7:0] lane_t;
typedef struct packed { lane_t [0:3] lanes; lane_t tag; } frame_t;
typedef struct packed { lane_t [0:1] pair; } pair_t;
typedef struct packed { pair_t [0:1] pairs; lane_t tag; } nest_t;
class Holder;
  frame_t frame;
  frame_t arr[2];
  function logic [7:0] rd_arr0(); return arr[0].lanes[0]; endfunction
  function logic [7:0] rd_arr(int i); return arr[i].lanes[1]; endfunction
  function void w_foreach(logic [7:0] v);
    foreach (frame.lanes[k]) if (k == 2) frame.lanes[k] = v;
  endfunction
endclass
class Derived extends Holder;
  function void w_super(logic [7:0] v); super.frame.lanes[0] = v; endfunction
endclass
module top;
  Holder h; Derived d;
  nest_t g_nest;
  frame_t g_frame;
  frame_t g_arr [0:1];
  logic [15:0] w_cont2;
  assign w_cont2 = g_frame.lanes[1:2];
  initial begin
    h = new; d = new;
    h.frame.lanes = 32'h468ACE13;
    h.arr[0].lanes = 32'h468ACE13; h.arr[1].lanes = 32'h11223344;
    g_nest.pairs = 32'h468ACE13;
    g_frame.lanes = 32'h468ACE13;
    g_arr[1].lanes = 32'h55667788;
    #1;
    $display("R2 %h %h %h", h.rd_arr0(), h.rd_arr(0), h.rd_arr(1));
    $display("R3 %h %h %h %h", g_nest.pairs[0].pair[0], g_nest.pairs[0].pair[1],
             g_nest.pairs[1].pair[0], g_nest.pairs[1].pair[1]);
    $display("R6 %h %h %h", h.frame.lanes[1:2], h.frame.lanes[0:1], g_frame.lanes[2:3]);
    $display("R7 %h %h", g_arr[1].lanes[2], w_cont2);
    $display("R8 %h %h", h.frame.lanes[1][3:0], h.frame.lanes[2][7]);
    d.frame.lanes = 32'h468ACE13;
    d.w_super(8'hAA);
    h.w_foreach(8'hCC);
    h.frame.lanes[3] <= 8'hDD;
    g_nest.pairs[1].pair[0] = 8'h55;
    #1;
    $display("W1 %h %h %h", d.frame.lanes, h.frame.lanes, g_nest.pairs);
    h.frame.lanes[1:2] = 16'hBEEF;
    $display("W2 %h", h.frame.lanes);
  end
endmodule
"#,
    );
    expect(
        &msgs,
        &[
            "R2 46 8a 22",
            "R3 46 8a ce 13",
            "R6 8ace 468a ce13",
            "R7 77 8ace",
            "R8 a 1",
            "W1 aa8ace13 468accdd 468a5513",
            "W2 46beefdd",
        ],
    );
}
