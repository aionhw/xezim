//! §7.13: a member of a struct-typed ELEMENT of a class collection.
//!
//! `m[k1][k2]...[kn] = '{...}` writes an (unpacked) struct into a class
//! collection element, stored member-wise as `<elemkey>.<member>` cells (the
//! same model UVM's recurrence guard `m_recur_states[l][r][p].state` relies
//! on). Field reads `m[...].state` project those cells. Previously a multidim
//! assoc element of struct type read every field back as 0 — which made UVM's
//! `state != FINISHED` cycle guard always read `NEVER`, so `compare` /
//! `do_compare` over a cyclic object graph recursed forever and overflowed
//! the stack.

use xezim::simulate;

fn sim_src(src: &str) -> Vec<String> {
    let sim = simulate(src, 200).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// 3-D associative array of structs as a class member — the exact shape of
/// UVM's comparer recurrence-state table (`state_info_t m_rec[a][b][c]`).
const MULTIDIM_STRUCT_ELEM: &str = r#"
module top;
  typedef struct { int state; int ret_val; } state_info_t;
  class C;
    state_info_t m_rec[int][int][int];
    function new(); endfunction
    function void test();
      m_rec[1][2][3] = '{5, 7};
      m_rec[10][20][30].state = 42;
      m_rec[10][20][30].ret_val = 99;
      $display("DOT1 %0d %0d", m_rec[1][2][3].state, m_rec[1][2][3].ret_val);
      $display("DOT2 %0d %0d", m_rec[10][20][30].state, m_rec[10][20][30].ret_val);
      if (m_rec[1][2][3].state == 5 && m_rec[1][2][3].ret_val == 7 &&
          m_rec[10][20][30].state == 42 && m_rec[10][20][30].ret_val == 99)
        $display("TAG_PASS");
      else
        $display("TAG_FAIL");
    endfunction
  endclass
  initial begin C c = new; c.test(); end
endmodule
"#;

#[test]
fn multidim_struct_element_field_access() {
    let msgs = sim_src(MULTIDIM_STRUCT_ELEM);
    assert!(
        msgs.iter().any(|m| m == "DOT1 5 7"),
        "expected field reads of pattern-written element, got: {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m == "DOT2 42 99"),
        "expected field reads of separately-written cells, got: {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "expected TAG_PASS, got: {:?}",
        msgs
    );
}