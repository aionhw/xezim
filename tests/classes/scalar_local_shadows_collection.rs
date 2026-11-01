//! §23.9: a name declared locally in a subroutine is the local item. A scalar
//! formal or local therefore names its own variable, not a caller's
//! same-named local queue and not a same-named queue property that a
//! subclass of the method's class declares.

fn run(src: &str) -> Vec<String> {
    xezim::simulate(src, 100_000)
        .unwrap()
        .output
        .iter()
        .map(|entry| entry.message.clone())
        .collect()
}

#[test]
fn scalar_formal_and_local_shadow_caller_local_queue() {
    let lines = run(r#"
function automatic void split(string str, byte sep, ref string values[$]);
  int s = 0, e = 0;
  values.delete();
  while (e < str.len()) begin
    for (s = e; e < str.len(); ++e)
      if (str[e] == sep) break;
    if (s != e) values.push_back(str.substr(s, e - 1));
    e++;
  end
endfunction
class cfg;
  function string parse(string arg);
    string fields[$];
    split(arg, ",", fields);
    return $sformatf("%0d %p", fields.size(), fields);
  endfunction
  function int count();
    string str;
    str = "abcde";
    return str.len();
  endfunction
  task tparse(string str, output int n);
    n = str.len();
  endtask
endclass
class user;
  cfg c = new();
  function void outer();
    string str[$];
    int n;
    str = {"first", "second"};
    $display("split %s", c.parse("A:0,BB:1,C:2"));
    $display("local %0d", c.count());
    c.tparse("abc", n);
    $display("task %0d", n);
    $display("own %0d", str.size());
  endfunction
endclass
module t;
  initial begin
    user u = new();
    u.outer();
    $finish;
  end
endmodule
"#);
    assert_eq!(
        lines,
        [
            r#"split 3 '{"A:0", "BB:1", "C:2"}"#,
            "local 5",
            "task 3",
            "own 2"
        ]
    );
}

#[test]
fn base_method_scalar_local_shadows_subclass_queue_property() {
    let lines = run(r#"
class base_c;
  function int fn();
    int v = 5;
    v = v + 1;
    return v;
  endfunction
  function string fs();
    string str;
    str = "ab";
    return str;
  endfunction
endclass
class sub_c extends base_c;
  int v[$];
  string str[$];
endclass
module t;
  initial begin
    sub_c s = new();
    int n;
    string r;
    n = s.fn();
    r = s.fs();
    $display("sub %0d %s %0d %0d", n, r, s.v.size(), s.str.size());
    $finish;
  end
endmodule
"#);
    assert_eq!(lines, ["sub 6 ab 0 0"]);
}
