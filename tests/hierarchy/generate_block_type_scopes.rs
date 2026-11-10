//! §6.18, §23.9: a typedef of a generate block is the block's. Two blocks
//! may each declare a type of one name, a reference names the type of the
//! nearest block that declares it, and the type reads the block's parameters.
//! Verilator 5.055 gives the expected value on each program.
use xezim::simulate;

/// The `T|` lines, sorted.
fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).unwrap_or_else(|e| panic!("rejected: {e}"));
    let mut lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    lines.sort();
    lines
}

const TWO_TYPEDEFS: &str = r#"
  if (1) begin : a
    localparam int W = 4;
    typedef logic [W-1:0] t;
    t v = '1;
    initial #1 $display("T|a=%0d", v);
  end
  if (1) begin : b
    localparam int W = 8;
    typedef logic [W-1:0] t;
    t v = '1;
    initial #1 $display("T|b=%0d", v);
  end
"#;

#[test]
fn two_blocks_typedefs_of_one_name_size_their_own_variables() {
    let src = format!("module top; {TWO_TYPEDEFS} initial #2 $finish; endmodule");
    assert_eq!(t_lines(&src), ["T|a=15", "T|b=255"]);
}

#[test]
fn two_blocks_typedefs_of_one_name_size_their_own_variables_in_an_instance() {
    let src = format!(
        "module m; {TWO_TYPEDEFS} endmodule module top; m u(); initial #2 $finish; endmodule"
    );
    assert_eq!(t_lines(&src), ["T|a=15", "T|b=255"]);
}

#[test]
fn two_blocks_typedefs_of_one_name_size_their_own_variables_in_unlabelled_blocks() {
    // §27.6: the blocks are genblk1 and genblk2.
    let src = TWO_TYPEDEFS.replace(" : a", "").replace(" : b", "");
    let src = format!("module top; {src} initial #2 $finish; endmodule");
    assert_eq!(t_lines(&src), ["T|a=15", "T|b=255"]);
}

#[test]
fn a_blocks_typedef_types_its_functions_formal_and_result() {
    // §13.4: 4'd15 + 1 wraps in the 4-bit result.
    let src = r#"
module top;
  if (1) begin : a
    typedef logic [3:0] t;
    function t f(t x); return x + 1; endfunction
    initial #1 $display("T|a=%0d %0d", f(4'd15), $bits(t));
  end
  if (1) begin : b
    typedef logic [7:0] t;
    function t f(t x); return x + 1; endfunction
    initial #1 $display("T|b=%0d %0d", f(8'd255), $bits(t));
  end
  initial #2 $finish;
endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=0 4", "T|b=0 8"]);
}

#[test]
fn a_cast_to_a_blocks_typedef_uses_its_width() {
    // §6.24.1.
    let src = r#"
module top;
  if (1) begin : a
    typedef logic [3:0] t;
    initial #1 $display("T|a=%0d", t'(8'hFF));
  end
  if (1) begin : b
    typedef logic [7:0] t;
    initial #1 $display("T|b=%0d", t'(16'hFFFF));
  end
  initial #2 $finish;
endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=15", "T|b=255"]);
}

#[test]
fn a_blocks_struct_typedef_sizes_its_members_by_the_blocks_parameter() {
    // §7.2.1.
    let src = r#"
module top;
  if (1) begin : a
    localparam int W = 4;
    typedef struct packed { logic [W-1:0] x; logic y; } s_t;
    s_t s;
    initial begin s.x = '1; s.y = 1; #1 $display("T|a=%0d %0d %0d", s.x, s.y, $bits(s_t)); end
  end
  if (1) begin : b
    localparam int W = 8;
    typedef struct packed { logic [W-1:0] x; logic y; } s_t;
    s_t s;
    initial begin s.x = '1; s.y = 0; #1 $display("T|b=%0d %0d %0d", s.x, s.y, $bits(s_t)); end
  end
  initial #2 $finish;
endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=15 1 5", "T|b=255 0 9"]);
}

