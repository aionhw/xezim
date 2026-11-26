//! §13.5, §13.5.1, §25.9 (#1297): a subroutine's own formal or variable is its
//! own, whatever a caller's virtual-interface formal of the same name holds.
//! In `v.put(x)` with `function void put(int v); data = v; endfunction`,
//! called from a method whose virtual-interface formal is also `v`, the
//! callee's `v` is the integer passed: an interface function and task, a
//! module function and task, a class method and a static method as the
//! caller; a typedef'd virtual interface; a formal passed on; two calls in a
//! row (the caller's binding is back after the first). Also the callee's
//! local (top level and in a block), an output, ref, string or real formal,
//! a modport import, a recursive call, a fork in a task, and a module or
//! package variable `v` read by a callee, while the caller's own `v` still
//! works in an inline constraint. Each row uses an interface instance of its
//! own. The expected lines are the reference simulator's.
use std::process::Command;

const DESIGN_ROWS: &str = r#"
interface bus_if #(int W = 8);
  logic [W-1:0] data;
  int count = 0;
  function void put(int v);   data = v; count = count + 1; endfunction
  task          put_t(int v); data = v; count = count + 1; endtask
  function int  get();        return data; endfunction
  function int  calls();      return count; endfunction
endinterface

typedef virtual bus_if #(16) bus_vif_t;

