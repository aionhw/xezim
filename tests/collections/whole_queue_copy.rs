//! IEEE 1800-2023 §7.5: a whole dynamic-array / queue assignment copies the
//! SIZE and every element. `b.q = a.q` (or `q = rhs.q` inside a `copy` method)
//! must not store a scalar — both sides are collections stored per-instance
//! as `<handle>#<member>[i]` + a `.size` shadow.
//!
//! Previously only the associative-array whole-copy path existed; a
//! queue/dynamic-array copy between class members fell through to a scalar
//! assignment that left the destination empty. This broke UVM `do_copy` (the
//! destination object's dynamic arrays stayed at size 0) and, through the
//! resulting pack/unpack misalignment, the entire UVM pack suite.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// `b.q = a.q` between two objects copies size + elements.
const CLASS_QUEUE_COPY: &str = r#"
class item;
  shortint q[];
  function new(); endfunction
  function void copy(item rhs);
    q = rhs.q;
  endfunction
endclass
module top;
  initial begin
    item a, b;
    a = new; a.q = new[3]; a.q[0]=10; a.q[1]=-50; a.q[2]=30000;
    b = new; b.copy(a);
    $display("SIZE %0d %0d %0d %0d", b.q.size(), b.q[0], b.q[1], b.q[2]);
    if (b.q.size()==3 && b.q[0]==10 && b.q[1]==-50 && b.q[2]==30000)
      $display("TAG_PASS");
    else $display("TAG_FAIL");
  end
endmodule
"#;

#[test]
fn whole_queue_copy_between_objects() {
    let sim = simulate(CLASS_QUEUE_COPY, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m.starts_with("SIZE 3")),
        "expected size 3, got: {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "expected TAG_PASS, got: {:?}",
        msgs
    );
}

/// `this.q = rhs.q` (explicit this) also works.
const EXPLICIT_THIS_COPY: &str = r#"
class item;
  shortint q[];
  function new(); endfunction
  function void copy(item rhs);
    this.q = rhs.q;
  endfunction
endclass
module top;
  initial begin
    item a, b;
    a = new; a.q = new[2]; a.q[0]=7; a.q[1]=8;
    b = new; b.copy(a);
    $display("SIZE %0d %0d %0d", b.q.size(), b.q[0], b.q[1]);
    if (b.q.size()==2 && b.q[0]==7 && b.q[1]==8) $display("TAG_PASS");
    else $display("TAG_FAIL");
  end
endmodule
"#;

#[test]
fn explicit_this_queue_copy() {
    let sim = simulate(EXPLICIT_THIS_COPY, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m.starts_with("SIZE 2")),
        "expected size 2, got: {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "expected TAG_PASS, got: {:?}",
        msgs
    );
}

/// Copying a SMALLER array into a destination shrinks it (stale elements
/// beyond the new size are dropped).
const SHRINK_ON_COPY: &str = r#"
class item;
  int q[];
  function new(); endfunction
  function void copy(item rhs);
    q = rhs.q;
  endfunction
endclass
module top;
  initial begin
    item a, b;
    a = new; a.q = new[2]; a.q[0]=1; a.q[1]=2;
    b = new; b.q = new[5];
    b.q[0]=9; b.q[1]=9; b.q[2]=9; b.q[3]=9; b.q[4]=9;
    b.copy(a);
    $display("SIZE %0d", b.q.size());
    if (b.q.size()==2) $display("TAG_PASS");
    else $display("TAG_FAIL");
  end
endmodule
"#;

#[test]
fn queue_copy_shrinks_destination() {
    let sim = simulate(SHRINK_ON_COPY, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "SIZE 2"),
        "expected size 2 (shrunk), got: {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "expected TAG_PASS, got: {:?}",
        msgs
    );
}
