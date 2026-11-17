//! §7.5.1, §8.7: `new[n]` (and `new[n](src)`) that sizes a row of an array of
//! dynamic arrays, or a dynamic member of a struct, inside a class method is
//! the dynamic-array constructor. It parses like the call `new(n)`, and the
//! whole-collection store evaluated it as one before the sized-new store ran,
//! so the enclosing class's constructor ran again on `this` (with the size as
//! its argument) once per statement, and a size expression that is a call ran
//! twice. Each shape checks one constructor run, an unchanged object name and
//! the size asked for; the expected lines are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class cell_t; int v; endclass
class box_t;  int v; function new(int x = 0); v = x; endfunction endclass
typedef struct { int q[]; int tag; } rec_t;

class base_t;
  string name;
  int    ctor_calls;
  function new(string n = "base");
    ctor_calls++;
    name = n;
  endfunction
endclass

class holder_t extends base_t;
  cell_t c2[][];
  box_t  b2[][];
  box_t  fh[3];
  int    i2[][];
  int    t3[][][];
  int    dq[$][];
  int    aa[string][];
  int    fa[3][];
  int    d[];
  rec_t  s;
  rec_t  sarr[2];
  static int sf[3][];
  int    pd[]    = new[8];
  int    pdd[][] = new[2];
  int    n = 8;
  int    k = 0;
  int    evals;
  int    last_row;

  function new(string nm = "holder");
    super.new(nm);
  endfunction
  function int next();
    evals++;
    return 8;
  endfunction

  function void s_cell_row();     c2 = new[1]; c2[0] = new[n]; endfunction
  function void s_int_row();      i2 = new[1]; i2[0] = new[n]; endfunction
  function void s_this_row();     c2 = new[1]; this.c2[0] = new[n]; endfunction
  function void s_copy_row();     i2 = new[1]; i2[0] = new[3]; i2[0] = new[n](i2[0]); endfunction
  function void s_queue_outer();  int t[]; dq.push_back(t); dq[0] = new[n]; endfunction
  function void s_assoc_outer();  aa["k"] = new[n]; endfunction
  function void s_fixed_outer();  fa[1] = new[n]; endfunction
  function void s_static_outer(); sf[1] = new[n]; endfunction
  function void s_struct_member();     s.q = new[n]; endfunction
  function void s_this_struct_member(); this.s.q = new[n]; endfunction
  function void s_struct_array_member(); sarr[1].q = new[n]; endfunction
  function void s_local_array();    int loc[][]; loc = new[2]; loc[1] = new[n]; i2 = loc; endfunction
  function void s_local_init();     int l2[][] = new[2]; l2[1] = new[n]; i2 = l2; endfunction
  function void s_one_d();          d = new[n]; endfunction
  function void s_one_d_copy();     d = new[2]; d = new[n](d); endfunction
  function void s_post_increment(); c2 = new[2]; c2[k++] = new[n]; endfunction
  function void s_loop();           i2 = new[3]; for (int p = 0; p < 3; p++) i2[p] = new[n]; endfunction
  task          s_task();           i2 = new[1]; i2[0] = new[n]; endtask
  function void s_three_d();        t3 = new[1]; t3[0] = new[2]; t3[0][1] = new[n]; endfunction
  function void s_size_call();      i2 = new[2]; i2[0] = new[next()]; endfunction
  function void ref_row(ref int a[][]); a[0] = new[n]; endfunction
  function void s_ref_formal();     i2 = new[1]; ref_row(i2); endfunction
  // An element that IS a class handle is built by the constructor of ITS
  // class: new(5) there is not a dynamic-array new.
  function void s_handle_element();
    b2 = new[1]; b2[0] = new[2]; b2[0][1] = new(5); fh[1] = new(6);
  endfunction
  task s_static_local();
    static int tmp[][];
    tmp = new[n];
    for (int i = 0; i < n; i++) tmp[i] = new[n];
    last_row = tmp[n-1].size();
  endtask
  task s_automatic_local();
    int tmp[][];
    tmp = new[n];
    for (int i = 0; i < n; i++) tmp[i] = new[n];
    last_row = tmp[n-1].size();
  endtask
endclass

class derived_t extends holder_t;
  function new(string nm = "derived");
    super.new(nm);
  endfunction
  function void s_derived_row(); i2 = new[1]; i2[0] = new[n]; endfunction
endclass

class other_t extends base_t;
  holder_t h;
  function new(string nm = "other");
    super.new(nm);
  endfunction
  function void s_other_object(); h.i2 = new[1]; h.i2[0] = new[8]; endfunction
endclass

class param_t #(type T = int) extends base_t;
  T r[][];
  function new(string nm = "param");
    super.new(nm);
  endfunction
  function void s_param_row(); r = new[1]; r[0] = new[8]; endfunction
endclass

// A constructor that sizes its own row. The bound keeps a build that runs the
// constructor again from recursing for ever.
class own_t;
  int r[][];
  int ctor_calls;
  function new(int w = 8);
    ctor_calls++;
    if (ctor_calls > 3) return;
    r = new[1];
    r[0] = new[w];
  endfunction
endclass

