//! §23.9: a name declared in a subroutine is the local item, and §8.10: a
//! class property is the item named inside the class's methods. A scalar
//! formal, local or property therefore hides a MODULE-level collection of
//! the same name: `str.len()`, `str[i]`, `return str` and `str.substr()` on a
//! `string str` formal read that string, not the module's `string str[$]`.

fn run(src: &str) -> Vec<String> {
    xezim::simulate(src, 100_000)
        .unwrap()
        .output
        .iter()
        .map(|entry| entry.message.clone())
        .collect()
}

#[test]
fn unit_and_module_function_scalar_formals_hide_module_queues() {
    let lines = run(r#"
function automatic int flen(string str);
  return str.len();
endfunction
function automatic string fpush(string str);
  str = {str, "!"};
  return str;
endfunction
function automatic int fidx(string str);
  return str[0];
endfunction
module top;
  string str[$];
  int v[$];
  int w[$];
  function automatic int cnt(int v);
    return v + 1;
  endfunction
  function automatic int wsum(int w);
    int r;
    r = w * 2;
    w = w + 1;
    return r + w;
  endfunction
  function automatic string sub(string str);
    return str.substr(1, 2);
  endfunction
  function automatic int gc(string str);
    return str.getc(1);
  endfunction
  initial begin
    str = {"a", "bb", "ccc"};
    v = {1, 2};
    w = {7, 8, 9};
    $display("T|%0d %0d %0d %0d", flen("hello"), cnt(10), str.size(), v.size());
    $display("T|%s %0d %s %0d", fpush("hi"), fidx("Az"), sub("hello"), gc("xyz"));
    $display("T|%0d %0d %0d", wsum(5), w.size(), str.size());
    $finish;
  end
endmodule
"#);
    assert_eq!(lines, vec!["T|5 11 3 2", "T|hi! 65 el 121", "T|16 3 3"]);
}

#[test]
fn module_task_scalar_formals_hide_module_collections() {
    let lines = run(r#"
module top;
  string str[$];
  int v[$];
  int da[];
  task automatic twr(string str, int v, int da);
    str = {str, "-xyz"};
    v = v * 3;
    da = da + 100;
    $display("T|%s %0d %0d %0d %0d", str, v, da, str.len(), str[1]);
  endtask
  function automatic int fda(int da);
    return da + 1;
  endfunction
  initial begin
    str = {"a", "bb", "ccc"};
    v = {1, 2};
    da = new[4];
    twr("q", 5, 6);
    $display("T|%0d %0d %0d %0d", fda(41), str.size(), v.size(), da.size());
    $finish;
  end
endmodule
"#);
    assert_eq!(lines, vec!["T|q-xyz 15 106 5 45", "T|42 3 2 4"]);
}

#[test]
fn class_scalar_properties_hide_module_queues() {
    let lines = run(r#"
class c;
  string str;
  int v;
  function new(); str = "abcd"; v = 7; endfunction
  function int slen(); return str.len(); endfunction
  function int vget(); return v; endfunction
  function void sset(string s); str = s; endfunction
  function string sidx(); return $sformatf("%c", str[1]); endfunction
  function void vinc(); v = v + 1; endfunction
endclass
module top;
  string str[$];
  int v[$];
  c o;
  initial begin
    str = {"a", "bb", "ccc"};
    v = {1, 2};
    o = new;
    $display("T|%0d %0d %s %0d %0d", o.slen(), o.vget(), o.sidx(), str.size(), v.size());
    o.sset("zz"); o.vinc();
    $display("T|%0d %0d %s %0d %0d", o.slen(), o.vget(), o.sidx(), str.size(), v.size());
    str.push_back("d"); v.push_back(3);
    $display("T|%0d %0d %0d %0d", o.slen(), o.vget(), str.size(), v.size());
    $finish;
  end
endmodule
"#);
    assert_eq!(lines, vec!["T|4 7 b 3 2", "T|2 8 z 3 2", "T|2 8 4 3"]);
}

#[test]
fn return_of_a_shadowing_scalar_is_the_scalar() {
    let lines = run(r#"
class c;
  string str;
  int v;
  function new(); str = "abcd"; v = 7; endfunction
  function string sget(); return str; endfunction
  function int vget(); return v; endfunction
  function string ssub(); return str.substr(1, 2); endfunction
  function int sgc(); return str.getc(0); endfunction
endclass
module top;
  int v[$];
  string str[$];
  int da[];
  c o;
  function automatic int idf(int v);
    return v;
  endfunction
  function automatic string sidf(string str);
    return str;
  endfunction
  function automatic int daf(int da);
    return da;
  endfunction
  initial begin
    v = {1, 2}; str = {"a", "bb", "ccc"}; da = new[2];
    o = new;
    $display("T|%0d %s %0d", idf(41), sidf("xy"), daf(9));
    $display("T|%s %0d %s %0d", o.sget(), o.vget(), o.ssub(), o.sgc());
    $display("T|%0d %0d %0d", v.size(), str.size(), da.size());
    $finish;
  end
endmodule
"#);
    assert_eq!(lines, vec!["T|41 xy 9", "T|abcd 7 bc 97", "T|2 3 2"]);
}
