//! IEEE 1800-2023 §7.2.1, §11.5.1 / GH#1046: an INDEX select of a packed-array
//! member of a packed struct reached through a class property (`h.frame.
//! lanes[0]`) was mis-evaluated as a BIT select of the whole member, yielding
//! a 1-bit value instead of the full element width. Fixes the read path
//! (including the implicit-`this` receiver), the write path (including
//! TypeReference-typed members like `lane_t [0:3]`), and indexed part-selects
//! (`lanes[0 +: 1]`). Expected values are from the reference simulator.

use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_line(msgs: &[String], want: &str) {
    assert!(
        msgs.iter().any(|m| m.trim() == want),
        "missing {want:?} in {msgs:#?}"
    );
}

#[test]
fn packed_array_struct_member_index_select_read() {
    let msgs = messages(
        "module t;
  typedef logic [7:0] lane_t;
  typedef struct packed { lane_t [0:3] lanes; lane_t tag; } frame_t;
  class Holder; frame_t frame; endclass
  Holder h;
  initial begin
    h = new();
    h.frame.lanes = 32'h468ACE13;
    #1 $display(\"L0=%02h L1=%02h L3=%02h TAG=%02h\", h.frame.lanes[0], h.frame.lanes[1], h.frame.lanes[3], h.frame.tag);
  end
endmodule",
    );
    assert_line(&msgs, "L0=46 L1=8a L3=13 TAG=xx");
}

#[test]
fn packed_array_struct_member_index_select_write() {
    let msgs = messages(
        "module t;
  typedef logic [7:0] lane_t;
  typedef lane_t [0:3] lanes_t;
  typedef struct packed { lanes_t lanes; lane_t tag; } frame_t;
  class Holder; frame_t frame; endclass
  Holder h;
  initial begin
    h = new();
    h.frame.lanes = 32'h468ACE13;
    h.frame.lanes[0] = 8'hAA;
    #1 $display(\"W=%h\", h.frame.lanes);
  end
endmodule",
    );
    // Writing element 0 (MS byte) leaves bits [31:24]=AA.
    assert_line(&msgs, "W=aa8ace13");
}

#[test]
fn packed_array_struct_member_implicit_this_read() {
    let msgs = messages(
        "module t;
  typedef logic [7:0] lane_t;
  typedef struct packed { lane_t [0:3] lanes; lane_t tag; } frame_t;
  class Holder;
    frame_t frame;
    function logic [7:0] read0(); return frame.lanes[0]; endfunction
    function logic [7:0] read_this(); return this.frame.lanes[1]; endfunction
  endclass
  Holder h;
  initial begin
    h = new();
    h.frame.lanes = 32'h468ACE13;
    #1 $display(\"IM=%02h TH=%02h\", h.read0(), h.read_this());
  end
endmodule",
    );
    assert_line(&msgs, "IM=46 TH=8a");
}

#[test]
fn packed_array_struct_member_part_select() {
    let msgs = messages(
        "module t;
  typedef logic [7:0] lane_t;
  typedef struct packed { lane_t [0:3] lanes; lane_t tag; } frame_t;
  class Holder; frame_t frame; endclass
  Holder h;
  initial begin
    h = new();
    h.frame.lanes = 32'h468ACE13;
    #1 $display(\"PS0=%02h PS1=%02h\", h.frame.lanes[0 +: 1], h.frame.lanes[1 +: 1]);
  end
endmodule",
    );
    assert_line(&msgs, "PS0=46 PS1=8a");
}
