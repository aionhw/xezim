//! IEEE 1800-2017 §8.24, §8.23: the out-of-block definition of a nested
//! class's method, `function int outer::inner::m(...)`, names the nested
//! class through the whole scope chain. It was a parse error (#617, reported
//! by git-hevangel); its body now runs, for instance, static and same-named
//! methods, beside the outer class's own.
use xezim::simulate;

#[test]
fn nested_class_out_of_block_methods() {
    let out: Vec<String> = simulate(
        r#"// IEEE 1800-2023 8.23, 8.24, A.1.9: a method of a class nested in another class
// can be defined outside both classes. The method name is written with a
// class scope, and a class scope is a chain of classes:
//
//     function int outer::inner::m(...); ... endfunction
//
// The body belongs to `inner`, not to `outer`. Every line should print PASS.
// With the bug the definition is a parse error (`expected Semicolon, found
// DoubleColon '::'`) and nothing runs.
class outer;
  int tag = 7;
  class inner;
    int count = 0;
    extern function int bump(int step);
    extern task         clear();
    extern function int describe();
    extern static function int twice(int v);
  endclass
  extern function int describe();
endclass

function int outer::inner::bump(int step);      // two links
  count += step;
  return count;
endfunction

task outer::inner::clear();
  count = 0;
endtask

function int outer::inner::describe();           // same name as outer's method
  return 100 + count;
endfunction

function int outer::inner::twice(int v);         // static method of the nested class
  return 2 * v;
endfunction

function int outer::describe();                  // one link: unchanged
  return 200 + tag;
endfunction

module t;
  int fails = 0;

  task automatic check(string label, int got, int expected);
    if (got == expected) $display("PASS %s", label);
    else begin $display("FAIL %s: got %0d, expected %0d", label, got, expected); fails++; end
  endtask

  initial begin
    outer      o = new();
    outer::inner i = new();
    void'(i.bump(3));
    check("nested method runs its own body",       i.bump(4),             7);
    check("nested method keeps its own members",   i.count,               7);
    i.clear();
    check("nested task runs",                      i.count,               0);
    check("nested describe is the nested body",    i.describe(),          100);
    check("outer describe is the outer body",      o.describe(),          207);
    check("static method of the nested class",     outer::inner::twice(21), 42);
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
"#,
        100,
    )
    .expect("simulate")
    .output
    .iter()
    .map(|l| l.message.clone())
    .filter(|m| m.contains("PASS") || m.contains("FAIL"))
    .collect();
    assert!(out.iter().any(|m| m == "ALL PASS"), "{out:?}");
    assert!(!out.iter().any(|m| m.contains("FAIL")), "{out:?}");
}
