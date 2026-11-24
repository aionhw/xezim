//! §21.2.1.7 — `%p` of a DIRECT (unassigned) queue/array slice must render the
//! element list `'{...}`, not the concatenated packed value. `q[2:$]` resolves
//! `$` to the last live index. Verified against a commercial simulator.

use xezim::simulate;

fn line(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("sim")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn direct_queue_slice_p_format() {
    let src = r#"
module t;
  int q[$] = '{1,2,3,4,5};
  initial begin
    $display("A=%p", q[1:3]);   // '{2, 3, 4}
    $display("B=%p", q[2:$]);   // '{3, 4, 5}
    $display("C=%p", q[0:0]);   // '{1}
    $finish;
  end
endmodule
"#;
    let out = line(src);
    for w in ["A='{2, 3, 4}", "B='{3, 4, 5}", "C='{1}"] {
        assert!(out.iter().any(|m| m == w), "missing {:?}; got {:?}", w, out);
    }
}

/// §7.4.6, §7.10.1, §11.3.2 (#1540).
#[test]
fn slice_object_collections_and_evaluate_each_receiver_once() {
    let src = r#"
module top;
  class obj_t;
    int q[$], copy[$], dyn[];
    int fixed[0:3];
    function void read_this();
      int tmp[$];
      tmp = this.q[0:1];
      this.copy = q[2-:2];
      $display("METHOD %p %p", tmp, copy);
    endfunction
  endclass
  obj_t obj;
  int gets;
  int got[$];
  function obj_t receiver(); gets++; return obj; endfunction
  initial begin
    obj = new;
    obj.q = {7,8,9,10}; obj.dyn = {11,12,13,14}; obj.fixed = '{21,22,23,24};
    got = receiver().q[1:$]; $display("TAIL %p %0d", got, gets);
    got = {receiver().q[0+:2],receiver().q[2-:2]}; $display("CONCAT %p %0d", got, gets);
    got = receiver().dyn[1:2]; $display("DYN %p %0d", got, gets);
    got = receiver().fixed[1:2]; $display("FIXED %p %0d", got, gets);
    got = obj.q[9:$]; $display("EMPTY %p", got);
    obj.read_this();
  end
endmodule
"#;
    assert_eq!(
        line(src),
        [
            "TAIL '{8, 9, 10} 1",
            "CONCAT '{7, 8, 8, 9} 3",
            "DYN '{12, 13} 4",
            "FIXED '{22, 23} 5",
            "EMPTY '{}",
            "METHOD '{7, 8} '{8, 9}"
        ]
    );
}
