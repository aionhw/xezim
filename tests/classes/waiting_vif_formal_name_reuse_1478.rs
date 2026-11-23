//! §6.21, §11.4.5, §25.9 (#1478): virtual-interface formals and locals belong
//! to their own subroutine frame (`VifAliasFrame`), not a flat global table.
//! While a task waits on `#delay`, `@(event)`, or `wait(...)` with a virtual-
//! interface formal (or parameterized `T` local bound to a virtual interface),
//! unrelated scopes, overlapping/killed tasks, loop variables (`for`/`foreach`),
//! class/module properties, `this`-property shadowers, and `randomize()` solves
//! reuse the same bare identifier without comparing as that interface.
//! All shapes were verified byte-for-byte against the reference simulator
//! across compiled, interpreted (`XEZIM_COMPILE_METHODS=0`), tier-0
//! (`XEZIM_METHOD_TIER=0`), and sequential (`XEZIM_NO_PARALLEL=1`) runs.

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static DIR_SEQ: AtomicUsize = AtomicUsize::new(0);

fn run_mode(
    design: &str,
    compile: &str,
    tier: Option<&str>,
    no_parallel: bool,
) -> String {
    let seq = DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "xezim_vif_formal_reuse_1478_{}_{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("design.sv");
    std::fs::write(&path, design).expect("write design");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.args([
        "--simulate",
        "-s",
        "top",
        path.to_str().unwrap(),
        "--no-cache",
    ])
    .env("XEZIM_COMPILE_METHODS", compile)
    .env_remove("XEZIM_METHOD_CACHE");
    if let Some(t) = tier {
        cmd.env("XEZIM_METHOD_TIER", t);
    } else {
        cmd.env_remove("XEZIM_METHOD_TIER");
    }
    if no_parallel {
        cmd.env("XEZIM_NO_PARALLEL", "1");
    } else {
        cmd.env_remove("XEZIM_NO_PARALLEL");
    }
    let out = cmd.output().expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(
        out.status.success(),
        "compile={compile} tier={tier:?} no_parallel={no_parallel} failed:\n{text}"
    );
    text
}

fn check_all_modes(design: &str, expected_passes: usize) {
    let modes = [
        ("1", None, false),
        ("0", None, false),
        ("0", Some("0"), false),
        ("1", None, true),
    ];
    for (compile, tier, no_parallel) in modes {
        let log = run_mode(design, compile, tier, no_parallel);
        let fails: Vec<&str> = log
            .lines()
            .filter(|l| l.starts_with("FAIL "))
            .collect();
        let passes = log.lines().filter(|l| l.starts_with("PASS ")).count();
        assert!(
            fails.is_empty() && passes == expected_passes,
            "compile={compile} tier={tier:?} no_parallel={no_parallel}: passes={passes} expected={expected_passes} fails={fails:?}\n{log}"
        );
    }
}

