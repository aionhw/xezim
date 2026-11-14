//! §23.6, §7.2.1, §7.4.1: members of struct variables of a module instance.
//! A `$root`-qualified name (`$root.top.u.s.m`) reads and writes like the
//! absolute `top.u.s.m`, from another instance, a task and a function; the
//! absolute form takes element selects (`top.u.s.lanes[2]`). In an instance —
//! which every top of a multi-top design is — a packed array of packed
//! structs inside a packed struct (`s.pairs[1].pair[0]`) and a packed-array
//! member of an unpacked struct (`us.lanes[0]`) select whole elements in
//! their declared order, as at top level (#1046). Expected values are the
//! reference simulator's.

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
fn root_qualified_struct_members_read_and_write() {
    assert_eq!(
        lines(
            r#"typedef logic [7:0] lane_t;
typedef struct packed { lane_t [0:3] lanes; lane_t tag; } frame_t;
module leaf;
  frame_t h_frame;
  int     n;
  logic [15:0] w;
endmodule
module probe_a;
  frame_t h_frame;
  initial begin
    h_frame.lanes = 32'h468ACE13;
    h_frame.tag = 8'h13;
    #1;
    r1.got_whole = $root.r1.ua.h_frame.lanes;
    r1.got_tag   = $root.r1.ua.h_frame.tag;
    r1.got_el    = $root.r1.ua.h_frame.lanes[1];
    r1.got_all   = $root.r1.ua.h_frame;
  end
endmodule
module r1;
  logic [31:0] got_whole; logic [7:0] got_tag, got_el; logic [39:0] got_all;
  logic [7:0] t_tag; logic [31:0] t_whole;
  probe_a ua();
  leaf ul();
  task automatic rd(output logic [7:0] tg, output logic [31:0] wh);
    tg = $root.r1.ul.h_frame.tag;
    wh = $root.r1.ul.h_frame.lanes;
  endtask
  function automatic logic [7:0] frd(); return $root.r1.ul.h_frame.lanes[2]; endfunction
  initial begin
    $root.r1.ul.h_frame.lanes = 32'hA1B2C3D4;
    $root.r1.ul.h_frame.tag = 8'h5E;
    $root.r1.ul.n = 9;
    $root.r1.ul.w[7:0] = 8'hCD;
    #2;
    $display("A %h %h %h %h", got_whole, got_tag, got_el, got_all);
    $display("B %h %h %h %0d %h", ul.h_frame.lanes, ul.h_frame.tag, $root.r1.ul.h_frame, $root.r1.ul.n, $root.r1.ul.w);
    rd(t_tag, t_whole);
    $display("C %h %h %h", t_tag, t_whole, frd());
    $display("D %h %h", $root.r1.ul.h_frame.lanes[0], r1.ul.h_frame.tag);
  end
endmodule
"#
        ),
        [
            r#"A 468ace13 13 8a 468ace1313"#,
            r#"B a1b2c3d4 5e a1b2c3d45e 9 xxcd"#,
            r#"C 5e a1b2c3d4 c3"#,
            r#"D a1 5e"#,
        ]
    );
}

/// The same reads with a second top module (the multi-top wrapper).
#[test]
fn root_qualified_struct_members_with_two_tops() {
    assert_eq!(
        lines(
            r#"typedef logic [7:0] lane_t;
typedef struct packed { lane_t [0:3] lanes; lane_t tag; } frame_t;
module leaf;
  frame_t h_frame;
  initial begin
    h_frame.lanes = 32'h468ACE13;
    h_frame.tag = 8'h13;
  end
endmodule
module r2;
  leaf ul();
  logic [31:0] a; logic [7:0] b, c; logic [39:0] w;
  initial begin
    #1;
    a = $root.r2.ul.h_frame.lanes;
    b = $root.r2.ul.h_frame.tag;
    c = $root.r2.ul.h_frame.lanes[1];
    w = $root.r2.ul.h_frame;
    $display("A %h %h %h %h", a, b, c, w);
    $display("B %h %h %h", r2.ul.h_frame.lanes, r2.ul.h_frame.tag, r2.ul.h_frame.lanes[2]);
    $display("C %h %h %h", ul.h_frame.lanes, ul.h_frame.tag, ul.h_frame.lanes[3]);
  end
endmodule
module other_top;
endmodule
"#
        ),
        [
            r#"A 468ace13 13 8a 468ace1313"#,
            r#"B 468ace13 13 ce"#,
            r#"C 468ace13 13 13"#,
        ]
    );
}

