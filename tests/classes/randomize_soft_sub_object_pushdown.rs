//! §18.5.14.1, §18.5.9: a `soft` item of the class that reads a rand
//! sub-object (`soft sub.v == 7`) is pushed into the sub-object's own solve
//! as an in-line item. The soft-priority ranking indexed a per-block depth
//! table that held no entry for the in-line block, so the nested randomize
//! panicked when the sub-object's dynamic array sent it through the trial
//! loop. In-line soft items outrank the class's own, and a later in-line
//! item outranks an earlier one.
#[test]
fn soft_item_on_sub_object_and_inline_soft_rank_above_class_items() {
    let result = xezim::simulate(
        r#"
class sub_c;
  rand bit [7:0] r[];
  rand bit [7:0] v;
  function new(); r = new[4]; endfunction
  constraint s_c { foreach (r[i]) r[i] > 8'd10; }
endclass
class soft_c;
  rand sub_c sub;
  rand int main_cnt;
  constraint k { main_cnt inside {[1:100]}; soft sub.v == 7; }
  function new(); sub = new(); endfunction
endclass
class dyn_c;
  rand bit [7:0] r[];
  rand bit [7:0] v;
  function new(); r = new[3]; endfunction
  constraint s_c { foreach (r[i]) r[i] > 8'd10; soft v == 3; }
endclass
module t;
  initial begin
    soft_c s = new();
    dyn_c d = new();
    int ok = 0, good = 0;
    repeat (5) if (s.randomize()) begin
      ok++;
      if (s.sub.v == 7 && s.main_cnt inside {[1:100]} && s.sub.r[0] > 10) good++;
    end
    $display("soft ok %0d good %0d", ok, good);
    ok = 0; good = 0;
    repeat (5) if (d.randomize()) begin ok++; if (d.v == 3) good++; end
    $display("class ok %0d good %0d", ok, good);
    ok = 0; good = 0;
    repeat (5) if (d.randomize() with { soft v == 7; }) begin ok++; if (d.v == 7) good++; end
    $display("inline ok %0d good %0d", ok, good);
    ok = 0; good = 0;
    repeat (5) if (d.randomize() with { soft v == 7; soft v == 9; }) begin ok++; if (d.v == 9) good++; end
    $display("later ok %0d good %0d", ok, good);
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
        [
            "soft ok 5 good 5",
            "class ok 5 good 5",
            "inline ok 5 good 5",
            "later ok 5 good 5"
        ]
    );
}
