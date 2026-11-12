//! class-perf Step 9e: VOID-return functions and CONSTRUCTORS (`new`)
//! compile as class-method bytecode, plus the two lowering surfaces they
//! need to engage: BARE member stores (`cnt = v` — resolves to class
//! scope first, like the bare read) and BARE this-bounded calls
//! (`helper(2)` — the callee name is a method of the enclosing chain;
//! lowered to `CallMethod` on the `this` register so runtime keeps full
//! dispatcher semantics: virtual dispatch, defaults, AST-fallback
//! callees).
//!
//! A void result is a PASSTHROUGH cell: width 0 (keep source), no
//! resize, no signedness stamp — the register is seeded from the frame's
//! implicit return local, mirroring the interpreter's epilogue
//! (`return_value.or(implicit)`), so a body that never writes the cell
//! returns the identical seed value. `function new` parses as Implicit
//! with no dimensions and takes the same path.
//!
//! Covers: ctor with default formals + bare int-member stores; void
//! methods with member read+store, control flow, early bare `return;`;
//! bare void-to-void calls through a while loop; local VarDecls; a
//! derived-class void method calling INHERITED helpers bare; a void
//! method called through `void'(...)` inside a compiled body. Expected
//! declines (clean AST fallback, output identical): a `$display` body,
//! a string-member store (Step 9a boundary), and `super.new` in a
//! derived ctor.
//!
//! Expectations are reference-simulator-validated (method_vm36 runs
//! TAG-identically under the reference simulator, gate OFF, and gate
//! ON). The gate is forced ON so a plain `cargo test` exercises the
//! compiled path.
use xezim::simulate;

fn gate_on() -> bool {
    super::compiled_method_test_env::eager()
}

/// Subprocess stdout capture — for the `$display`-containing void body
/// (which declines to the AST interpreter; its stdout must still match).
fn xezim_bin() -> std::path::PathBuf {
    // Cargo names the binary it built for the integration tests.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_xezim"))
}

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

#[test]
fn void_methods_and_ctors_compile() {
    if !gate_on() {
        return;
    }
    let src = r#"
class vc;
  int cnt;
  int marks;
  string tag;
  function new(int start = 1, string t = "c");
    cnt = start;
    marks = 0;
  endfunction
  function void bump(int by);
    cnt = cnt + by;
  endfunction
  function void early(int x);
    if (x > 10) begin
      marks = marks | 1;
      return;
    end
    marks = marks | 2;
  endfunction
  function void chain(int n);
    int i = 0;
    while (i < n) begin
      bump(2);
      i = i + 1;
    end
    early(cnt);
  endfunction
  function void local_ctl(int n);
    int acc = 0;
    int i = 0;
    while (i < n) begin
      acc = acc + i;
      i = i + 1;
    end
    if (acc > 4) marks = marks + 4;
    else marks = marks + 8;
  endfunction
  // `void'(...)` wrapper around a bare void call, inside a compiled body
  // (the statement arm peels the Paren and takes the bare-call lowering).
  function void cast_call(int n);
    void'(bump(n));
  endfunction
  function void set_tag(string t);
    tag = t;
  endfunction
  function void noisy(int x);
    $display("noisy %0d", x);
    tag = "noisy";
  endfunction
  function int get_cnt();
    return cnt;
  endfunction
  function int get_marks();
    return marks;
  endfunction
  function string get_tag();
    return tag;
  endfunction
endclass

class vd extends vc;
  int extra;
  function new(int e);
    super.new(e, "b");
    extra = e;
  endfunction
  function int get_extra();
    return extra;
  endfunction
  // bare calls to INHERITED methods from the derived class's compiled
  // body (the admission set walks the extends chain; runtime dispatch
  // resolves the base implementation on the vd handle).
  function void d_help(int n);
    bump(n);
    early(cnt);
  endfunction
endclass

module top;
  vc c0, c1;
  vd d0;
  int r1, r2, r3, r4, r5, r6, r7, r8, m1, m2, m3;
  string s2, s5, s7v;
  initial begin
    c0 = new();
    r1 = c0.get_cnt();
    c1 = new(5, "z");
    r2 = c1.get_cnt();
    c1.set_tag("z");
    s2 = c1.get_tag();
    c1.bump(3);
    r3 = c1.get_cnt();
    c1.chain(2);
    r4 = c1.get_cnt();
    m1 = c1.get_marks();
    c1.local_ctl(4);
    m2 = c1.get_marks();
    c1.noisy(7);
    s5 = c1.get_tag();
    d0 = new(9);
    r5 = d0.get_cnt();
    r6 = d0.get_extra();
    s7v = d0.get_tag();
    d0.d_help(1);
    r7 = d0.get_cnt();
    m3 = d0.get_marks();
    c1.cast_call(10);
    r8 = c1.get_cnt();
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulation should run");
    assert_eq!(u(&sim, "r1"), 1, "ctor default formal");
    assert_eq!(u(&sim, "r2"), 5, "ctor explicit formal");
    assert_eq!(u(&sim, "r3"), 8, "void bump via module caller");
    // chain(2): bump(2) x2 -> 12; early(12) takes the >10 branch (marks|=1)
    assert_eq!(u(&sim, "r4"), 12, "chain bare calls");
    assert_eq!(u(&sim, "m1"), 1, "early-return branch");
    // local_ctl(4): acc = 0+1+2+3 = 6 > 4 -> marks += 4
    assert_eq!(u(&sim, "m2"), 5, "void method locals + while");
    assert_eq!(
        u(&sim, "r5"),
        9,
        "derived ctor via super.new (AST fallback)"
    );
    assert_eq!(u(&sim, "r6"), 9, "derived extra store");
    // d_help(1): inherited bump(1) -> 10; early(10) not >10 -> marks|=2
    assert_eq!(u(&sim, "r7"), 10, "inherited bare call");
    assert_eq!(u(&sim, "m3"), 2, "inherited bare call, else branch");
    // void'(bump(10)) inside a compiled body: 12 + 10 = 22
    assert_eq!(u(&sim, "r8"), 22, "void' wrapper around bare call");
}

/// A `$display` body declines to the AST interpreter (Expr_display is not
/// a compiled statement in method mode today); its stdout must still be
/// byte-identical, proving the clean-fallback contract.
#[test]
fn void_display_body_falls_back_cleanly() {
    if !gate_on() {
        return;
    }
    let src = r#"
class vc2;
  int x;
  function new();
    x = 0;
  endfunction
  function void noisy(int v);
    $display("noisy %0d", v);
    x = v;
  endfunction
  function int get_x();
    return x;
  endfunction
endclass

module top;
  int r1;
  initial begin
    vc2 c;
    c = new();
    c.noisy(7);
    r1 = c.get_x();
  end
endmodule
"#;
    let dir = std::env::temp_dir().join("xezim_void_test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("method_void_display.sv");
    std::fs::write(&path, src).unwrap();
    let out = std::process::Command::new(xezim_bin())
        .args(["--simulate", "-s", "top"])
        .arg(&path)
        .env("XEZIM_COMPILE_METHODS", "1")
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("noisy 7"),
        "declining void body must still print via AST: {}",
        stdout
    );
}
