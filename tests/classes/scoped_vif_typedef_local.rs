//! §6.18, §8.23, §25.9: a local declared with a virtual-interface typedef
//! reached through a class scope (`cfg_type::vif_t v = cfg.get_vif(i);`,
//! A.2.2.1 `[ class_scope ] type_identifier`) is a virtual-interface
//! variable: it starts null and its initializer binds the interface
//! instance. The declaration was taken for a virtual interface only through
//! bare typedef names, so the local held no binding and a call through it
//! ran nothing; with the leaf declared by two classes, assignments, formals
//! and class properties typed through the scope missed it too. In a compiled
//! method a virtual-interface local also started x (`v == null` read 0).
//! Every expected line was checked on the reference simulator. Run compiled
//! and interpreted (#1252).

use std::process::Command;

/// The design of #1252.
const ISSUE: &str = r#"
interface bus_if #(int W = 8);
  logic [W-1:0] data;
  function void put(int v); data = v; endfunction
  function int  get();      return data; endfunction
endinterface

class cfg_c #(int W = 8);
  typedef virtual bus_if #(W) bus_vif_t;     // the scoped typedef
  typedef int                 count_t;
  bus_vif_t slot[2];
  function bus_vif_t get_vif(int i = 0);
    return slot[i];
  endfunction
endclass

module top;
  bus_if #(16) a ();
  bus_if #(16) b ();
  typedef cfg_c #(16) cfg_type;              // a typedef of a specialization

  // the user of the configuration, as a testbench component would be
  class user_t;
    cfg_type cfg;
    function new(cfg_type c); cfg = c; endfunction
    function void bind_each(int base);       // a local with an initializer, in a loop
      for (int i = 0; i < 2; i++) begin
        cfg_type::bus_vif_t v = cfg.get_vif(i);
        v.put(base + i);
      end
    endfunction
    function void bind_one(int i, int x);    // a local scoped by the class itself
      cfg_c#(16)::bus_vif_t w = cfg.get_vif(i);
      w.put(x);
    endfunction
    function int count_local();              // a scoped typedef that is not a vif
      cfg_type::count_t k = 7;
      return k;
    endfunction
  endclass

  task automatic check(string what, int got, int expected);
    if (got == expected) $display("PASS %s = %0d", what, got);
    else                 $display("FAIL %s = %0d, expected %0d", what, got, expected);
  endtask

  initial begin
    cfg_type cfg = new;
    user_t   u;
    cfg.slot[0] = a;
    cfg.slot[1] = b;
    u = new(cfg);
    u.bind_each(10);
    check("local in a loop, first instance",  a.get(), 10);
    check("local in a loop, second instance", b.get(), 11);
    u.bind_one(1, 21);
    check("local scoped by the class specialization", b.get(), 21);
    check("scoped integer typedef stays an integer",  u.count_local(), 7);
    $finish;
  end
endmodule
"#;

/// The leaf declared by several classes (one of them as an `int`): a task
/// local, a re-bind, a scoped property, a static initializer, a class, a
/// base class and a package class as the scope, a formal, a block local.
const SHAPES: &str = r#"
interface bus_if #(int W = 8);
  logic [W-1:0] data;
  function void put(int v); data = v; endfunction
  function int  get();      return data; endfunction
endinterface

package pk;
  class pcfg #(int W = 8);
    typedef virtual bus_if #(W) vif_t;
    vif_t s;
  endclass
  typedef pcfg #(16) pcfg16;
endpackage

class cfg_c #(int W = 8);
  typedef virtual bus_if #(W) bus_vif_t;
  bus_vif_t slot[2];
  static bus_vif_t sv;
  function bus_vif_t get_vif(int i = 0); return slot[i]; endfunction
endclass

class other_c;                     // a second class declaring the same leaf
  typedef virtual bus_if #(16) bus_vif_t;
  typedef int vif_t;
endclass

class plain_c;
  typedef virtual bus_if #(16) vif_t;
endclass

class base_c;
  typedef virtual bus_if #(16) bvif_t;
endclass
class derived_c extends base_c;
endclass