#[test]
fn waiting_vif_formal_does_not_alias_other_scopes() {
    let core = r#"
interface bus_if;
  int id = 0;
endinterface

class prop_t;
  int i = 0;
  function int is_zero();
    return (i == 0);
  endfunction
endclass

class user_t;
  function int foreach_zero_count();
    int q[3];
    int c = 0;
    foreach (q[i]) if (i == 0) c++;
    return c;
  endfunction
  function int foreach_nonzero_count();
    int q[3];
    int c = 0;
    foreach (q[i]) if (i != 0) c++;
    return c;
  endfunction
  function int local_zero();
    int i = 0;
    return (i == 0);
  endfunction
  function int local_not_zero();
    int i = 0;
    return (i != 0);
  endfunction
  function int local_seven();
    int i = 7;
    return (i == 7);
  endfunction
  function int for_variable_zero();
    int c = 0;
    for (int i = 0; i < 3; i++) if (i == 0) c++;
    return c;
  endfunction
  function int vector_local_zero();
    logic [3:0] i = 4'd0;
    return (i == 0);
  endfunction
  function int event_name_local();
    int n = 0;
    return (n == 0);
  endfunction
  function int level_name_for_variable();
    int c = 0;
    for (int m = 3; m > 0; m--) if (m == 3) c++;
    return c;
  endfunction
  task automatic task_local_zero(output int r);
    int i = 0;
    r = (i == 0);
  endtask
endclass

module top;
  bus_if b0();
  bus_if b1();
  event go;
  bit open;
  int i = 0;
  int n = 0;
  int m = 0;

  task automatic check(string what, int got, int expected);
    if (got == expected) $display("PASS %s", what);
    else                 $display("FAIL %s: got %0d, expected %0d", what, got, expected);
  endtask

  task automatic wait_delay(virtual bus_if i);
    #5;
    check("delay formal: its interface is still bound (id)", i.id, 5);
    check("delay formal: not null", (i != null), 1);
  endtask
  task automatic wait_event(virtual bus_if n);
    @(go);
    check("event formal: its interface is still bound (id)", n.id, 6);
    check("event formal: not null", (n != null), 1);
  endtask
  task automatic wait_level(virtual bus_if m);
    wait (open);
    check("level formal: its interface is still bound (id)", m.id, 5);
    check("level formal: not null", (m != null), 1);
  endtask
  initial wait_delay(b0);
  initial wait_event(b1);
  initial wait_level(b0);
  initial begin #10 -> go; #5 open = 1; end

  function automatic int module_foreach_zero();
    int q[2];
    int c = 0;
    foreach (q[i]) if (i == 0) c++;
    return c;
  endfunction

  task automatic module_task_local(output int r);
    int i = 0;
    r = (i == 0);
  endtask

  initial begin
    user_t u;
    prop_t pr;
    int r;
    virtual bus_if vh;
    virtual bus_if vu;
    b0.id = 5;
    b1.id = 6;
    u = new;
    pr = new;
    vh = b0;
    #1;
    check("foreach index == 0 (once in 3)",       u.foreach_zero_count(), 1);
    check("foreach index != 0 (twice in 3)",      u.foreach_nonzero_count(), 2);
    check("int local i == 0",                     u.local_zero(), 1);
    check("int local i != 0 is false",            u.local_not_zero(), 0);
    check("int local i == 7 when i is 7",         u.local_seven(), 1);
    check("for variable i == 0 (once in 3)",      u.for_variable_zero(), 1);
    check("4-bit local i == 0",                   u.vector_local_zero(), 1);
    check("local n (event formal's name) == 0",   u.event_name_local(), 1);
    check("for variable m (level formal's name)", u.level_name_for_variable(), 1);
    check("class property i == 0",                pr.is_zero(), 1);
    u.task_local_zero(r);
    check("task local i == 0 (in a class)",       r, 1);
    module_task_local(r);
    check("task local i == 0 (in the module)",    r, 1);
    check("foreach index == 0 (module function)", module_foreach_zero(), 1);
    check("module variable i == 0",               (i == 0), 1);
    check("module variable i != 0 is false",      (i != 0), 0);
    check("module variable n == 0",               (n == 0), 1);
    n = 7;
    check("module variable n == 7 when n is 7",   (n == 7), 1);
    check("module variable m != 0 is false",      (m != 0), 0);
    check("virtual interface variable == null is false", (vh == null), 0);
    check("virtual interface variable != null",          (vh != null), 1);
    check("unbound virtual interface variable == null",  (vu == null), 1);
    check("virtual interface variable reads its id",     vh.id, 5);
    #19;
    check("after the waits: module variable i == 0", (i == 0), 1);
    $finish;
  end
endmodule
"#;
    check_all_modes(core, 29);

    let scopes = r#"
interface bus_if;
  int id = 0;
endinterface

package pk;
  int i = 0;
  class pk_class;
    function int is_zero();
      return (i == 0);
    endfunction
  endclass
  function int pk_is_zero();
    return (i == 0);
  endfunction
endpackage

module top;
  bus_if b0();
  int i = 0;

  class mod_class;
    function int is_zero();
      return (i == 0);
    endfunction
    function int is_not_zero();
      return (i != 0);
    endfunction
  endclass

  task automatic check(string what, int got, int expected);
    if (got == expected) $display("PASS %s", what);
    else                 $display("FAIL %s: got %0d, expected %0d", what, got, expected);
  endtask

  task automatic wait_delay(virtual bus_if i);
    #5;
  endtask
  initial wait_delay(b0);

  initial begin
    mod_class mc;
    pk::pk_class pc;
    mc = new;
    pc = new;
    #1;
    check("module variable i == 0 read in a class method",  mc.is_zero(), 1);
    check("module variable i != 0 read in a class method", mc.is_not_zero(), 0);
    check("package variable i == 0 read in a class method", pc.is_zero(), 1);
    check("package variable i == 0 read in a package function", pk::pk_is_zero(), 1);
    check("package variable i == 0 read in the module",     (pk::i == 0), 1);
    $finish;
  end
endmodule
"#;
    check_all_modes(scopes, 5);
}

