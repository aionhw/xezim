//! §6.21, §13.3.1, §13.3.2: a subroutine with no written lifetime in a static definition
//! is static. Its variables, formals included, retain their values between
//! invocations, also when a clocked process runs on the compiled tier.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// `clk` rises at 5, 15, ... 95: ten calls by time 100.
const CLK: &str = "logic clk = 0; always #5 clk = ~clk;";

#[test]
fn a_module_task_with_no_written_lifetime_keeps_its_static_local() {
    let src = format!(
        r#"module top; {CLK} int acc = 0;
  task tk(); static int w = 0; w++; acc += w; endtask
  always @(posedge clk) tk();
  initial begin #100 $display("%0d", acc); $finish; end
endmodule"#
    );
    assert_eq!(lines(&src), ["55"]);
}

#[test]
fn only_an_automatic_task_gets_fresh_locals() {
    let src = format!(
        r#"module top; {CLK} int a = 0, b = 0, c = 0;
  task automatic ta(); int w = 0; w++; a += w; endtask
  task static ts(); static int w = 0; w++; b += w; endtask
  task ti(); static int w = 0; w++; c += w; endtask
  always @(posedge clk) begin ta(); ts(); ti(); end
  initial begin #100 $display("%0d %0d %0d", a, b, c); $finish; end
endmodule"#
    );
    assert_eq!(lines(&src), ["10 55 55"]);
}

#[test]
fn a_top_level_block_task_keeps_its_static_local_bare_and_by_path() {
    let src = format!(
        r#"module top; {CLK}
  if (1) begin : g
    int bare = 0, path = 0;
    task tb(); static int w = 0; w++; bare += w; endtask
    task tp(); static int w = 0; w++; path += w; endtask
    always @(posedge clk) begin tb(); g.tp(); end
  end
  initial begin #100 $display("%0d %0d", g.bare, g.path); $finish; end
endmodule"#
    );
    assert_eq!(lines(&src), ["55 55"]);
}

#[test]
fn block_tasks_in_an_instance_keep_one_static_local_per_generate_iteration() {
    let src = format!(
        r#"module m; {CLK}
  for (genvar i = 0; i < 2; i++) begin : h
    int acc = 0;
    task t(); static int w = 0; w++; acc += w; endtask
    always @(posedge clk) if (i == 0 || $time < 50) t();
    if (1) begin : n
      int deep = 0;
      task d(); static int w = 0; w++; deep += w; endtask
      always @(posedge clk) d();
    end
  end
  initial #100 $display("%0d %0d %0d %0d", h[0].acc, h[1].acc, h[0].n.deep, h[1].n.deep);
endmodule
module top; m u(); initial #101 $finish; endmodule"#
    );
    assert_eq!(lines(&src), ["55 15 55 55"]);
}

#[test]
fn a_void_function_with_no_written_lifetime_keeps_its_static_local() {
    let src = format!(
        r#"module top; {CLK} int acc = 0;
  function void f(); static int w = 0; w++; acc += w; endfunction
  always @(posedge clk) f();
  initial begin #100 $display("%0d", acc); $finish; end
endmodule"#
    );
    assert_eq!(lines(&src), ["55"]);
}

#[test]
fn a_function_with_a_result_and_no_written_lifetime_keeps_its_static_local() {
    let src = format!(
        r#"module top; {CLK} int acc = 0;
  function int f(); static int w = 0; w++; return w; endfunction
  always @(posedge clk) acc += f();
  initial begin #100 $display("%0d", acc); $finish; end
endmodule"#
    );
    assert_eq!(lines(&src), ["55"]);
}

#[test]
fn a_local_without_a_qualifier_is_static_in_a_static_task() {
    let src = format!(
        r#"module top; {CLK} int a = 0, b = 0, c = 0;
  task top_level(); int w; w++; a = w; endtask
  task nested(); begin int w; w++; b += w; end endtask
  task looped(int n); for (int i = 0; i < n; i++) begin int w; w++; c += w; end endtask
  always @(posedge clk) begin top_level(); nested(); looped(1); end
  initial begin #100 $display("%0d %0d %0d", a, b, c); $finish; end
endmodule"#
    );
    assert_eq!(lines(&src), ["10 55 55"]);
}

