//! IEEE 1800-2023 §9.4.5, §10.9.1, §11.6.1: an assignment pattern captured by
//! an intra-assignment delay or event control evaluates each item when the
//! statement runs, in the type of the element it is matched to, as the same
//! assignment without the timing control does. Each item was evaluated at
//! its own width, so `w8 = #1 '{a4 + b4, 1}` stored 0 for the 4-bit sum 16.

use xezim::simulate;

fn run(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn delayed_array_pattern_items_take_the_element_width() {
    let msgs = run(r##"
module top;
  logic [3:0] a4 = 4'd15, b4 = 4'd1;
  logic [7:0] w8 [2];
  logic [7:0] k8 [2];
  logic [7:0] f8 [2];
  logic [7:0] m8 [2][2];
  logic [7:0] d8 [1:0];
  logic [7:0] q8 [$];
  logic [7:0] y8 [];
  initial begin
    w8 = #1 '{a4 + b4, 1};
    k8 = #1 '{1: a4 + b4, default: 0};
    f8 = #1 '{default: a4 + b4};
    m8 = #1 '{'{a4 + b4, 1}, '{2, a4 + b4}};
    d8 = #1 '{a4 + b4, 3};
    q8 = #1 '{a4 + b4, 4};
    y8 = #1 '{5, a4 + b4};
    $display("w8=%0d,%0d k8=%0d,%0d f8=%0d,%0d",
             w8[0], w8[1], k8[0], k8[1], f8[0], f8[1]);
    $display("m8=%0d,%0d,%0d,%0d d8=%0d,%0d",
             m8[0][0], m8[0][1], m8[1][0], m8[1][1], d8[1], d8[0]);
    $display("q8=%0d,%0d y8=%0d,%0d", q8[0], q8[1], y8[0], y8[1]);
  end
endmodule
"##);
    assert_eq!(
        msgs,
        [
            "w8=16,1 k8=0,16 f8=16,16",
            "m8=16,1,2,16 d8=16,3",
            "q8=16,4 y8=5,16",
        ]
    );
}

#[test]
fn delayed_struct_and_row_pattern_items_take_the_member_width() {
    let msgs = run(r##"
module top;
  typedef struct { logic [7:0] x; logic [7:0] y; } s_t;
  logic [3:0] a4 = 4'd15, b4 = 4'd1;
  s_t s;
  s_t t;
  logic [7:0] m8 [2][2];
  initial begin
    s = #1 '{a4 + b4, 1};
    t = #1 '{y: a4 + b4, x: 2};
    m8[1] = #1 '{3, a4 + b4};
    $display("s=%0d,%0d t=%0d,%0d m8[1]=%0d,%0d", s.x, s.y, t.x, t.y, m8[1][0], m8[1][1]);
  end
endmodule
"##);
    assert_eq!(msgs, ["s=16,1 t=2,16 m8[1]=3,16"]);
}

#[test]
fn event_controlled_forked_and_nonblocking_pattern_items_take_the_element_width() {
    let msgs = run(r##"
module top;
  logic [3:0] a4 = 4'd15, b4 = 4'd1;
  logic [7:0] e8 [2];
  logic [7:0] g8 [2];
  logic [7:0] j8 [2];
  logic [7:0] n8 [2];
  event e;
  initial e8 = @(e) '{a4 + b4, 1};
  initial begin
    g8 <= @(e) '{a4 + b4, 2};
    fork j8 = #1 '{a4 + b4, 3}; join_none
    n8 <= #1 '{a4 + b4, 4};
    #1 -> e;
    #1 $display("e8=%0d,%0d g8=%0d,%0d j8=%0d,%0d n8=%0d,%0d",
                e8[0], e8[1], g8[0], g8[1], j8[0], j8[1], n8[0], n8[1]);
  end
endmodule
"##);
    assert_eq!(msgs, ["e8=16,1 g8=16,2 j8=16,3 n8=16,4"]);
}

#[test]
fn undelayed_and_narrow_target_patterns_are_unchanged() {
    let msgs = run(r##"
module top;
  logic [3:0] a4 = 4'd15, b4 = 4'd1;
  logic [7:0] v8 [2];
  logic [3:0] p4 [2];
  initial begin
    v8 = '{a4 + b4, 1};
    p4 = #1 '{a4 + b4, 4'd9};
    #1 $display("v8=%0d,%0d p4=%0d,%0d", v8[0], v8[1], p4[0], p4[1]);
  end
endmodule
"##);
    assert_eq!(msgs, ["v8=16,1 p4=0,9"]);
}

/// §10.9.2: a replicated structure pattern evaluates each member in that
/// member's assignment context, including when §9.4.5 captures it for later.
#[test]
fn replicated_struct_capture_keeps_each_member_width() {
    let msgs = run(r##"
module top;
  typedef struct { logic [3:0] lo; logic [7:0] hi; } s_t;
  logic [7:0] source = 8'hab;
  logic [3:0] a4 = 4'd15, b4 = 4'd1;
  s_t plain, delayed, nba, evented, event_nba, carry;
  event e;
  initial begin
    plain = '{2{source}};
    delayed = #1 '{2{source}};
    carry = #1 '{2{a4 + b4}};
    #1;
    $display("plain=%h,%h delayed=%h,%h nba=%h,%h",
             plain.lo, plain.hi, delayed.lo, delayed.hi, nba.lo, nba.hi);
    $display("event=%h,%h event_nba=%h,%h carry=%h,%h",
             evented.lo, evented.hi, event_nba.lo, event_nba.hi, carry.lo, carry.hi);
  end
  initial nba <= #1 '{2{source}};
  initial evented = @(e) '{2{source}};
  initial event_nba <= @(e) '{2{source}};
  initial begin
    #0 source = 8'h12;
    #1 -> e;
  end
endmodule
"##);
    assert_eq!(
        msgs,
        [
            "plain=b,ab delayed=b,ab nba=b,ab",
            "event=b,ab event_nba=b,ab carry=0,10",
        ]
    );
}

/// §10.9.2: converting a replicated item for a two-state member must not
/// discard the X/Z bits assigned to a subsequent four-state member.
#[test]
fn replicated_struct_capture_keeps_each_member_state_type() {
    let msgs = run(r##"
module top;
  typedef struct { bit [7:0] a; logic [7:0] b; } s_t;
  logic [7:0] source = 8'b1x0z11xz;
  s_t plain, delayed, nba, evented, event_nba;
  event e;
  initial begin
    plain = '{2{source}};
    delayed = #1 '{2{source}};
    #1;
    $display("plain=%b,%b delayed=%b,%b nba=%b,%b",
             plain.a, plain.b, delayed.a, delayed.b, nba.a, nba.b);
    $display("event=%b,%b event_nba=%b,%b",
             evented.a, evented.b, event_nba.a, event_nba.b);
  end
  initial nba <= #1 '{2{source}};
  initial evented = @(e) '{2{source}};
  initial event_nba <= @(e) '{2{source}};
  initial begin
    #0 source = 8'hff;
    #1 -> e;
  end
endmodule
"##);
    assert_eq!(
        msgs,
        [
            "plain=10001100,1x0z11xz delayed=10001100,1x0z11xz nba=10001100,1x0z11xz",
            "event=10001100,1x0z11xz event_nba=10001100,1x0z11xz",
        ]
    );
}