class user_t;
  function void by_function(virtual bus_if #(16) v, int x);  v.put(x);   endfunction
  task          by_task(virtual bus_if #(16) v, int x);      v.put_t(x); endtask
  function void by_typedef(bus_vif_t v, int x);              v.put(x);   endfunction
  static function void by_static(virtual bus_if #(16) v, int x); v.put(x); endfunction
  function void by_distinct(virtual bus_if #(16) h, int x);  h.put(x);   endfunction
  function void twice(virtual bus_if #(16) v, int x);        v.put(x); v.put(x + 1); endfunction
  function void passed_on(virtual bus_if #(16) v, int x);    by_function(v, x + 2); endfunction
  function void by_local(virtual bus_if #(16) h, int x);
    virtual bus_if #(16) v;
    v = h;
    v.put(x);
  endfunction
endclass

module top;
  bus_if #(16) i0 ();
  bus_if #(16) i1 ();
  bus_if #(16) i2 ();
  bus_if #(16) i3 ();
  bus_if #(16) i4 ();
  bus_if #(16) i5 ();
  bus_if #(16) i6 ();
  bus_if #(16) i7 ();
  bus_if #(16) i8 ();
  bus_if #(16) i9 ();
  bus_if #(16) i10 ();
  bus_if #(16) i11 ();

  function void mod_function(virtual bus_if #(16) v, int x); v.put(x);   endfunction
  task          mod_task(virtual bus_if #(16) v, int x);     v.put_t(x); endtask
  function int  plain_v(int v); return v + 1; endfunction
  function void two_formals(virtual bus_if #(16) v, virtual bus_if #(16) w, int x);
    v.put(x);
    w.put(x + 1);
  endfunction

  // one check per row: the value of the instance's variable and the number of calls it ran
  task automatic check(string what, virtual bus_if #(16) i, int want_data, int want_calls);
    if (i.get() == want_data && i.calls() == want_calls)
      $display("PASS %s: data %0d, %0d call(s)", what, i.get(), i.calls());
    else
      $display("FAIL %s: data %0d, %0d call(s); expected data %0d, %0d call(s)", what, i.get(), i.calls(), want_data, want_calls);
  endtask

  user_t u;
  virtual bus_if #(16) h0, h1, h2, h3, h4, h5, h6, h7, h8, h9, h10, h11;
  initial begin
    u = new;
    h0 = i0; h1 = i1; h2 = i2; h3 = i3; h4 = i4; h5 = i5;
    h6 = i6; h7 = i7; h8 = i8; h9 = i9; h10 = i10; h11 = i11;
    u.by_function(h0, 5);       check("class_function", i0, 5, 1);
    u.by_task(h1, 6);           check("class_task", i1, 6, 1);
    u.by_typedef(h2, 7);        check("class_typedef", i2, 7, 1);
    user_t::by_static(h3, 8);   check("class_static", i3, 8, 1);
    u.by_distinct(h4, 9);       check("class_distinct", i4, 9, 1);
    u.twice(h5, 10);            check("class_twice", i5, 11, 2);
    u.passed_on(h6, 20);        check("class_passed_on", i6, 22, 1);
    u.by_local(h7, 30);         check("class_local", i7, 30, 1);
    mod_function(h8, 40);       check("module_function", i8, 40, 1);
    mod_task(h9, 50);           check("module_task", i9, 50, 1);
    two_formals(h10, h11, 60);  check("module_two_v", i10, 60, 1); check("module_two_w", i11, 61, 1);
    if (plain_v(5) == 6) $display("PASS plain_v: %0d", plain_v(5));
    else                 $display("FAIL plain_v: %0d, expected 6", plain_v(5));
    $finish;
  end
endmodule
"#;

const DESIGN_READ: &str = r#"
interface bus_if;
  logic [15:0] data;
  function int  peek(int v);     return v;     endfunction
  function int  peek_add(int v); return v + 0; endfunction
  function void put(int v);      data = v;     endfunction
  function int  get();           return data;  endfunction
endinterface

module top;
  bus_if a ();
  task automatic check(string what, int got, int expected);
    if (got == expected) $display("PASS %s = %0d", what, got);
    else                 $display("FAIL %s = %0d, expected %0d", what, got, expected);
  endtask
  function void go(virtual bus_if v);
    check("return v",     v.peek(5),     5);
    check("return v + 0", v.peek_add(5), 5);
    v.put(9);
    check("data = v, then return data", v.get(), 9);
  endfunction
  initial begin
    go(a);
    $finish;
  end
endmodule
"#;

const DESIGN_NEIGHBOURS: &str = r#"
interface bus_if;
  logic [15:0] data;
  int v = 77;
  function void put(int v); data = v; endfunction
  function int  get(); return data; endfunction
  function int  peek(int v); return v; endfunction
  function void put_local(int x); int v; v = x; data = v; endfunction
  function void put_block(int x); begin int v; v = x; data = v; end endfunction
  function int  get_member(); return v; endfunction
  task automatic put_t_local(int x); int v; v = x; data = v; endtask
  task automatic put_out(int x, output int v); v = x + 1; data = v; endtask
  function automatic void put_ref(ref int v); data = v; endfunction
  function string sput(string v); return v; endfunction
  function real rput(real v); return v; endfunction
  function automatic int rec(int v, int n); if (n == 0) return v; return rec(v + 1, n - 1); endfunction
  modport mp(import put, import get, import peek);
endinterface
package pk;
  int v = 66;
  function int read_pkg_v(); return v; endfunction
endpackage
class holder;
  int v = 33;
  function int getv(); return v; endfunction
  function int ident(int v); return v; endfunction
  function int local_v(int x); int v; v = x; return v; endfunction
  static function int twice(int v); return 2 * v; endfunction
endclass
class item;
  rand int x;
endclass
module user(bus_if v);
  function int via_port(int x); v.put(x); return v.get(); endfunction
endmodule
module top;
  bus_if i0 (); bus_if i1 (); bus_if i2 (); bus_if i3 (); bus_if i4 (); bus_if i5 ();
  bus_if i6 (); bus_if i7 (); bus_if i8 (); bus_if i9 (); bus_if i10 (); bus_if i11 ();
  bus_if i12 (); bus_if i13 ();
  user u0 (.v(i12));
  int v = 55;
  holder hh = new;
  function int plain_v(int v); return v; endfunction
  function int read_mod_v(); return v; endfunction
  task automatic check(string what, int got, int want);
    if (got == want) $display("PASS %s = %0d", what, got);
    else             $display("FAIL %s = %0d, expected %0d", what, got, want);
  endtask
  function void f_local(virtual bus_if v, int x);       v.put_local(x); endfunction
  function void f_block(virtual bus_if v, int x);       v.put_block(x); endfunction
  function int  f_member(virtual bus_if v);             return v.get_member(); endfunction
  task automatic t_local(virtual bus_if v, int x);      v.put_t_local(x); endtask
  task automatic t_out(virtual bus_if v, int x, output int o); v.put_out(x, o); endtask
  function automatic void f_ref(virtual bus_if v, int x); int r; r = x; v.put_ref(r); endfunction
  function int  f_peek_mp(virtual bus_if.mp v, int x);  return v.peek(x); endfunction
  function void f_put_mp(virtual bus_if.mp v, int x);   v.put(x); endfunction
  function int  f_plain(virtual bus_if v, int x);       return plain_v(x); endfunction
  function int  f_modv(virtual bus_if v);               return read_mod_v(); endfunction
  function int  f_pkgv(virtual bus_if v);               return pk::read_pkg_v(); endfunction
  function int  f_cls(virtual bus_if v);                return hh.getv(); endfunction
  function int  f_cls_ident(virtual bus_if v, int x);   return hh.ident(x); endfunction
  function int  f_cls_local(virtual bus_if v, int x);   return hh.local_v(x); endfunction
  function int  f_static(virtual bus_if v, int x);      return holder::twice(x); endfunction
  function string f_s(virtual bus_if v, string x);      return v.sput(x); endfunction
  function real   f_r(virtual bus_if v, real x);        return v.rput(x); endfunction
  function int  f_rec(virtual bus_if v, int x);         return v.rec(x, 3); endfunction
  function automatic int f_rand(virtual bus_if v);
    item it = new;
    void'(it.randomize() with { x == v.get(); });
    return it.x;
  endfunction
  task automatic t_fork(virtual bus_if v, int x);
    fork
      v.put(x);
    join_none
    #1;
  endtask
  initial begin
    int o;
    f_local(i0, 5);           check("callee local v", i0.get(), 5);
    f_block(i1, 6);           check("callee block-local v", i1.get(), 6);
    check("interface member v", f_member(i2), 77);
    t_local(i3, 8);           check("task local v", i3.get(), 8);
    t_out(i4, 11, o);         check("task output formal v", o, 12);
                              check("task output formal v, data", i4.get(), 12);
    f_ref(i5, 10);            check("ref formal v", i5.get(), 10);
    check("modport import peek", f_peek_mp(i6, 11), 11);
    f_put_mp(i7, 12);         check("modport import put", i7.get(), 12);
    check("module function formal v", f_plain(i8, 13), 13);
    check("module variable v", f_modv(i8), 55);
    check("package variable v", f_pkgv(i8), 66);
    check("class property v", f_cls(i8), 33);
    check("class method formal v", f_cls_ident(i8, 14), 14);
    check("class method local v", f_cls_local(i8, 15), 15);
    check("static class method formal v", f_static(i8, 9), 18);
    if (f_s(i9, "abc") == "abc") $display("PASS string formal v"); else $display("FAIL string formal v");
    if (f_r(i9, 2.5) == 2.5) $display("PASS real formal v"); else $display("FAIL real formal v");
    check("recursive interface function formal v", f_rec(i10, 20), 23);
    i11.put(3);
    check("randomize with the caller's own formal v", f_rand(i11), 3);
    check("interface port v", u0.via_port(7), 7);
    t_fork(i13, 30);          check("fork in a task with formal v", i13.get(), 30);
    $finish;
  end
endmodule
"#;

const EXPECTED_ROWS: &[&str] = &[
    "PASS class_function: data 5, 1 call(s)",
    "PASS class_task: data 6, 1 call(s)",
    "PASS class_typedef: data 7, 1 call(s)",
    "PASS class_static: data 8, 1 call(s)",
    "PASS class_distinct: data 9, 1 call(s)",
    "PASS class_twice: data 11, 2 call(s)",
    "PASS class_passed_on: data 22, 1 call(s)",
    "PASS class_local: data 30, 1 call(s)",
    "PASS module_function: data 40, 1 call(s)",
    "PASS module_task: data 50, 1 call(s)",
    "PASS module_two_v: data 60, 1 call(s)",
    "PASS module_two_w: data 61, 1 call(s)",
    "PASS plain_v: 6",
];

const EXPECTED_READ: &[&str] = &[
    "PASS return v = 5",
    "PASS return v + 0 = 5",
    "PASS data = v, then return data = 9",
];

const EXPECTED_NEIGHBOURS: &[&str] = &[
    "PASS callee local v = 5",
    "PASS callee block-local v = 6",
    "PASS interface member v = 77",
    "PASS task local v = 8",
    "PASS task output formal v = 12",
    "PASS task output formal v, data = 12",
    "PASS ref formal v = 10",
    "PASS modport import peek = 11",
    "PASS modport import put = 12",
    "PASS module function formal v = 13",
    "PASS module variable v = 55",
    "PASS package variable v = 66",
    "PASS class property v = 33",
    "PASS class method formal v = 14",
    "PASS class method local v = 15",
    "PASS static class method formal v = 18",
    "PASS string formal v",
    "PASS real formal v",
    "PASS recursive interface function formal v = 23",
    "PASS randomize with the caller's own formal v = 3",
    "PASS interface port v = 7",
    "PASS fork in a task with formal v = 30",
];

fn run(tag: &str, design: &str, envs: &[(&str, &str)]) -> String {
    let dir = std::env::temp_dir().join(format!(
        "xezim_vif_name_clash_{}_{}",
        std::process::id(),
        tag
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.sv");
    std::fs::write(&path, design).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.args([
        "--simulate",
        "-s",
        "top",
        path.to_str().unwrap(),
        "--no-cache",
    ])
    .env_remove("XEZIM_METHOD_CACHE");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    text
}

fn check(tag: &str, design: &str, expected: &[&str]) {
    for (mode, envs) in [
        ("compiled", &[("XEZIM_COMPILE_METHODS", "1")][..]),
        ("interpreted", &[("XEZIM_COMPILE_METHODS", "0")][..]),
        ("tier0", &[("XEZIM_METHOD_TIER", "0")][..]),
    ] {
        let log = run(&format!("{tag}_{mode}"), design, envs);
        let got: Vec<&str> = log
            .lines()
            .filter(|l| l.starts_with("PASS") || l.starts_with("FAIL"))
            .collect();
        assert_eq!(got, expected, "{tag}, {mode}:\n{log}");
    }
}

#[test]
fn callee_formal_named_like_a_caller_vif_formal_is_its_own() {
    check("rows", DESIGN_ROWS, EXPECTED_ROWS);
}

#[test]
fn callee_formal_read_whole_is_the_value_passed() {
    check("read", DESIGN_READ, EXPECTED_READ);
}

#[test]
fn callee_locals_and_scope_variables_named_like_a_caller_vif_formal() {
    check("neighbours", DESIGN_NEIGHBOURS, EXPECTED_NEIGHBOURS);
}
