//! §9.4.2, §7.2.1: an event control on a member of an element of an
//! unpacked array of packed structs (`@(sa[1].a)`) waits on that member's
//! value. Each element is one signal and the member some of its bits, so the
//! term watches the element its index selects and fires only when the
//! member's value changes; an edge is judged on the member's least
//! significant bit. The member access gave the event control no operand to
//! watch: a procedural `@(sa[1].a)` never woke, an `always @(sa[1].a)` block
//! missed changes, and one also read by a continuous assignment woke on every
//! change of the element.
//!
//! Every count below skips time 0, where the variables are first written.
//! The expected values are Verilator 5.046's.

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 1_000_000)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect()
}

/// The issue's repro: the element member wakes like a scalar struct's
/// member, a whole element and a vector element.
#[test]
fn procedural_wait_on_an_element_member() {
    let src = r#"
module tb;
  typedef struct packed { logic [7:0] a; logic [7:0] b; } s_t;
  s_t sa[2];
  s_t s;
  logic [7:0] m[2];
  int n1, n2, n3, n4;
  initial begin #1; forever begin @(sa[1].a); n1++; end end
  initial begin #1; forever begin @(s.a); n2++; end end
  initial begin #1; forever begin @(sa[1]); n3++; end end
  initial begin #1; forever begin @(m[1]); n4++; end end
  initial begin
    #2 sa[1] = 16'h1122; s = 16'h1122; m[1] = 8'h11;
    #1 sa[1] = 16'h7788; s = 16'h7788; m[1] = 8'h77;
    #1 $display("n1=%0d n2=%0d n3=%0d n4=%0d", n1, n2, n3, n4);
    $finish;
  end
endmodule
"#;
    assert_eq!(lines(src), ["n1=2 n2=2 n3=2 n4=2"]);
}

/// Procedural and `always` controls wake once per change of the member, not
/// on a change of another member or element or on a write of the same value,
/// also when a continuous assignment reads the member.
#[test]
fn element_member_wakes_once_per_change() {
    let src = r#"
module tb;
  typedef struct packed { logic [7:0] a; logic [7:0] b; } s_t;
  s_t sa[2];
  int n_proc, n_alw, n_disp;
  wire [7:0] wa = sa[1].a;
  initial begin #1; forever begin @(sa[1].a); n_proc++; end end
  always @(sa[1].a) if ($time > 0) n_alw++;
  always @(sa[1].a) if ($time > 0) begin n_disp++; $display("%0t disp a=%h wa=%h", $time, sa[1].a, wa); end
  initial begin
    #2 sa[1] = 16'h1122;
    #1 sa[1].b = 8'h33;
    #1 sa[0] = 16'h4455;
    #1 sa[1] = 16'h7788;
    #1 sa[1].a = 8'h99;
    #1 sa[1].a = 8'h99;
    #1 $display("n_proc=%0d n_alw=%0d n_disp=%0d", n_proc, n_alw, n_disp);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "2 disp a=11 wa=11",
            "5 disp a=77 wa=77",
            "6 disp a=99 wa=99",
            "n_proc=3 n_alw=3 n_disp=3",
        ]
    );
}

/// Edges on a one-bit member, a nested member, an element of a
/// two-dimensional array, an element selected by a variable, and a bit of a
/// member.
#[test]
fn edges_nested_members_and_variable_elements() {
    let src = r#"
module tb;
  typedef struct packed { logic [1:0] hi; logic clk; logic valid; } bus_t;
  typedef struct packed { logic [3:0] x; bus_t in; } outer_t;
  bus_t ba[0:1];
  outer_t oa[2];
  bus_t m2[2][2];
  int i;
  int n_pos, n_neg, n_hi, n_nest, n_2d, n_var, n_bit, n_pos_proc;
  always @(posedge ba[1].clk) if ($time > 0) begin n_pos++; $display("%0t posedge ba[1].clk ba[1]=%b", $time, ba[1]); end
  always @(negedge ba[1].clk) if ($time > 0) n_neg++;
  always @(ba[1].hi) if ($time > 0) n_hi++;
  always @(oa[1].in.clk) if ($time > 0) n_nest++;
  always @(m2[1][0].hi) if ($time > 0) n_2d++;
  always @(ba[i].valid) if ($time > 0) n_var++;
  always @(ba[1].hi[1]) if ($time > 0) n_bit++;
  initial begin #1; forever begin @(posedge ba[1].clk); n_pos_proc++; end end
  initial begin
    i = 0;
    ba[0] = '0; ba[1] = '0; oa[0] = '0; oa[1] = '0; m2[0][0] = '0; m2[0][1] = '0; m2[1][0] = '0; m2[1][1] = '0;
    #2 ba[1].valid = 1;
    #1 ba[1].clk = 1;
    #1 ba[1].hi = 2'b10;
    #1 ba[1].clk = 0;
    #1 ba[1].hi = 2'b11;
    #1 ba[0].clk = 1;
    #1 oa[1].in.valid = 1;
    #1 oa[1].in.clk = 1;
    #1 oa[0].in.clk = 1;
    #1 m2[1][1].hi = 2'b01;
    #1 m2[1][0].hi = 2'b01;
    #1 m2[0][0].hi = 2'b10;
    #1 ba[0].valid = 1;
    #1 i = 1;
    #1 ba[1].valid = 0;
    #1 ba[0].valid = 0;
    #1 $display("n_pos=%0d n_neg=%0d n_hi=%0d n_nest=%0d n_2d=%0d n_var=%0d n_bit=%0d n_pos_proc=%0d", n_pos, n_neg, n_hi, n_nest, n_2d, n_var, n_bit, n_pos_proc);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "3 posedge ba[1].clk ba[1]=0011",
            "n_pos=1 n_neg=1 n_hi=2 n_nest=1 n_2d=1 n_var=2 n_bit=1 n_pos_proc=1",
        ]
    );
}

