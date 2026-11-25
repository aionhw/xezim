//! IEEE 1800-2023 §10.3.3 applies the continuous assignment delay to the
//! whole right-hand value. Under §10.9.2, each structure pattern item takes
//! its member's type, including an unpacked array member.

use xezim::simulate;

fn run(src: &str) -> Vec<String> {
    simulate(src, 100_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|entry| entry.message.clone())
        .collect()
}

#[test]
fn delayed_pattern_updates_the_array_member() {
    let lines = run(r#"
`timescale 1ns/1ps
module top;
  typedef struct { int tag; int a[1]; } S;
  S s;
  int x;
  assign #3 s = '{tag:x, a:'{default:x}};
  initial begin
    x = 7;
    #2 $display("before tag=%0d a=%0d", s.tag, s.a[0]);
    #2 $display("after tag=%0d a=%0d", s.tag, s.a[0]);
    $finish;
  end
endmodule
"#);
    assert_eq!(lines, ["before tag=0 a=0", "after tag=7 a=7"]);
}

#[test]
fn delayed_copy_updates_the_array_member() {
    let lines = run(r#"
`timescale 1ns/1ps
module top;
  typedef struct { int tag; int a[1]; } S;
  S s, s2;
  assign #3 s = s2;
  initial begin
    s2.tag = 7;
    s2.a[0] = 7;
    #2 $display("before tag=%0d a=%0d", s.tag, s.a[0]);
    #2 $display("after tag=%0d a=%0d", s.tag, s.a[0]);
    $finish;
  end
endmodule
"#);
    assert_eq!(lines, ["before tag=0 a=0", "after tag=7 a=7"]);
}

#[test]
fn delayed_assignment_updates_the_selected_array_element() {
    let lines = run(r#"
`timescale 1ns/1ps
module top;
  typedef struct { int tag; int a[1]; } S;
  S s;
  int x;
  assign #3 s.a[0] = x;
  initial begin
    x = 7;
    #2 $display("before a=%0d", s.a[0]);
    #2 $display("after a=%0d", s.a[0]);
    $finish;
  end
endmodule
"#);
    assert_eq!(lines, ["before a=0", "after a=7"]);
}

#[test]
fn undelayed_pattern_updates_both_members() {
    let lines = run(r#"
`timescale 1ns/1ps
module top;
  typedef struct { int tag; int a[1]; } S;
  S s;
  int x;
  assign s = '{tag:x, a:'{default:x}};
  initial begin
    x = 7;
    #1 $display("tag=%0d a=%0d", s.tag, s.a[0]);
    $finish;
  end
endmodule
"#);
    assert_eq!(lines, ["tag=7 a=7"]);
}

#[test]
fn scalar_member_updates_after_the_continuous_delay() {
    let lines = run(r#"
`timescale 1ns/1ps
module top;
  typedef struct { int tag; } S;
  S s;
  int x;
  assign #3 s = '{tag:x};
  initial begin
    x = 7;
    #2 $display("before tag=%0d", s.tag);
    #2 $display("after tag=%0d", s.tag);
    $finish;
  end
endmodule
"#);
    assert_eq!(lines, ["before tag=0", "after tag=7"]);
}

/// §9.4.5 samples the right-hand side before the procedural delay.
#[test]
fn procedural_delay_stores_the_captured_struct_pattern() {
    let lines = run(r#"
`timescale 1ns/1ps
module top;
  typedef struct { int tag; int a[1]; } S;
  S s;
  int x;
  initial begin
    x = 7;
    s = #3 '{tag:x, a:'{default:x}};
  end
  initial #1 x = 9;
  initial begin
    #2 $display("before tag=%0d a=%0d", s.tag, s.a[0]);
    #2 $display("after tag=%0d a=%0d", s.tag, s.a[0]);
    $finish;
  end
endmodule
"#);
    assert_eq!(lines, ["before tag=0 a=0", "after tag=7 a=7"]);
}

/// §10.3.3: a change to the whole value restarts its inertial delay.
#[test]
fn changing_one_member_reschedules_the_pending_structure() {
    let lines = run(r#"
module top;
  typedef struct {int tag; int a[1];} S;
  S s;
  int x, y;
  assign #3 s = '{tag:x, a:'{y}};
  initial begin
    x=5; y=7; #2; x=6;
    #2 $display("pending %0d %0d", s.tag, s.a[0]);
    #2 $display("arrived %0d %0d", s.tag, s.a[0]);
  end
endmodule"#);
    assert_eq!(lines, ["pending 0 0", "arrived 6 7"]);
}

/// §7.6 and §9.4.5: sample every field of a class-property array before
/// delaying the procedural assignment.
#[test]
fn procedural_delay_captures_class_struct_array_fields() {
    let lines = run(r#"
module top;
  typedef struct { int a; byte b; } S;
  class C;
    S rows[2];
    S dst[2];
    task copy();
      rows[0] = '{1, 2};
      rows[1] = '{3, 4};
      fork
        dst = #2 rows;
        #1 begin
          rows[0].a = 9;
          rows[1].b = 8;
          $display("pending %0d %0d", dst[0].a, dst[1].b);
        end
      join
      $display("saved %0d %0d %0d %0d", dst[0].a, dst[0].b, dst[1].a, dst[1].b);
    endtask
  endclass
  C c;
  initial begin
    c = new;
    c.copy();
  end
endmodule
"#);
    assert_eq!(lines, ["pending 0 0", "saved 1 2 3 4"]);
}

/// §7.6 and §9.4.5: sample an object array through a receiver call once,
/// before delaying the store.
#[test]
fn procedural_delay_captures_a_function_returned_receiver() {
    let lines = run(r#"
module top;
typedef struct { int a; byte b; } S;
class C;
  S src[2];
endclass
C c;
int calls;
S dst[2];
function C pick();
  calls++;
  return c;
endfunction
initial begin
  c=new; c.src[0]='{11,12}; c.src[1]='{13,14};
  fork
    dst = #3 pick().src;
    #1 c.src[0].a=99;
  join
  $display("receiver %0d %0d %0d", calls,dst[0].a,dst[1].b);
end
endmodule
"#);
    assert_eq!(lines, ["receiver 1 11 14"]);
}
