//! §18.5.9, §18.5.8.1: a class constraint on a method of a rand sub-object's
//! array (`sub.r.sum() == 60`) constrains only that sub-object, so it joins
//! the sub-object's own solve, where its `sum()` is solved jointly with the
//! sub-object's `foreach` constraint. It used to stay with the enclosing
//! solve only, which cannot model a sub-object's array, so randomize()
//! succeeded only when a trial's draw happened to hit the sum and the
//! shared trial budget ran out on some calls.
#[test]
fn sum_of_sub_object_array_is_solved_in_the_sub_object() {
    let result = xezim::simulate(
        r#"
class sub_c;
  rand bit [7:0] r[];
  rand bit [7:0] v;
  function new(); r = new[4]; endfunction
  constraint s_c { foreach (r[i]) r[i] > 8'd10; }
endclass
class call_c;
  rand sub_c sub;
  rand int main_cnt;
  constraint k { main_cnt inside {[1:100]}; sub.r.sum() == 60; }
  function new(); sub = new(); endfunction
endclass
class only_c;
  rand sub_c sub;
  constraint k { sub.r.sum() == 60; }
  function new(); sub = new(); endfunction
endclass
module t;
  initial begin
    call_c c = new();
    only_c o = new();
    int ok = 0, good = 0, sum;
    repeat (10) if (c.randomize()) begin
      ok++;
      sum = 0;
      foreach (c.sub.r[i]) begin sum += c.sub.r[i]; if (c.sub.r[i] <= 10) sum = -1000; end
      if (sum % 256 == 60 && c.main_cnt inside {[1:100]}) good++;
    end
    $display("call ok %0d good %0d", ok, good);
    ok = 0; good = 0;
    repeat (10) if (o.randomize()) begin
      ok++;
      sum = 0;
      foreach (o.sub.r[i]) begin sum += o.sub.r[i]; if (o.sub.r[i] <= 10) sum = -1000; end
      if (sum % 256 == 60) good++;
    end
    $display("only ok %0d good %0d", ok, good);
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
    assert_eq!(lines, ["call ok 10 good 10", "only ok 10 good 10"]);
}