#[test]
fn struct_member_element_selects_in_an_instance() {
    assert_eq!(
        lines(
            r#"package tp;
  typedef logic [7:0] lane_t;
  typedef struct packed { lane_t [0:1] pair; } pair_t;
  typedef struct packed { pair_t [0:1] pairs; lane_t tag; } nest_t;
  typedef struct { lane_t [0:3] lanes; lane_t tag; } ustruct_t;
  typedef struct packed { lane_t [0:3] lanes; lane_t tag; } frame_t;
endpackage
import tp::*;
module m1c;
  nest_t g_nest;
  ustruct_t g_ustruct;
  frame_t g_frame;
  logic [7:0] a, b;
  initial begin
    g_nest.pairs = 32'h468ACE13;
    g_ustruct.lanes = 32'h468ACE13;
    g_frame.lanes = 32'h468ACE13;
    #1;
    $display("R1 %h %h %h %h", g_nest.pairs[0].pair[0], g_nest.pairs[0].pair[1], g_nest.pairs[1].pair[0], g_nest.pairs[1].pair[1]);
    $display("R2 %h %h %h", g_ustruct.lanes[0], g_ustruct.lanes[3], g_frame.lanes[1]);
    a = g_nest.pairs[1].pair[0]; b = g_ustruct.lanes[2];
    $display("R3 %h %h", a, b);
    g_nest.pairs[1].pair[0] = 8'h55;
    g_ustruct.lanes[1] = 8'h66;
    $display("W1 %h %h", g_nest.pairs, g_ustruct.lanes);
  end
endmodule
module m2;
  m1c u();
endmodule
"#
        ),
        [
            r#"R1 46 8a ce 13"#,
            r#"R2 46 13 8a"#,
            r#"R3 ce ce"#,
            r#"W1 468a5513 4666ce13"#,
        ]
    );
}

#[test]
fn struct_member_element_selects_with_two_tops() {
    assert_eq!(
        lines(
            r#"package tp;
  typedef logic [7:0] lane_t;
  typedef struct packed { lane_t [0:1] pair; } pair_t;
  typedef struct packed { pair_t [0:1] pairs; lane_t tag; } nest_t;
  typedef struct { lane_t [0:3] lanes; lane_t tag; } ustruct_t;
  typedef struct packed { lane_t [0:3] lanes; lane_t tag; } frame_t;
endpackage
import tp::*;
module m1;
  nest_t g_nest;
  ustruct_t g_ustruct;
  frame_t g_frame;
  logic [7:0] a, b;
  initial begin
    g_nest.pairs = 32'h468ACE13;
    g_ustruct.lanes = 32'h468ACE13;
    g_frame.lanes = 32'h468ACE13;
    #1;
    $display("R1 %h %h %h %h", g_nest.pairs[0].pair[0], g_nest.pairs[0].pair[1], g_nest.pairs[1].pair[0], g_nest.pairs[1].pair[1]);
    $display("R2 %h %h %h", g_ustruct.lanes[0], g_ustruct.lanes[3], g_frame.lanes[1]);
    a = g_nest.pairs[1].pair[0]; b = g_ustruct.lanes[2];
    $display("R3 %h %h", a, b);
    g_nest.pairs[1].pair[0] = 8'h55;
    g_ustruct.lanes[1] = 8'h66;
    $display("W1 %h %h", g_nest.pairs, g_ustruct.lanes);
  end
endmodule
module other_top;
  initial #2 $display("other");
endmodule
"#
        ),
        [
            r#"R1 46 8a ce 13"#,
            r#"R2 46 13 8a"#,
            r#"R3 ce ce"#,
            r#"W1 468a5513 4666ce13"#,
            r#"other"#,
        ]
    );
}
