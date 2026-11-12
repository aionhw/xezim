//! §8.9, §7.2.1, §7.4.1: a packed-struct static class property is one shared
//! cell. A member write (`Cls::sp.m = v`, `sp.m = v` in a static function)
//! updates that cell, and an element select of a packed-array member
//! (`Cls::sp.lanes[0]`, `sp.lanes[1]` in a method) slices it — from module
//! scope, a static function, an instance method, through `this` and through a
//! handle, with or without any object of the class (#1046). Expected values
//! are the reference simulator's.

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
fn static_packed_struct_member_writes_and_element_selects() {
    assert_eq!(
        lines(
            r#"typedef logic [7:0] lane_t;
typedef struct packed { lane_t [0:3] lanes; lane_t tag; } frame_t;
class H;
  static frame_t s_frame;
  static int s_int;
  static logic [31:0] s_word;
  static function logic [7:0] rd_el(); return s_frame.lanes[0]; endfunction
  static function logic [31:0] rd_whole(); return s_frame.lanes; endfunction
  static function logic [39:0] rd_all(); return s_frame; endfunction
  static function logic [7:0] rd_tag(); return s_frame.tag; endfunction
  function logic [31:0] i_rd_whole(); return s_frame.lanes; endfunction
  function logic [7:0] i_rd_el(); return s_frame.lanes[1]; endfunction
  function logic [31:0] i_rd_this(); return this.s_frame.lanes; endfunction
  static function void wr_el(lane_t v); s_frame.lanes[2] = v; endfunction
  static function void wr_whole(logic [31:0] v); s_frame.lanes = v; endfunction
  static function int rd_int(); return s_int; endfunction
  static function logic [7:0] rd_word(); return s_word[15:8]; endfunction
endclass
module s1;
  H h;
  initial begin
    h = new;
    H::s_frame.lanes = 32'h468ACE13;
    H::s_frame.tag = 8'h77;
    H::s_int = 5;
    H::s_word = 32'h11223344;
    $display("A %h %h %h", H::s_frame.lanes[0], H::s_frame.lanes, H::s_frame.tag);
    $display("B %h %h %h %h", H::rd_el(), H::rd_whole(), H::rd_all(), H::rd_tag());
    $display("C %h %h %h", h.i_rd_whole(), h.i_rd_el(), h.i_rd_this());
    $display("D %0d %h %h %h", H::rd_int(), H::rd_word(), h.s_frame.lanes, h.s_frame.lanes[3]);
    H::wr_el(8'hEE);
    $display("E %h %h", H::s_frame.lanes, H::rd_whole());
    H::wr_whole(32'hDEADBEEF);
    $display("F %h %h %h", H::s_frame.lanes, H::rd_whole(), H::s_frame.lanes[1]);
    H::s_frame = 40'h0102030405;
    $display("G %h %h %h", H::s_frame, H::rd_all(), H::rd_el());
  end
endmodule
"#
        ),
        [
            r#"A 46 468ace13 77"#,
            r#"B 46 468ace13 468ace1377 77"#,
            r#"C 468ace13 8a 468ace13"#,
            r#"D 5 33 468ace13 13"#,
            r#"E 468aee13 468aee13"#,
            r#"F deadbeef deadbeef ad"#,
            r#"G 0102030405 0102030405 01"#,
        ]
    );
}

/// No object of the class exists: the static is reached through its class.
#[test]
fn static_packed_struct_members_without_objects() {
    assert_eq!(
        lines(
            r#"typedef struct packed { logic [7:0] a; logic [7:0] b; } st_t;
class H;
  static st_t s;
endclass
module s2;
  initial begin
    H::s.a = 8'h12;
    $display("A %h %h", H::s.a, H::s);
    H::s = 16'h3456;
    $display("B %h %h", H::s.a, H::s);
    H::s.b = 8'h78;
    $display("C %h %h", H::s.b, H::s);
  end
endmodule
"#
        ),
        [r#"A 12 12xx"#, r#"B 34 3456"#, r#"C 78 3478"#]
    );
}
