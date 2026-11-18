//! §8.25, §23.10.2.2: inside a parameterized class, `typedef drv_c #(.W(W))
//! t;` names the same specialization as `typedef drv_c #(W) t;`. Read
//! through the enclosing class's specialization, the whole named argument
//! was substituted, name included (`.W(W)` became `.16(16)` in `outer_c
//! #(16)`), matched no parameter, and `t::type_id::create()` or a static call
//! through the typedef used the default specialization. A constant
//! expression over the enclosing parameters (`.D(W/4)`, or `#(W, W/4)`) was
//! dropped the same way. Every expected line was checked on the reference
//! simulator. Run compiled and interpreted (#1254).

use std::process::Command;

/// The design of #1254.
const ISSUE: &str = r#"
virtual class registry_base;
endclass

class registry #(type T = int);          // a factory, as a library would have
  static function T create();
    T obj = new;
    return obj;
  endfunction
endclass

class drv_c #(int W = 8, int D = 1);
  typedef registry #(drv_c #(W, D)) type_id;
  function int w();  return W;  endfunction
  function int d();  return D;  endfunction
  static function int sw(); return W; endfunction
endclass

class outer_c #(int W = 8);
  typedef drv_c #(.W(W))        named_t;      // named assignment
  typedef drv_c #(W)            ordered_t;    // ordered assignment
  typedef drv_c #(.D(3), .W(W)) swapped_t;    // named, out of order
  function int made_named();   named_t   d = named_t::type_id::create();   return d.w(); endfunction
  function int made_ordered(); ordered_t d = ordered_t::type_id::create(); return d.w(); endfunction
  function int made_swapped(); swapped_t d = swapped_t::type_id::create(); return d.w() * 10 + d.d(); endfunction
  function int new_named();    named_t   d = new;                          return d.w(); endfunction
  function int static_named(); return named_t::sw(); endfunction
endclass

module top;
  task automatic check(string what, int got, int expected);
    if (got == expected) $display("PASS %s = %0d", what, got);
    else                 $display("FAIL %s = %0d, expected %0d", what, got, expected);
  endtask

  typedef drv_c #(.W(16)) mod_named_t;
  initial begin
    outer_c #(16) o = new;
    outer_c #(8)  p = new;
    check("create through the named typedef",    o.made_named(), 16);
    check("create through the ordered typedef",  o.made_ordered(), 16);
    check("create through a swapped named list", o.made_swapped(), 163);
    check("new through the named typedef",       o.new_named(), 16);
    check("static call through the named typedef", o.static_named(), 16);
    check("default specialization still works",  p.made_named(), 8);
    begin
      mod_named_t m = mod_named_t::type_id::create();
      check("module-scope named typedef",        m.w(), 16);
    end
    $finish;
  end
endmodule
"#;

/// An expression of the enclosing parameter, swapped names, a type
/// parameter and a constant by name, a partial list, a property declared
/// with a named list, direct static and factory calls through a named
/// specialization, a `$cast` to it, and a named base specialization.
const SHAPES: &str = r#"
class registry #(type T = int);
  static function T create();
    T obj = new;
    return obj;
  endfunction
endclass

class drv_c #(int W = 8, int D = 1);
  typedef registry #(drv_c #(W, D)) type_id;
  function int w();  return W;  endfunction
  function int d();  return D;  endfunction
  static function int sw(); return W; endfunction
endclass

class box_c #(type T = int, int N = 2);
  T v;
  function int bits(); return $bits(v); endfunction
  function int n(); return N; endfunction
  typedef registry #(box_c #(T, N)) type_id;
endclass

class base_c #(int W = 8);
  function int bw(); return W; endfunction
endclass

class outer_c #(int W = 8, type T = byte);
  typedef drv_c #(.W(W), .D(W/4))   arith_t;      // value expression of a param
  localparam int D_OF = 5;
  typedef drv_c #(.D(W), .W(D_OF))  swap2_t;      // swapped names
  typedef box_c #(.T(T), .N(W))     tbox_t;       // type param by name
  typedef drv_c #(.W(7))            const_t;      // a constant by name
  typedef drv_c #(.D(W))            partial_t;    // partial list, W defaults
  drv_c #(.W(W)) prop;                            // a property typed by name
  function int a();  arith_t d = arith_t::type_id::create(); return d.w() * 100 + d.d(); endfunction
  function int s2(); swap2_t d = swap2_t::type_id::create(); return d.w() * 100 + d.d(); endfunction
  function int tb(); tbox_t b = tbox_t::type_id::create(); return b.bits() * 100 + b.n(); endfunction
  function int c();  const_t d = const_t::type_id::create(); return d.w(); endfunction
  function int p();  partial_t d = partial_t::type_id::create(); return d.w() * 100 + d.d(); endfunction
  function int pr(); prop = new; return prop.w(); endfunction
  function int direct(); return drv_c #(.W(W))::sw(); endfunction
  function int direct_create(); drv_c #(.W(W)) d = drv_c #(.W(W))::type_id::create(); return d.w(); endfunction
  function int castok();
    drv_c #(W) want; arith_t got = arith_t::type_id::create();
    drv_c #(.W(W), .D(W/4)) want2;
    return $cast(want2, got);
  endfunction
endclass

class derived_c #(int W = 8) extends base_c #(.W(W));
endclass

module top;
  task automatic check(string what, int got, int expected);
    if (got == expected) $display("PASS %s = %0d", what, got);
    else                 $display("FAIL %s = %0d, expected %0d", what, got, expected);
  endtask
  initial begin
    outer_c #(16, shortint) o = new;
    derived_c #(32) dc = new;
    check("value expression", o.a(), 1604);
    check("swapped names", o.s2(), 516);
    check("type param by name", o.tb(), 1616);
    check("constant by name", o.c(), 7);
    check("partial list", o.p(), 816);
    check("property typed by name", o.pr(), 16);
    check("direct static call", o.direct(), 16);
    check("direct create", o.direct_create(), 16);
    check("cast to named spec", o.castok(), 1);
    check("named base", dc.bw(), 32);
    $finish;
  end
endmodule
"#;

fn run(design: &str, compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_named_param_typedef_{}_{}",
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
fn named_param_typedef_through_enclosing_spec() {
    check(ISSUE, 7);
}

#[test]
fn named_param_typedef_shapes() {
    check(SHAPES, 10);
}
