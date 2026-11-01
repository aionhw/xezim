//! §18.5.9, §18.3: a rand sub-object that no constraint of the class reads
//! shares no constraint with it, so it must not keep the class's own
//! constraints out of the joint solve. A sub-object holding a rand dynamic
//! array sent the class's `sum()` constraint to the per-variable trials, and
//! randomize() returned 0 on a satisfiable class. The `read` and `back`
//! cases (a sub-object a class constraint reads, and one whose constraint
//! reads the class through a handle) pass without the fix and guard that
//! both are still solved with the class.
#[test]
fn unread_sub_object_keeps_parent_in_joint_solve() {
    let result = xezim::simulate(
        r#"
class sub_c;
  rand bit [7:0] r[];
  function new(); r = new[4]; endfunction
  constraint s_c { foreach (r[i]) r[i] > 8'd10; }
endclass
class cfg_c;
  rand sub_c sub;
  rand int main_cnt;
  rand int part[];
  int total = 10000;
  constraint k {
    part.size() == 5;
    main_cnt inside {[10 : total]};
    foreach (part[i]) part[i] inside {[10 : total]};
    main_cnt + part.sum() == total;
  }
  function new(); sub = new(); endfunction
endclass
class leaf_c;
  rand bit [7:0] v;
endclass
class read_c;
  rand leaf_c leaf;
  rand bit [7:0] x;
  constraint k { x < 8'd100; leaf.v == x + 8'd1; }
  function new(); leaf = new(); endfunction
endclass
class back_c;
  rand back_sub_c sub;
  rand bit [7:0] x;
  constraint k { x inside {[1:5]}; }
  function new(); sub = new(); sub.p = this; endfunction
endclass
class back_sub_c;
  back_c p;
  rand bit [7:0] y;
  constraint s { y < p.x; }
endclass
module t;
  initial begin
    cfg_c c = new();
    read_c rd = new();
    back_c bk = new();
    int ok = 0, good = 0;
    for (int k = 0; k < 5; k++) begin
      if (!c.randomize()) continue;
      ok++;
      if (c.main_cnt + c.part.sum() == 10000 && c.part.size() == 5
          && c.sub.r[0] > 10 && c.sub.r[3] > 10)
        good++;
    end
    $display("unread ok %0d good %0d", ok, good);
    ok = 0;
    good = 0;
    repeat (20) if (rd.randomize()) begin
      ok++;
      if (rd.x < 100 && rd.leaf.v == rd.x + 1) good++;
    end
    $display("read ok %0d good %0d", ok, good);
    ok = 0;
    good = 0;
    repeat (20) if (bk.randomize()) begin
      ok++;
      if (bk.x inside {[1:5]} && bk.sub.y < bk.x) good++;
    end
    $display("back ok %0d good %0d", ok, good);
    $finish;
  end
endmodule
"#,
        100_000,
    )
    .unwrap();
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(
        lines,
        ["unread ok 5 good 5", "read ok 20 good 20", "back ok 20 good 20"]
    );
}
