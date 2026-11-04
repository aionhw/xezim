//! IEEE 1800-2023 §3.12.1 / §8.24: unit visibility and out-of-block methods.

#[test]
fn unit_subroutines_keep_lexical_bindings_and_class_helpers() {
    let sim = xezim::simulate(
        r#"
int shared=3;
function automatic int helper(); return shared; endfunction
function automatic int outer(); return helper()+1; endfunction
function automatic int add(input int x); return helper()+x; endfunction
class C;
  extern function int get();
endclass
function int C::get(); return helper(); endfunction
module child;
  int shared=99;
  C c;
  initial begin
    c=new;
    $display("T|child|%0d %0d %0d %0d",outer(),c.get(),shared,add(shared));
  end
endmodule
module class_only;
  int shared=101;
  C c;
  initial begin c=new; $display("T|class_only|%0d %0d",c.get(),shared); end
endmodule
module unused; int value; endmodule
module top;
  child u0(), u1();
  unused u2();
  class_only u3();
  initial #1 $display("T|top|%0d",outer());
endmodule
"#,
        100,
    )
    .expect("simulate");
    let mut lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|s| s.starts_with("T|"))
        .collect();
    lines.sort_unstable();
    assert_eq!(
        lines,
        [
            "T|child|4 3 99 102",
            "T|child|4 3 99 102",
            "T|class_only|3 101",
            "T|top|4"
        ]
    );
}

#[test]
fn root_task_helpers_keep_scope_across_suspension_and_copyback() {
    let sim = xezim::simulate(
        r#"
int shared=3;
task automatic read_shared(output int value, input int extra=0); value=shared+extra; endtask
task automatic later(output int value, input int extra); #1 read_shared(value,extra); endtask
class C;
  extern task get(output int value, input int extra);
  extern task get_later(output int value, input int extra);
endclass
task C::get(output int value, input int extra); read_shared(value,extra); endtask
task C::get_later(output int value, input int extra); later(value,extra); endtask
module child;
  int shared=99;
  int immediate, delayed;
  C c;
  initial begin
    c=new; c.get(immediate,shared);
    $display("T|immediate|%0d %0d",immediate,shared);
    c.get_later(delayed,shared);
    $display("T|delayed|%0d %0d %0d",delayed,shared,$time);
  end
endmodule
module top; child u0(),u1(); endmodule
"#,
        100,
    )
    .expect("simulate");
    let mut lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|s| s.starts_with("T|"))
        .collect();
    lines.sort_unstable();
    assert_eq!(
        lines,
        [
            "T|delayed|102 99 1",
            "T|delayed|102 99 1",
            "T|immediate|102 99",
            "T|immediate|102 99"
        ]
    );
}