#[test]
fn overlapping_killed_parameterized_and_module_vif_cousins() {
    let src = r#"
interface bus_if;
  int id = 0;
endinterface

class vbox #(type T = int);
  T val;
  function void make_local(T a);
    T leaked_loc;
    leaked_loc = a;
  endfunction
  function int call_helper(T i, int dummy);
    return top.read_mod_i_eq0();
  endfunction
  task automatic wait_local(T a);
    T waited_loc;
    waited_loc = a;
    #5;
  endtask
endclass

class ibox #(type T = int);
  T val;
  function void set(T v);
    val = v;
  endfunction
  function int val_is_zero();
    return (val == 0);
  endfunction
endclass

module top;
  bus_if b0();
  bus_if b1();
  int i = 0;
  int leaked_loc = 0;
  int waited_loc = 0;
  int plain_t_var = 0;
  virtual bus_if mv;

  function automatic int read_mod_i_eq0();
    return (i == 0);
  endfunction

  task automatic check(string what, int got, int expected);
    if (got == expected) $display("PASS %s", what);
    else                 $display("FAIL %s: got %0d, expected %0d", what, got, expected);
  endtask

  task automatic overlap_t(virtual bus_if i, int d, int want_id, string tag);
    #(d);
    check(tag, i.id, want_id);
  endtask

  task automatic wait_vif_p(virtual bus_if plain_t_var);
    #4;
    check("cousin6 vif task id", plain_t_var.id, 11);
  endtask
  task automatic wait_int_p(int plain_t_var);
    #6;
    check("cousin6 int task val", (plain_t_var == 0), 1);
  endtask

  task automatic wait_mv(virtual bus_if mv);
    #5;
    check("cousin9 task mv.id", mv.id, 11);
  endtask

  task automatic task_with_shadow_loops(virtual bus_if i);
    int q[3];
    int c1 = 0;
    int c2 = 0;
    #2;
    foreach (q[i]) if (i == 0) c1++;
    for (int i = 0; i < 3; i++) if (i == 0) c2++;
    check("cousin4 foreach shadows own vif formal", c1, 1);
    check("cousin4 for shadows own vif formal", c2, 1);
    check("cousin4 own vif formal intact after loops", i.id, 11);
  endtask

  int killed_i = 0;
  task automatic wait_forever(virtual bus_if killed_i);
    #100;
  endtask

  vbox #(virtual bus_if) bv_wait;
  initial begin
    b0.id = 11;
    b1.id = 22;
    mv = b1;
    bv_wait = new;
    fork
      overlap_t(b0, 5, 11, "cousin1 overlap task1 id");
      begin #2; overlap_t(b1, 5, 22, "cousin1 overlap task2 id"); end
      wait_vif_p(b0);
      begin #1; wait_int_p(0); end
      wait_mv(b0);
      task_with_shadow_loops(b0);
      bv_wait.wait_local(b0);
    join_none
  end

  initial begin
    vbox #(virtual bus_if) bv;
    ibox #(int) bi;
    virtual bus_if vcopy;
    process p;
    bv = new;
    bi = new;

    fork
      begin
        p = process::self();
        wait_forever(b0);
      end
    join_none
    #1;
    p.kill();
    #0;
    check("cousin7 killed task does not leak killed_i", (killed_i == 0), 1);

    bv.make_local(b0);
    check("cousin2 non-waiting method local T leaked_loc == 0 after return", (leaked_loc == 0), 1);
    check("cousin2w waiting param-task local T waited_loc == 0 while waiting", (waited_loc == 0), 1);

    check("cousin2b sync param-method T i does not shadow module i in callee", bv.call_helper(b0, 0), 1);

    bi.set(i);
    check("cousin3 box#(int)::set(i) val == 0 while task waits", bi.val_is_zero(), 1);

    check("cousin9 module mv == b1 while task waits on mv=b0", (mv == b1), 1);
    check("cousin9 module mv == b0 is false", (mv == b0), 0);
    vcopy = mv;
    check("cousin9 vcopy = mv gets b1 not b0", vcopy.id, 22);

    #10;
    check("cousin1 after both overlapping tasks finish: i == 0", (i == 0), 1);
    check("cousin6 after vif+int overlapping tasks finish: plain_t_var == 0", (plain_t_var == 0), 1);
    check("cousin2w after waiting param-task finishes: waited_loc == 0", (waited_loc == 0), 1);
    $finish;
  end
endmodule
"#;
    check_all_modes(src, 19);
}