module top;
  bus_if #(16) a ();
  bus_if #(16) b ();
  typedef cfg_c #(16) cfg_type;

  class user_t;
    cfg_type cfg;
    cfg_type::bus_vif_t pv;        // property typed through the scope
    function new(cfg_type c); cfg = c; endfunction
    function int null_local();
      cfg_type::bus_vif_t n;        // no initializer: null
      return n == null;
    endfunction
    task t_bind(int x);             // a task local
      cfg_type::bus_vif_t v = cfg.get_vif(1);
      v.put(x);
    endtask
    function void rebind(int x);
      cfg_type::bus_vif_t v = cfg.get_vif(0);
      v = cfg.get_vif(1);
      v.put(x);
    endfunction
    function void via_prop(int x);
      pv = cfg.get_vif(0);
      pv.put(x);
    endfunction
    function void static_init(int x);
      cfg_type::bus_vif_t v = cfg_type::sv;
      v.put(x);
    endfunction
    function void plain_scope(int x);
      plain_c::vif_t v = cfg.get_vif(0);
      v.put(x);
    endfunction
    function void base_scope(int x);
      derived_c::bvif_t v = cfg.get_vif(1);
      v.put(x);
    endfunction
    function void pkg_scope(int x);
      pk::pcfg16::vif_t v = cfg.get_vif(0);
      v.put(x);
    endfunction
    function int other_int();
      other_c::vif_t k = 9;          // same leaf elsewhere, an int
      return k;
    endfunction
    function void formal_vif(cfg_type::bus_vif_t f, int x);
      f.put(x);
    endfunction
  endclass

  task automatic check(string what, int got, int expected);
    if (got == expected) $display("PASS %s = %0d", what, got);
    else                 $display("FAIL %s = %0d, expected %0d", what, got, expected);
  endtask

  initial begin
    cfg_type cfg = new;
    user_t   u;
    cfg.slot[0] = a;
    cfg.slot[1] = b;
    cfg_type::sv = b;
    u = new(cfg);
    check("null local", u.null_local(), 1);
    u.t_bind(31);          check("task local", b.get(), 31);
    u.rebind(32);          check("rebind", b.get(), 32);
    u.via_prop(33);        check("scoped property", a.get(), 33);
    u.static_init(34);     check("static init", b.get(), 34);
    u.plain_scope(35);     check("plain class scope", a.get(), 35);
    u.base_scope(36);      check("base class typedef via derived", b.get(), 36);
    u.pkg_scope(37);       check("package class typedef", a.get(), 37);
    check("other int leaf", u.other_int(), 9);
    u.formal_vif(cfg.get_vif(1), 38); check("formal", b.get(), 38);
    begin
      automatic cfg_type::bus_vif_t m = cfg.get_vif(0);   // procedural local
      m.put(39);
      check("initial-block local", a.get(), 39);
    end
    $finish;
  end
endmodule
"#;

/// Virtual-interface locals without an initializer start null, spelled
/// through a module typedef, a class typedef, `virtual` and a class scope.
const NULL_LOCALS: &str = r#"
interface bus_if #(int W = 8);
  logic [W-1:0] data;
  function void put(int v); data = v; endfunction
  function int  get();      return data; endfunction
endinterface
typedef virtual bus_if #(16) mvif_t;
class cfg_c;
  typedef virtual bus_if #(16) cvif_t;
  mvif_t slot[2];
  function mvif_t get_vif(int i = 0); return slot[i]; endfunction
  function int null_mod();  mvif_t n; return n == null; endfunction
  function int null_cls();  cvif_t n; return n == null; endfunction
  function int null_virt(); virtual bus_if #(16) n; return n == null; endfunction
  function int null_scoped(); cfg_c::cvif_t n; return n == null; endfunction
endclass
module top;
  bus_if #(16) a ();
  initial begin
    cfg_c c = new;
    $display("T|null_mod=%0d null_cls=%0d null_virt=%0d null_scoped=%0d", c.null_mod(), c.null_cls(), c.null_virt(), c.null_scoped());
  end
endmodule
"#;

fn run(design: &str, compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_scoped_vif_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, design).expect("write design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_COMPILE_METHODS", compile)
        .env("XEZIM_METHOD_TIER", "0")
        .env_remove("XEZIM_METHOD_CACHE")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    text
}

fn check(design: &str, passes: usize) {
    for compile in ["1", "0"] {
        let log = run(design, compile);
        let pass = log.lines().filter(|l| l.starts_with("PASS ")).count();
        assert!(
            pass == passes && !log.contains("FAIL"),
            "XEZIM_COMPILE_METHODS={compile}: {pass} of {passes} checks passed:\n{log}"
        );
    }
}

#[test]
fn scoped_vif_typedef_local_binds() {
    check(ISSUE, 4);
}

#[test]
fn scoped_vif_typedef_shapes() {
    check(SHAPES, 11);
}

#[test]
fn vif_local_starts_null() {
    for compile in ["1", "0"] {
        let log = run(NULL_LOCALS, compile);
        assert!(
            log.contains("T|null_mod=1 null_cls=1 null_virt=1 null_scoped=1"),
            "XEZIM_COMPILE_METHODS={compile}:\n{log}"
        );
    }
}
