//! A VIRTUAL method call used as a binary operand / compound-assignment RHS
//! must evaluate ONCE. UVM's `uvm_event#(T)::trigger` does
//! `skip += cb_q[i].pre_trigger(this, data)` where `cb_q[i]` is a queue
//! element; with a CLASS-METHOD receiver the width-probe once ran the method
//! purely to read its width and then re-ran it for the value — so a
//! counter-flip callback (every other trigger) fired `pre_trigger` TWICE per
//! `trigger` and a parameterized event-callback test reported
//! `pre_trigger of 10, saw: 20`. The width comes from the method's declared
//! return type, resolved through the inheritance chain and the receiver's
//! class (a class FIELD of `this`, or a queue element), NEVER from executing
//! the method.
//!
//! This also covers the TYPEDEF'd element type: `cb_type cb_q[$]` where
//! `typedef event_cb cb_type` holds the receiver class, so a queue of a
//! typedef'd callback still resolves the method width without running it.
use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

fn run(src: &str) -> String {
    std::fs::write("/tmp/event_cb_binary_eval.sv", src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", "/tmp/event_cb_binary_eval.sv"])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const SRC: &str = r#"module top;
  class evt;
    function int d(); return 7; endfunction
  endclass
  class base_cb;
    virtual function bit pre(evt e, int x); return 0; endfunction
    virtual function bit post(evt e, int x); return 0; endfunction
  endclass
  class my_cb extends base_cb;
    int pre_count;
    int post_count;
    virtual function bit pre(evt e, int x);
      pre_count++;
      return (pre_count % 2);   // block every other trigger
    endfunction
    virtual function bit post(evt e, int x);
      post_count++;
      return 0;
    endfunction
  endclass
  class boxer;
    // a TYPEDEF'd element type, like `cb_type cb_q[$]` in UVM's event
    typedef base_cb cb_type;
    my_cb m;                 // class-FIELD receiver of this
    base_cb q[$];            // a queue of class elements (holds my_cb dyn)
    evt e;
    function new();
      m = new;
      e = new;
      q.push_back(m);        // upcast my_cb -> base_cb (dyn type preserved)
    endfunction
    function void trigger();
      int skip;
      skip = 0;
      // field receiver inside a class method, compound `+=`
      skip += m.pre(e, 1);
      // queue-element receiver
      skip += q[0].post(e, 2);
    endfunction
    function int pre_cnt(); return m.pre_count; endfunction
    function int post_cnt(); return m.post_count; endfunction
  endclass
  initial begin
    boxer b;
    b = new;
    repeat(10) b.trigger();
    $display("RESULT pre=%0d post=%0d", b.pre_cnt(), b.post_cnt());
    if (b.pre_cnt() == 10 && b.post_cnt() == 10)
      $display("RESULT PASS single_eval");
    else
      $display("RESULT FAIL double_eval");
    $finish;
  end
endmodule
"#;

#[test]
fn virtual_method_in_binary_is_evaluated_once() {
    let out = run(SRC);
    assert!(
        out.contains("RESULT PASS single_eval"),
        "a class method used in a binary/compound-assignment must run ONCE per\n\
         call (width comes from the declared return type, not from executing\n\
         the method). Pre-fix the event callback fired twice per trigger\n\
         (pre=20, post=20 for 10 triggers):\n{out}"
    );
    assert!(
        out.contains("RESULT pre=10 post=10"),
        "expected exactly 10 pre and 10 post callback executions:\n{out}"
    );
}