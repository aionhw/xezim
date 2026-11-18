//! §13.5, §11.6.1, §11.8.1: an input actual is assigned to its formal on
//! entry, so it is context-determined by the formal's width, for a task and a
//! covergroup constructor as for a function. Both evaluated it
//! self-determined, so `t(x + y, o)` and `cg = new(x + y)` over 8-bit operands
//! into an `int` formal kept 44 instead of 300. A formal written with neither
//! a type nor a direction (`input int a, b`) takes the previous formal's type,
//! so `b`'s actual is sized like `a`'s. Every expected line was checked on the
//! reference simulator. Run compiled and interpreted (#1256).

use std::process::Command;

const DESIGN: &str = r#"
covergroup cg_int (int lo) with function sample (int v);
  option.per_instance = 1;
  cp : coverpoint v { bins hit = {lo}; }
endgroup
covergroup cg_vec (bit [8:0] lo) with function sample (int v);
  option.per_instance = 1;
  cp : coverpoint v { bins hit = {lo}; }
endgroup
covergroup cg_dflt (int lo = 8'd200 + 8'd100) with function sample (int v);
  option.per_instance = 1;
  cp : coverpoint v { bins hit = {lo}; }
endgroup

module top;
  byte unsigned x, y;
  bit a, b;
  task automatic t_int(input int v, output int o); o = v; endtask
  task automatic t_vec(input bit [8:0] v, output int o); o = v; endtask
  function automatic int f_two(input int a, b); return a + b; endfunction
  class k_t;
    byte unsigned p = 200, q = 100;
    task run(output int o); t_int(p + q, o); endtask
    function int m_two(input int a, b); return a + b; endfunction
    function int run_two(); return m_two(p + q, p + q); endfunction
  endclass
  task automatic check(string what, int got, int expected);
    if (got == expected) $display("PASS %s = %0d", what, got);
    else                 $display("FAIL %s = %0d, expected %0d", what, got, expected);
  endtask
  initial begin
    k_t k = new;
    cg_int c1, c2;
    cg_vec c3;
    cg_dflt c4;
    int o;
    x = 200; y = 100; a = 0; b = 1;
    t_int(x + y, o);          check("task int input", o, 300);
    t_int(a | (b << 1), o);   check("task int input, shift", o, 2);
    t_vec(x + y, o);          check("task bit [8:0] input", o, 300);
    k.run(o);                 check("task input from a class task", o, 300);
    check("inherited formal type", f_two(x + y, x + y), 600);
    check("inherited formal type, method", k.run_two(), 600);
    c1 = new(x + y);          c1.sample(300);
    check("covergroup int actual", c1.get_inst_coverage(), 100);
    c2 = new(a | (b << 1));   c2.sample(2);
    check("covergroup int actual, shift", c2.get_inst_coverage(), 100);
    c3 = new(x + y);          c3.sample(300);
    check("covergroup bit [8:0] actual", c3.get_inst_coverage(), 100);
    c4 = new();               c4.sample(300);
    check("covergroup default", c4.get_inst_coverage(), 100);
    $finish;
  end
endmodule
"#;

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_subr_actual_width_{}_{}",
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

#[test]
fn task_and_covergroup_actuals_take_formal_width() {
    for compile in ["1", "0"] {
        let log = run(compile);
        let pass = log.lines().filter(|l| l.starts_with("PASS ")).count();
        assert!(
            pass == 10 && !log.contains("FAIL"),
            "XEZIM_COMPILE_METHODS={compile}: {pass} of 10 checks passed:\n{log}"
        );
    }
}