#[test]
fn a_blocks_struct_typedef_reaches_a_child_port_in_two_instances() {
    // §23.3.3: the 16-bit port takes the zero-extended struct.
    let src = r#"
module child(input logic [15:0] d); initial #1 $display("T|%m d=%0d", d); endmodule
module m #(parameter int W = 4);
  if (1) begin : a
    typedef struct packed { logic [W-1:0] hi, lo; } s_t;
    s_t s = '{hi: 2, lo: 3};
    child c(.d(s));
    initial #1 $display("T|%m hi=%0d bits=%0d", s.hi, $bits(s_t));
  end
endmodule
module top; m #(4) u(); m #(8) v(); initial #2 $finish; endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|top.u.a hi=2 bits=8",
            "T|top.u.a.c d=35",
            "T|top.v.a hi=2 bits=16",
            "T|top.v.a.c d=515",
        ]
    );
}

#[test]
fn a_blocks_typedef_hides_the_modules_only_inside_the_block() {
    let src = r#"
module top;
  typedef logic [7:0] t;
  t m = '1;
  if (1) begin : a
    typedef logic [3:0] t;
    t v = '1;
    initial #1 $display("T|a=%0d", v);
  end
  if (1) begin : b
    t v = '1;
    initial #1 $display("T|b=%0d m=%0d", v, m);
  end
  initial #2 $finish;
endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=15", "T|b=255 m=255"]);
}

#[test]
fn a_nested_block_a_process_local_and_a_task_local_take_the_blocks_typedef() {
    let src = r#"
module top;
  if (1) begin : a
    localparam int W = 4;
    typedef logic [W-1:0] t;
    if (1) begin : n
      t v = '1;
      initial begin t w = '1; #1 $display("T|n=%0d %0d", v, w); end
    end
    task show; t q = '1; $display("T|task=%0d", q); endtask
    initial #1 show();
  end
  initial #2 $finish;
endmodule
"#;
    assert_eq!(t_lines(src), ["T|n=15 15", "T|task=15"]);
}

#[test]
fn a_process_local_typedef_hides_the_blocks() {
    // §6.18, §23.9: the local type reads the block's parameter.
    let src = r#"
module top;
  if (1) begin : a
    localparam int W = 4;
    typedef logic [7:0] t;
    initial begin
      typedef logic [W-1:0] t;
      t v = '1;
      #1 $display("T|a=%0d", v);
    end
  end
  initial #2 $finish;
endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=15"]);
}

#[test]
fn a_function_local_typedef_hides_the_blocks() {
    let src = r#"
module top;
  if (1) begin : a
    typedef logic [7:0] t;
    function int f(); typedef logic [3:0] t; t v = '1; return v; endfunction
    initial #1 $display("T|a=%0d", f());
  end
  initial #2 $finish;
endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=15"]);
}

#[test]
fn a_loop_variable_of_a_blocks_typedef_takes_the_type() {
    // §12.7.1: a signed loop variable counts 3, 2, 1, 0.
    let src = r#"
module top;
  if (1) begin : a
    typedef int sint_t;
    int n;
    initial begin
      for (sint_t i = 3; i >= 0; i--) begin n++; if (n > 8) break; end
      $display("T|a=%0d", n);
    end
  end
  initial #2 $finish;
endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=4"]);
}

#[test]
fn a_constraint_casts_to_a_blocks_enum() {
    // §18.5, §6.24.1.
    let src = r#"
module top;
  if (1) begin : a
    typedef enum logic [1:0] {OFF = 2'd0, ON = 2'd1} mode_t;
    class C; rand logic [1:0] x; constraint k { x == mode_t'(1); } endclass
    initial begin C c = new; int ok; ok = c.randomize(); $display("T|a=%0d ok=%0d", c.x, ok); end
  end
  initial #2 $finish;
endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=1 ok=1"]);
}

#[test]
fn an_enum_typedef_of_a_block_names_its_members() {
    // §6.19.5.
    let src = r#"
module top;
  if (1) begin : a
    typedef enum logic [1:0] {IDLE, RUN, DONE} st_t;
    st_t s = IDLE;
    always #1 if (s != DONE) s = s.next();
    initial #5 $display("T|a=%s %0d %0d", s.name(), s, s.num());
  end
  initial #6 $finish;
endmodule
"#;
    assert_eq!(t_lines(src), ["T|a=DONE 2 3"]);
}