#[test]
fn an_output_formal_of_a_static_task_retains_its_value_between_calls() {
    let src = format!(
        r#"module top; {CLK} int r = 0;
  task t(output int o); o = o + 1; endtask
  always @(posedge clk) t(r);
  initial begin #100 $display("%0d", r); $finish; end
endmodule"#
    );
    assert_eq!(lines(&src), ["10"]);
}

#[test]
fn output_formals_own_their_initial_value_and_lifetime() {
    let src = r#"module top;
  bit [3:0] a = 9, b = 8, rw = 3;
  logic [7:0] unknowns = '0;
  chandle handle;
  task automatic fresh(output bit [3:0] o, output logic [7:0] x, inout bit [3:0] io, output chandle h);
    $display("fresh %0d %b %0d %b", o, x, io, h == null);
    o = 4; x = '1; io++;
  endtask
  task remember(output bit [3:0] o);
    $display("static %0d", o);
    o++;
  endtask
  initial begin
    fresh(a, unknowns, rw, handle);
    fresh(b, unknowns, rw, handle);
    remember(a);
    remember(b);
    $display("copy %0d %0d %0d", a, b, rw);
    $finish;
  end
endmodule"#;
    assert_eq!(
        lines(src),
        [
            "fresh 0 xxxxxxxx 3 1",
            "fresh 0 xxxxxxxx 4 1",
            "static 0",
            "static 1",
            "copy 1 2 5"
        ]
    );
}

#[test]
fn class_task_output_defaults_follow_the_type_parameter() {
    let src = r#"module top;
  class C #(type T = bit);
    task read_output(input T expected, output T o, output T entry);
      #1;
      $display("entry %b", o === expected);
      entry = o;
    endtask
  endclass
  C #(bit) bits = new;
  C #(string) strings = new;
  C #(logic) four_state = new;
  bit b = 1, before_b = 1;
  string s = "caller", before_s = "caller";
  logic l = 1, before_l = 1;
  initial begin
    bits.read_output(1'b0, b, before_b);
    strings.read_output("", s, before_s);
    four_state.read_output(1'bx, l, before_l);
    $display("bit %b %b", b, before_b);
    $display("string %0d %0d", s.len(), before_s.len());
    $display("logic %b %b", l, before_l);
    $finish;
  end
endmodule"#;
    assert_eq!(
        lines(src),
        [
            "entry 1",
            "entry 1",
            "entry 1",
            "bit 0 0",
            "string 0 0",
            "logic x x"
        ]
    );
}

#[test]
fn a_static_task_output_nba_updates_the_formal_after_copyout() {
    let src = r#"module top;
  bit a = 1, b = 0;
  task t(output bit q);
    #1;
    q <= 1;
  endtask
  initial begin
    t(a);
    #1;
    t(b);
    $display("copy %b %b", a, b);
    $finish;
  end
endmodule"#;
    assert_eq!(lines(src), ["copy 0 1"]);
}

#[test]
fn a_static_local_is_readable_by_path_after_compiled_calls() {
    let src = format!(
        r#"module top; {CLK}
  task t(); static int w = 0; w++; endtask
  always @(posedge clk) t();
  initial begin #100 $display("%0d", t.w); $finish; end
endmodule"#
    );
    assert_eq!(lines(&src), ["10"]);
}

#[test]
fn a_task_with_no_locals_still_runs_in_a_clocked_process() {
    let src = format!(
        r#"module top; {CLK} int a = 0;
  task t(int x); a += x; endtask
  always @(posedge clk) t(2);
  initial begin #100 $display("%0d", a); $finish; end
endmodule"#
    );
    assert_eq!(lines(&src), ["20"]);
}

#[test]
fn subroutines_of_an_automatic_module_get_fresh_locals_also_in_a_block() {
    let src = format!(
        r#"module automatic top; {CLK} int a = 0, b = 0;
  task t(); int w; w++; a += w; endtask
  if (1) begin : g
    task n(); int w; w++; b += w; endtask
    always @(posedge clk) n();
  end
  always @(posedge clk) t();
  initial begin #100 $display("%0d %0d", a, b); $finish; end
endmodule"#
    );
    assert_eq!(lines(&src), ["10 10"]);
}