#[test]
fn local_formal_and_loop_shadow_this_vif_property() {
    let src = r#"
interface bus_if;
  int id = 0;
endinterface

class with_vif_prop;
  virtual bus_if i;
  function int local_shadows_vif_prop();
    int i = 0;
    return (i == 0);
  endfunction
  function int formal_shadows_vif_prop(int i);
    return (i == 0);
  endfunction
  function int foreach_shadows_vif_prop();
    int q[3];
    int c = 0;
    foreach (q[i]) if (i == 0) c++;
    return c;
  endfunction
  function int for_shadows_vif_prop();
    int c = 0;
    for (int i = 0; i < 3; i++) if (i == 0) c++;
    return c;
  endfunction
  function int prop_still_not_null();
    return (i != null);
  endfunction
endclass

class with_param_prop #(type T = int);
  T i;
  function void set_prop(T v);
    i = v;
  endfunction
  function int local_shadows_param_prop();
    int i = 0;
    return (i == 0);
  endfunction
  function int foreach_shadows_param_prop();
    int q[3];
    int c = 0;
    foreach (q[i]) if (i == 0) c++;
    return c;
  endfunction
endclass

module top;
  bus_if b0();
  task automatic check(string what, int got, int expected);
    if (got == expected) $display("PASS %s", what);
    else                 $display("FAIL %s: got %0d, expected %0d", what, got, expected);
  endtask
  initial begin
    with_vif_prop w = new;
    with_param_prop #(virtual bus_if) wp = new;
    w.i = b0;
    wp.set_prop(b0);
    check("c10 local shadows vif prop", w.local_shadows_vif_prop(), 1);
    check("c10 formal shadows vif prop", w.formal_shadows_vif_prop(0), 1);
    check("c10 foreach shadows vif prop", w.foreach_shadows_vif_prop(), 1);
    check("c10 for shadows vif prop", w.for_shadows_vif_prop(), 1);
    check("c10 vif prop itself not null", w.prop_still_not_null(), 1);
    check("c10 local shadows param vif prop", wp.local_shadows_param_prop(), 1);
    check("c10 foreach shadows param vif prop", wp.foreach_shadows_param_prop(), 1);
    $finish;
  end
endmodule
"#;
    check_all_modes(src, 7);
}

