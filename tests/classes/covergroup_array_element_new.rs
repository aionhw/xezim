//! §19.3, §19.4: an element of an array of covergroup instances is built
//! with `cg[i] = new(args)`; the target's type says what `new` builds, not
//! the enclosing class (§8.7), and the actuals bind by position or name
//! (§13.5.3, §13.5.4). Left to the plain assignment, the call was taken for
//! the enclosing class's constructor: a constructor recursed until the stack
//! overflowed, a method re-ran the constructor and left the element null.
//! Building one instance of an outer covergroup type in an object also
//! dropped every other instance of that type the object held (a rule meant
//! for the one embedded covergroup of a class).
//!
//! Each element built with `new(n)` needs `n` samples of its bin to reach
//! 100% (`option.at_least`), so the count says which arguments it received
//! and that it is an instance of its own. Every expected line was checked on
//! the reference simulator. Run compiled and interpreted (#1251).

use std::process::Command;

/// The design of #1251.
const ISSUE: &str = r#"
covergroup cg_t (int need, int lo) with function sample (int v);
  option.per_instance = 1;
  option.at_least = need;
  cp : coverpoint v { bins hit = {lo}; }
endgroup

class owner_t;
  static int ctor_calls;
  cg_t cg[3];
  cg_t each[2];
  cg_t grid[2][2];
  function new();
    ctor_calls++;
    if (ctor_calls > 2) return;                // stop a recursion (see the first check)
    cg[1] = new(2, 5);                         // explicit index, in the constructor
    foreach (each[i]) each[i] = new(i + 1, 5); // foreach, in the constructor
    grid[1][0] = new(3, 5);                    // 2-D array
  endfunction
  function void rebuild();
    cg[2] = new(4, 5);                         // in an ordinary method
    cg[0] = new(.lo(5), .need(1));             // named actuals, out of order
  endfunction
  // samples needed until an element is fully covered; -1 when it never is
  function int samples_to_cover(string which);
    for (int k = 1; k <= 6; k++) begin
      case (which)
        "cg0":  cg[0].sample(5);
        "cg1":  cg[1].sample(5);
        "cg2":  cg[2].sample(5);
        "each0": each[0].sample(5);
        "each1": each[1].sample(5);
        default: grid[1][0].sample(5);
      endcase
      case (which)
        "cg0":  if (cg[0].get_inst_coverage() == 100.0) return k;
        "cg1":  if (cg[1].get_inst_coverage() == 100.0) return k;
        "cg2":  if (cg[2].get_inst_coverage() == 100.0) return k;
        "each0": if (each[0].get_inst_coverage() == 100.0) return k;
        "each1": if (each[1].get_inst_coverage() == 100.0) return k;
        default: if (grid[1][0].get_inst_coverage() == 100.0) return k;
      endcase
    end
    return -1;
  endfunction
endclass

module top;
  cg_t loose[2];                               // a module-scope array

  task automatic check(string what, int got, int expected);
    if (got == expected) $display("PASS %s = %0d", what, got);
    else                 $display("FAIL %s = %0d, expected %0d", what, got, expected);
  endtask

  initial begin
    owner_t o;
    int     k;
    o = new();
    check("the constructor ran once",             owner_t::ctor_calls, 1);
    check("cg[1] = new(2, 5) in the constructor", o.samples_to_cover("cg1"), 2);
    check("foreach element 0 in the constructor", o.samples_to_cover("each0"), 1);
    check("foreach element 1 in the constructor", o.samples_to_cover("each1"), 2);
    check("2-D element in the constructor",       o.samples_to_cover("grid"), 3);
    o.rebuild();
    check("the constructor did not run again",    owner_t::ctor_calls, 1);
    check("cg[2] = new(4, 5) in a method",        o.samples_to_cover("cg2"), 4);
    check("named actuals out of order",           o.samples_to_cover("cg0"), 1);
    check("an earlier element is still there",    o.samples_to_cover("each0"), 1);
    o.cg[1] = new(3, 5);                          // from module code, through a handle
    check("o.cg[1] = new(3, 5) from module code", o.samples_to_cover("cg1"), 3);
    loose[1] = new(2, 5);
    k = -1;
    for (int n = 1; n <= 6 && k < 0; n++) begin
      loose[1].sample(5);
      if (loose[1].get_inst_coverage() == 100.0) k = n;
    end
    check("module-scope array element",           k, 2);
    $finish;
  end
endmodule
"#;

/// Dynamic and associative arrays, `this.cg[i]`, two scalars of one type,
/// named actuals into a scalar, a procedural local array, a derived class,
/// a handle chain and an element of a handle array.
const SHAPES: &str = r#"
covergroup cg_t (int need, int lo = 5) with function sample (int v);
  option.per_instance = 1;
  option.at_least = need;
  cp : coverpoint v { bins hit = {lo}; }
endgroup

class own_t;
  static int ctor_calls;
  cg_t d[];
  cg_t aa[int];
  cg_t a, b;
  cg_t cg[2];
  function new();
    ctor_calls++;
    if (ctor_calls > 3) return;
    d = new[2];
    d[1] = new(2);              // dynamic array, default lo
    aa[7] = new(3, 5);          // associative array
    this.cg[0] = new(1);        // this.cg[i]
    a = new(1);                 // two scalars of one type
    b = new(.lo(5), .need(2));  // scalar named
  endfunction
  function int local_arr();
    cg_t loc[2];
    loc[1] = new(2);
    for (int k = 1; k <= 6; k++) begin loc[1].sample(5); if (loc[1].get_inst_coverage() == 100.0) return k; end
    return -1;
  endfunction
endclass

class der_t extends own_t;
  cg_t extra[2];
  function new();
    super.new();
    extra[1] = new(4);
  endfunction
endclass

class wrap_t;
  own_t o;
  function new(); o = new; endfunction
endclass

`define COVER_K(c, k) begin k = -1; for (int n = 1; n <= 6 && k < 0; n++) begin c.sample(5); if (c.get_inst_coverage() == 100.0) k = n; end end

module top;
  task automatic check(string what, int got, int expected);
    if (got == expected) $display("PASS %s = %0d", what, got);
    else                 $display("FAIL %s = %0d, expected %0d", what, got, expected);
  endtask
  initial begin
    own_t o = new;
    der_t dd;
    wrap_t w = new;
    own_t os[2];
    int k;
    check("ctor once", own_t::ctor_calls, 2);
    `COVER_K(o.d[1], k); check("dyn elem", k, 2);
    `COVER_K(o.aa[7], k); check("assoc elem", k, 3);
    `COVER_K(o.cg[0], k); check("this.cg[0]", k, 1);
    `COVER_K(o.a, k); check("scalar a still alive", k, 1);
    `COVER_K(o.b, k); check("scalar b named", k, 2);
    check("local array", o.local_arr(), 2);
    dd = new;
    `COVER_K(dd.extra[1], k); check("derived extra", k, 4);
    `COVER_K(dd.d[1], k); check("derived base elem", k, 2);
    w.o.cg[1] = new(3);
    `COVER_K(w.o.cg[1], k); check("handle chain", k, 3);
    os[1] = new;
    os[1].cg[1] = new(2);
    `COVER_K(os[1].cg[1], k); check("handle array elem", k, 2);
    check("ctor calls total", own_t::ctor_calls, 4);
    $finish;
  end
endmodule
"#;

fn run(design: &str, compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_cg_elem_new_{}_{}",
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
fn covergroup_array_element_new_in_class() {
    check(ISSUE, 11);
}

#[test]
fn covergroup_array_element_new_shapes() {
    check(SHAPES, 12);
}
