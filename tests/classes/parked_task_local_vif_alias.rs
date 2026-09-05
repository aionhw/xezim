//! Sibling of the `method_local_base` defect (xezim PR #150): a subroutine-
//! local `virtual ifc lv;` bound with `lv = vi;` was recorded in the
//! simulator-global alias map, keyed by the bare name. Two class tasks each
//! holding a local `lv` bound to a different interface and interleaved on
//! delays therefore saw each other's binding: a1's `lv.id` read a2's
//! interface after a2's task ran. A local's alias now lives in the frame
//! alias map, which is part of the process context and survives a park.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 10_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn parked_tasks_keep_their_own_local_vif_binding() {
    let msgs = messages(
        r#"
interface ifc(input int id); endinterface
class A;
  task run(virtual ifc vi, int k);
    virtual ifc lv;
    int i;
    lv = vi;
    for (i = 0; i < 3; i++) begin
      #(k);
      $display("A%0d i=%0d id=%0d", k, i, lv.id);
    end
  endtask
endclass
module tb;
  ifc i1(1); ifc i2(2);
  A a1 = new; A a2 = new;
  initial a1.run(i1, 5);
  initial a2.run(i2, 7);
  initial #60 $finish;
endmodule
"#,
    );
    for want in ["A5 i=0 id=1", "A5 i=1 id=1", "A5 i=2 id=1", "A7 i=0 id=2", "A7 i=1 id=2", "A7 i=2 id=2"] {
        assert!(msgs.iter().any(|m| m == want), "missing {want}; got {msgs:?}");
    }
}

// A module-scope `virtual ifc` variable (not a local) still binds globally.
#[test]
fn module_scope_vif_variable_still_binds() {
    let msgs = messages(
        r#"
interface ifc(input int id); endinterface
module tb;
  ifc i1(1); ifc i2(2);
  virtual ifc g;
  initial begin g = i2; #1 $display("G id=%0d", g.id); g = i1; #1 $display("G id=%0d", g.id); $finish; end
endmodule
"#,
    );
    assert!(msgs.iter().any(|m| m == "G id=2") && msgs.iter().any(|m| m == "G id=1"), "got {msgs:?}");
}