/// A task's automatic index (`@(sa[k].a)` in two forked calls), an `iff`
/// guard, and controls in a child module on an array with a descending
/// range. The task's loop is bounded: a wait that returns at once must not
/// spin the test forever.
#[test]
fn task_index_iff_and_child_module() {
    let src = r#"
module child;
  typedef struct packed { logic [3:0] hi; logic [3:0] lo; } p_t;
  p_t ca[3:1];
  int n_c, n_cp;
  always @(ca[2].lo) if ($time > 0) n_c++;
  initial begin #1; forever begin @(ca[3].hi); n_cp++; end end
  initial begin
    ca[1] = '0; ca[2] = '0; ca[3] = '0;
    #2 ca[2].hi = 4'h5;
    #1 ca[2].lo = 4'h6;
    #1 ca[3].lo = 4'h1;
    #1 ca[3].hi = 4'h2;
    #1 ca[2] = 8'h57;
  end
endmodule
module tb;
  typedef struct packed { logic [7:0] a; logic [7:0] b; } s_t;
  s_t sa[4];
  int n_task[4], t_task[4], n_iff;
  bit en;
  child u();
  task automatic watch(int k);
    repeat (3) begin @(sa[k].a); n_task[k]++; t_task[k] = $time; end
  endtask
  initial begin #1; fork watch(1); watch(2); join_none end
  always @(sa[3].b iff en) if ($time > 0) n_iff++;
  initial begin
    for (int k = 0; k < 4; k++) sa[k] = '0;
    en = 0;
    #2 sa[1].a = 8'h01;
    #1 sa[2].a = 8'h02;
    #1 sa[2].b = 8'h03;
    #1 sa[1] = 16'h0109;
    #1 sa[2] = 16'h0402;
    #1 sa[3].b = 8'h10;
    #1 en = 1;
    #1 sa[3].b = 8'h11;
    #1 sa[3].a = 8'h12;
    #1 $display("n_task=%0d@%0t,%0d@%0t n_iff=%0d n_c=%0d n_cp=%0d", n_task[1], t_task[1], n_task[2], t_task[2], n_iff, u.n_c, u.n_cp);
    $finish;
  end
endmodule
"#;
    assert_eq!(lines(src), ["n_task=1@2,2@6 n_iff=1 n_c=2 n_cp=1"]);
}

/// An element member inside a larger event expression: an operator, a
/// conditional, a concatenation and a function call. The expression's value
/// decides, so a change of `sa[1].b` or one that leaves the value alone does
/// not wake.
#[test]
fn compound_expressions_on_element_members() {
    let src = r#"
module tb;
  typedef struct packed { logic [7:0] a; logic [7:0] b; } s_t;
  s_t sa[2];
  logic [7:0] b;
  bit en;
  int n_xor, n_sum, n_cond, n_cat, n_fn;
  function automatic logic [7:0] f(logic [7:0] x); return x & 8'h0f; endfunction
  initial begin #1; forever begin @(sa[1].a ^ b); n_xor++; end end
  always @(sa[1].a + sa[0].b) if ($time > 0) n_sum++;
  always @(en ? sa[1].a : 8'h00) if ($time > 0) n_cond++;
  always @({sa[0].a, b}) if ($time > 0) n_cat++;
  always @(f(sa[1].a)) if ($time > 0) n_fn++;
  initial begin
    sa[0] = '0; sa[1] = '0; b = 8'h0f; en = 0;
    #2 sa[1].a = 8'h01;
    #1 sa[1].b = 8'h02;
    #1 sa[0].b = 8'h03;
    #1 en = 1;
    #1 sa[1].a = 8'h11;
    #1 sa[0].a = 8'h04;
    #1 b = 8'h0e;
    #1 sa[1].a = 8'h21;
    #1 $display("n_xor=%0d n_sum=%0d n_cond=%0d n_cat=%0d n_fn=%0d", n_xor, n_sum, n_cond, n_cat, n_fn);
    $finish;
  end
endmodule
"#;
    assert_eq!(lines(src), ["n_xor=4 n_sum=4 n_cond=3 n_cat=2 n_fn=1"]);
}
