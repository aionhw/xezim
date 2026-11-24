//! A write to a member of a packed struct nested in an element of an
//! unpacked-array class property (`inst[i].arch.vl = v`).
//!
//! The store resolved a member written directly on the element
//! (`inst[i].op = v`) as a slice of the element, but not a member one or more
//! packed structs further down. Such a write fell through to paths that do
//! not know the element's per-object storage, and was dropped: the element
//! kept its old value. Expected values match Verilator.

use xezim::simulate;

fn messages(src: &str, max_time: u64) -> Vec<String> {
    let sim = simulate(src, max_time).expect("simulation failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// IEEE 1800-2023 §7.2.1, §7.4.6: in a task, a member of a nested packed
/// struct in an element selected by a loop variable is written in place,
/// beside a member of the element itself. A nested member of a scalar
/// property and of a module-scope array are written as before.
#[test]
fn task_writes_nested_member_of_element_by_loop_variable() {
    let src = r#"
module t;
  typedef struct packed { logic [31:0] vl; logic [2:0] sew; } st_t;
  typedef struct packed { logic [7:0] op; st_t arch; } cmd_t;
  class C;
    cmd_t inst [1:0];
    cmd_t one;
    task fill();
      for (int i = 0; i < 2; i++) begin
        inst[i] = '0;
        inst[i].op = 8'h5;
        inst[i].arch.vl = 32'd16 + i;
        inst[i].arch.sew = 3'b001;
      end
      one = '0;
      one.arch.vl = 32'd7;
    endtask
  endclass
  cmd_t m_inst [1:0];
  initial begin
    C c = new;
    c.fill();
    $display("task: op=%h vl=%0d sew=%0d | vl1=%0d | one.vl=%0d", c.inst[0].op, c.inst[0].arch.vl, c.inst[0].arch.sew, c.inst[1].arch.vl, c.one.arch.vl);
    $display("whole0=%h whole1=%h", c.inst[0], c.inst[1]);
    m_inst[1] = '0;
    m_inst[1].arch.vl = 32'd5;
    $display("module: vl=%0d whole=%h", m_inst[1].arch.vl, m_inst[1]);
  end
endmodule
"#;
    let out = messages(src, 100);
    assert_eq!(
        out,
        [
            "task: op=05 vl=16 sew=1 | vl1=17 | one.vl=7",
            "whole0=02800000081 whole1=02800000089",
            "module: vl=5 whole=00000000028",
        ],
        "output: {out:?}"
    );
}

/// IEEE 1800-2023 §7.2.1, §7.4.6: in a function, the same write with a
/// constant element select, read back through other methods.
#[test]
fn function_writes_nested_member_with_constant_select() {
    let src = r#"
module t;
  typedef struct packed { logic [31:0] vl; logic [2:0] sew; } st_t;
  typedef struct packed { logic [7:0] op; st_t arch; } cmd_t;
  class C;
    cmd_t inst [1:0];
    function void set_vl();
      inst[1] = '0;
      inst[1].op = 8'ha5;
      inst[1].arch.vl = 32'd99;
    endfunction
    function int get_vl(int k);
      return inst[k].arch.vl;
    endfunction
    function cmd_t get(int k);
      return inst[k];
    endfunction
  endclass
  initial begin
    C c = new;
    c.set_vl();
    $display("func: vl=%0d op=%h whole=%h", c.get_vl(1), c.inst[1].op, c.get(1));
  end
endmodule
"#;
    let out = messages(src, 100);
    assert_eq!(
        out,
        ["func: vl=99 op=a5 whole=52800000318"],
        "output: {out:?}"
    );
}

/// IEEE 1800-2023 §7.2.1: a member three packed levels down
/// (`arr[i].mid.leaf.hi`) lands at the sum of the members' offsets and
/// leaves its siblings at every level alone.
#[test]
fn three_level_nested_member_of_element() {
    let src = r#"
module t;
  typedef struct packed { logic [3:0] hi; logic [3:0] lo; } leaf_t;
  typedef struct packed { logic [7:0] tag; leaf_t leaf; } mid_t;
  typedef struct packed { logic [7:0] op; mid_t mid; } top_t;
  class C;
    top_t arr [3];
    task fill();
      foreach (arr[i]) begin
        arr[i] = '0;
        arr[i].op = 8'h10 + i;
        arr[i].mid.tag = 8'h20 + i;
        arr[i].mid.leaf.hi = 4'h3;
        arr[i].mid.leaf.lo = i;
      end
      arr[2].mid.leaf.hi = 4'hc;
    endtask
    function string show(int k);
      return $sformatf("%h:%h:%h", arr[k], arr[k].mid.leaf, arr[k].mid.leaf.hi);
    endfunction
  endclass
  initial begin
    C c = new;
    c.fill();
    $display("deep: %s %s %s", c.show(0), c.show(1), c.show(2));
  end
endmodule
"#;
    let out = messages(src, 100);
    assert_eq!(
        out,
        ["deep: 102030:30:3 112131:31:3 1222c2:c2:c"],
        "output: {out:?}"
    );
}

/// IEEE 1800-2023 §7.2.1, §8.4: the element may be reached through `this` or
/// an object handle, and may belong to a queue or a dynamic array property.
#[test]
fn nested_member_through_this_handle_and_collections() {
    let src = r#"
module t;
  typedef struct packed { logic [31:0] vl; logic [2:0] sew; } st_t;
  typedef struct packed { logic [7:0] op; st_t arch; } cmd_t;
  class C;
    cmd_t inst [1:0];
    cmd_t q [$];
    cmd_t d [];
    task fill();
      inst[0] = '0;
      this.inst[0].arch.vl = 32'd21;
      q.push_back('0);
      q[0].arch.vl = 32'd7;
      d = new[2];
      d[1] = '0;
      d[1].arch.sew = 3'd5;
    endtask
  endclass
  initial begin
    C c = new;
    c.fill();
    c.inst[1] = '0;
    c.inst[1].arch.sew = 3'd6;
    $display("forms: this=%h handle=%h q=%h d=%h", c.inst[0], c.inst[1], c.q[0], c.d[1]);
  end
endmodule
"#;
    let out = messages(src, 100);
    assert_eq!(
        out,
        ["forms: this=000000000a8 handle=00000000006 q=00000000038 d=00000000005"],
        "output: {out:?}"
    );
}