#[test]
fn randomize_and_blocking_foreach_cousins() {
    let src = r#"
interface bus_if;
  int id = 7;
  function int get(); return id; endfunction
endinterface

class item_with_v;
  rand int v;
  rand int x;
  constraint c1 { v == 0; }
  constraint c2 { (v == 0) -> x == 42; }
endclass

module top;
  bus_if b0();
  task automatic check(string what, int got, int expected);
    if (got == expected) $display("PASS %s", what);
    else                 $display("FAIL %s: got %0d, expected %0d", what, got, expected);
  endtask

  function automatic int test_rand_prop_v(virtual bus_if v);
    item_with_v it = new;
    if (!it.randomize()) return -1;
    return it.x;
  endfunction

  task automatic test_blocking_foreach(virtual bus_if i);
    int q[3];
    int hits = 0;
    foreach (q[i]) begin
      #1;
      if (i == 0) hits++;
    end
    check("c4b blocking foreach shadows own vif formal", hits, 1);
    check("c4b own vif formal intact after blocking foreach", i.id, 7);
  endtask

  initial begin
    check("c11 randomize object constraint with prop v == 0", test_rand_prop_v(b0), 42);
    test_blocking_foreach(b0);
    $finish;
  end
endmodule
"#;
    check_all_modes(src, 3);
}

#[test]
fn parameterized_and_typedef_vif_local_initializers() {
    let src = r#"
interface bus_if;
  int id;
endinterface

class c13_param #(type T = int);
  typedef virtual bus_if vif_t;
  virtual bus_if v;
  int calls;

  function automatic virtual bus_if get_vif(virtual bus_if a);
    calls++;
    return a;
  endfunction

  task automatic run_tp(T a);
    T v = a;
    #1;
    if (v != null && this.v != null && v.id == 11 && this.v.id == 22)
      $display("PASS c13 T=vif local v=a shadows this.v");
    else
      $display("FAIL c13 T=vif local v=a");
  endtask

  task automatic run_once(virtual bus_if a);
    virtual bus_if v = get_vif(a);
    #1;
    if (calls == 1 && v != null && this.v != null && v.id == 11 && this.v.id == 22)
      $display("PASS c13 vif local initializer evaluated once");
    else
      $display("FAIL c13 vif local initializer evaluated once: calls=%0d", calls);
  endtask

  task automatic run_uninit();
    virtual bus_if v;
    #1;
    if (v == null && this.v != null && this.v.id == 22)
      $display("PASS c13 uninitialized vif local null shadows this.v");
    else
      $display("FAIL c13 uninitialized vif local null shadows this.v");
  endtask

  task automatic run_td(vif_t a);
    vif_t v = a;
    #1;
    if (v != null && this.v != null && v.id == 11 && this.v.id == 22)
      $display("PASS c13 typedef vif_t local v=a shadows this.v");
    else
      $display("FAIL c13 typedef vif_t local v=a");
  endtask
endclass

class c13_int #(type T = int);
  virtual bus_if v;
  task automatic run_int();
    T v = 0;
    #1;
    if (v == 0 && this.v != null && this.v.id == 22)
      $display("PASS c13 T=int local v=0 compares 0 while this.v is vif");
    else
      $display("FAIL c13 T=int local v=0");
  endtask
endclass

module top;
  bus_if b0();
  bus_if b1();
  initial begin
    c13_param#(virtual bus_if) p = new;
    c13_int#(int) pi = new;
    b0.id = 11;
    b1.id = 22;
    p.v = b1;
    pi.v = b1;
    p.run_tp(b0);
    p.calls = 0;
    p.run_once(b0);
    p.run_uninit();
    p.run_td(b0);
    pi.run_int();
    $finish;
  end
endmodule
"#;
    check_all_modes(src, 5);
}
