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

/// §7.2, §7.4.6, §7.10.1 (#1540 cousin): slicing queues and arrays of unpacked structs.
#[test]
fn slice_unpacked_struct_queues_and_arrays() {
    let src = r#"
module t;
  typedef struct { int a; int b; } us_t;
  typedef us_t us_q[$];
  class C;
    us_t sq[$];
    function us_q method_slice();
      return sq[1:2];
    endfunction
  endclass
  us_t sq_mod[$], fa[2];
  C obj;
  function automatic us_q free_slice();
    return obj.sq[0:1];
  endfunction
  initial begin
    us_t sq_loc[$], sq_fn[$];
    obj = new;
    obj.sq = '{'{1, 2}, '{3, 4}, '{5, 6}};
    sq_loc = obj.sq[0:1];
    sq_mod = obj.sq[0:1];
    fa = obj.sq[1:2];
    sq_fn = free_slice();
    $display("LOC %0d %0d %0d", sq_loc.size(), sq_loc[0].a, sq_loc[1].b);
    $display("MOD %0d %0d %0d", sq_mod.size(), sq_mod[0].a, sq_mod[1].b);
    $display("FA %0d %0d %0d %0d", fa[0].a, fa[0].b, fa[1].a, fa[1].b);
    $display("FN %0d %0d %0d", sq_fn.size(), sq_fn[0].a, sq_fn[1].b);
    sq_fn = obj.method_slice();
    $display("MTH %0d %0d %0d", sq_fn.size(), sq_fn[0].a, sq_fn[1].b);
    sq_mod = sq_mod[1:1];
    $display("OV1 %0d %0d %0d", sq_mod.size(), sq_mod[0].a, sq_mod[0].b);
    obj.sq = obj.sq[1:2];
    $display("OV2 %0d %0d %0d", obj.sq.size(), obj.sq[0].a, obj.sq[1].b);
    fa = free_slice();
    $display("FAF %0d %0d %0d %0d", fa[0].a, fa[0].b, fa[1].a, fa[1].b);
    sq_loc = obj.sq[9:$];
    $display("EMP %0d", sq_loc.size());
  end
endmodule
"#;
    assert_eq!(
        line(src),
        [
            "LOC 2 1 4",
            "MOD 2 1 4",
            "FA 3 4 5 6",
            "FN 2 1 4",
            "MTH 2 3 6",
            "OV1 1 3 4",
            "OV2 2 3 6",
            "FAF 3 4 5 6",
            "EMP 0",
        ]
    );
}
