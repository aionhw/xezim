//! §18.4, §7.8.2: a rand associative array keeps its keys under randomize().
//! With an `int` key, the `foreach` loop variable holds the key at the index
//! type, so a negative key stays negative in the body (`i < 0 ? 1 : 2`); it
//! was bound as a 32-bit unsigned value, and the joint solve's answer was
//! rejected by a checker that expected the other branch. A string-keyed
//! array is left to the per-element draw, and the rest of the class (a
//! `sum()` constraint only the joint solve satisfies) is still solved
//! jointly.
#[test]
fn negative_and_string_assoc_keys_keep_the_class_in_the_joint_solve() {
    let result = xezim::simulate(
        r#"
class neg_c;
  rand int main_cnt;
  rand int sub_cnt[];
  rand bit [3:0] neg[int];
  constraint c {
    sub_cnt.size() == 3;
    main_cnt inside {[10:1000]};
    foreach (sub_cnt[i]) sub_cnt[i] inside {[10:1000]};
    main_cnt + sub_cnt.sum() == 1000;
  }
  constraint n { foreach (neg[i]) neg[i] == (i < 0 ? 4'd1 : 4'd2); }
  function new(); neg[-3] = 0; neg[7] = 0; endfunction
endclass
class str_c;
  rand int main_cnt;
  rand int sub_cnt[];
  rand bit [3:0] str[string];
  constraint c {
    sub_cnt.size() == 3;
    main_cnt inside {[10:1000]};
    foreach (sub_cnt[i]) sub_cnt[i] inside {[10:1000]};
    main_cnt + sub_cnt.sum() == 1000;
  }
  constraint s { foreach (str[k]) str[k] == 4'd5; }
  function new(); str["a"] = 0; str["b"] = 0; endfunction
endclass
module t;
  initial begin
    neg_c n = new();
    str_c s = new();
    int ok = 0, good = 0;
    repeat (4) if (n.randomize()) begin
      ok++;
      if (n.main_cnt + n.sub_cnt.sum() == 1000 && n.neg[-3] == 1 && n.neg[7] == 2 && n.neg.num() == 2)
        good++;
    end
    $display("neg ok %0d good %0d", ok, good);
    ok = 0; good = 0;
    repeat (4) if (s.randomize()) begin
      ok++;
      if (s.main_cnt + s.sub_cnt.sum() == 1000 && s.str["a"] == 5 && s.str["b"] == 5 && s.str.num() == 2)
        good++;
    end
    $display("str ok %0d good %0d", ok, good);
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
    assert_eq!(lines, ["neg ok 4 good 4", "str ok 4 good 4"]);
}