module top;
  holder_t  h;
  derived_t dv;
  other_t   o;
  param_t #(int) pt;
  own_t     ow;
  int mi2[][];
  int mfa[3][];
  int fails = 0;
  int total = 0;

  task automatic chk(string what, int calls, bit name_ok, int got, int want = 8);
    total++;
    if (calls == 1 && name_ok && got == want) $display("PASS %s", what);
    else begin
      fails++;
      $display("FAIL %s: constructor ran %0d time(s), name %s, got %0d, expected %0d",
               what, calls, name_ok ? "kept" : "changed", got, want);
    end
  endtask

  initial begin
    h = new("h"); h.s_cell_row();     chk("row of class handles", h.ctor_calls, h.name == "h", h.c2[0].size());
    h = new("h"); h.s_int_row();      chk("row of ints", h.ctor_calls, h.name == "h", h.i2[0].size());
    h = new("h"); h.s_this_row();     chk("this.row", h.ctor_calls, h.name == "h", h.c2[0].size());
    h = new("h"); h.s_copy_row();     chk("row, new[n](src)", h.ctor_calls, h.name == "h", h.i2[0].size());
    h = new("h"); h.s_queue_outer();  chk("queue of dynamic arrays", h.ctor_calls, h.name == "h", h.dq[0].size());
    h = new("h"); h.s_assoc_outer();  chk("associative array of dynamic arrays", h.ctor_calls, h.name == "h", h.aa["k"].size());
    h = new("h"); h.s_fixed_outer();  chk("fixed array of dynamic arrays", h.ctor_calls, h.name == "h", h.fa[1].size());
    h = new("h"); h.s_static_outer(); chk("static fixed array of dynamic arrays", h.ctor_calls, h.name == "h", holder_t::sf[1].size());
    h = new("h"); h.s_struct_member();      chk("dynamic member of a struct", h.ctor_calls, h.name == "h", h.s.q.size());
    h = new("h"); h.s_this_struct_member(); chk("this.struct member", h.ctor_calls, h.name == "h", h.s.q.size());
    h = new("h"); h.s_struct_array_member(); chk("member of a struct array element", h.ctor_calls, h.name == "h", h.sarr[1].q.size());
    h = new("h"); h.s_local_array();    chk("local array", h.ctor_calls, h.name == "h", h.i2[1].size());
    h = new("h"); h.s_local_init();     chk("local array with initializer", h.ctor_calls, h.name == "h", h.i2[1].size());
    h = new("h"); h.s_one_d();          chk("one-dimensional member", h.ctor_calls, h.name == "h", h.d.size());
    h = new("h"); h.s_one_d_copy();     chk("one-dimensional member, new[n](src)", h.ctor_calls, h.name == "h", h.d.size());
    h = new("h"); h.s_post_increment(); chk("index k++", h.ctor_calls, h.name == "h", h.c2[0].size());
    h = new("h"); h.s_loop();           chk("rows in a loop", h.ctor_calls, h.name == "h", h.i2[2].size());
    h = new("h"); h.s_task();           chk("row in a task", h.ctor_calls, h.name == "h", h.i2[0].size());
    h = new("h"); h.s_three_d();        chk("row of a 3-D array", h.ctor_calls, h.name == "h", h.t3[0][1].size());
    h = new("h"); h.s_size_call();      chk("size expression is a call", h.ctor_calls, h.name == "h", h.i2[0].size());
    chk("size expression is evaluated once", 1, 1, h.evals, 1);
    h = new("h"); h.s_ref_formal();     chk("row through a ref formal", h.ctor_calls, h.name == "h", h.i2[0].size());
    h = new("h"); h.s_handle_element(); chk("handle element built by new(arg)", h.ctor_calls, h.name == "h", h.b2[0][1].v * 10 + h.fh[1].v, 56);
    h = new("h"); h.s_static_local();   chk("static local array of a task", h.ctor_calls, h.name == "h", h.last_row);
    h = new("h"); h.s_automatic_local(); chk("automatic local array of a task", h.ctor_calls, h.name == "h", h.last_row);
    dv = new("d"); dv.s_derived_row();  chk("derived class", dv.ctor_calls, dv.name == "d", dv.i2[0].size());
    h = new("h"); o = new("o"); o.h = h; o.s_other_object();
    chk("row of another object", h.ctor_calls + o.ctor_calls - 1, h.name == "h" && o.name == "o", h.i2[0].size());
    pt = new("p"); pt.s_param_row();    chk("parameterized class", pt.ctor_calls, pt.name == "p", pt.r[0].size());
    ow = new(8);                        chk("constructor that sizes its own row", ow.ctor_calls, 1, ow.r[0].size());
    // Not in a method: these never failed.
    h = new("h"); h.i2 = new[2]; h.i2[1] = new[8]; chk("row of an object, from module scope", h.ctor_calls, h.name == "h", h.i2[1].size());
    h = new("h");                       chk("property initializers", h.ctor_calls, h.name == "h", h.pd.size() + h.pdd.size() - 2);
    mi2 = new[1]; mi2[0] = new[8];      chk("module-scope row", 1, 1, mi2[0].size());
    mfa[2] = new[8];                    chk("module-scope fixed array row", 1, 1, mfa[2].size());
    if (fails == 0) $display("PASS all %0d shapes", total);
    else            $display("FAIL %0d of %0d shapes", fails, total);
    $finish;
  end
endmodule
"#;

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_dyn_new_in_method_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, DESIGN).expect("write design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_COMPILE_METHODS", compile)
        .env_remove("XEZIM_METHOD_CACHE")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    text
}

#[test]
fn dynamic_array_new_in_a_method_runs_no_constructor() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let fails: Vec<&str> = log.lines().filter(|l| l.starts_with("FAIL")).collect();
        assert!(
            fails.is_empty(),
            "XEZIM_COMPILE_METHODS={compile}:\n{}",
            fails.join("\n")
        );
        let passes = log.lines().filter(|l| l.starts_with("PASS ")).count();
        assert_eq!(passes, 34, "XEZIM_COMPILE_METHODS={compile}:\n{log}");
        assert!(
            log.contains("PASS all 33 shapes"),
            "XEZIM_COMPILE_METHODS={compile}:\n{log}"
        );
    }
}
