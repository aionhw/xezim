//! §8.4: only a non-static member access or a virtual method through a null
//! handle is illegal, so a non-virtual method whose body does not touch
//! `this` runs, bound to the receiver's declared class (§8.20) with a null
//! `this`. It returned a silent 0 for most receiver shapes and aborted the run
//! for a top-level `p.m()`. The receivers here are a formal, a function's
//! return value, elements of each kind of collection of handles, properties
//! (implicit and `this.`), a typedef'd, derived, struct-member and package
//! class handle, a task's local and a method calling another one bare. The
//! expected lines are the reference simulator's, except `param`: the
//! reference simulator stops on the parameterized class, which §8.4 allows.
use std::process::Command;

const DESIGN: &str = r#"
class pkt_c;
  int f;
  function int hdr_len(); return 42; endfunction
  function int twice(); return 2 * hdr_len(); endfunction
  function pkt_c nullh(); return null; endfunction
  task drive(output int r); r = 7; endtask
endclass
class der_c extends pkt_c;
  function int base_only(); return 43; endfunction
endclass
class holder_c;
  pkt_c p;
  function int call_implicit(); return p.hdr_len(); endfunction
  function int call_this(); return this.p.hdr_len(); endfunction
endclass
class outer_c;
  holder_c hn;
endclass
class pp #(type T = int, parameter int N = 1);
  function int hdr_len(); return 40 + N; endfunction
endclass
typedef pkt_c pkt_t;
typedef struct { pkt_c p; } s_t;
package npkg;
  class pc2; function int hdr_len(); return 42; endfunction endclass
  function automatic int pf(pc2 h); return h.hdr_len(); endfunction
endpackage
module top;
  pkt_c gp, garr[2], gq[$], gda[], gaa[string];
  holder_c h = new;
  outer_c o = new;
  pp #(int, 2) gpp;
  pkt_t tp;
  der_c dv;
  s_t s;
  npkg::pc2 pv;
  function automatic int call_arg(pkt_c p); return p.hdr_len(); endfunction
  function automatic pkt_c get_null(); return null; endfunction
  task automatic in_task(output int r); pkt_c p; r = p.hdr_len(); endtask
  initial begin
    pkt_c p;
    int v;
    gq.push_back(null);
    gda = new[1];
    gaa["k"] = null;
    $display("formal %0d", call_arg(null));
    $display("return %0d", get_null().hdr_len());
    $display("fixed %0d queue %0d dyn %0d assoc %0d", garr[0].hdr_len(), gq[0].hdr_len(), gda[0].hdr_len(), gaa["k"].hdr_len());
    $display("implicit %0d this %0d", h.call_implicit(), h.call_this());
    $display("typedef %0d derived %0d", tp.hdr_len(), dv.base_only());
    $display("struct %0d package %0d", s.p.hdr_len(), npkg::pf(pv));
    $display("nested %0d returned %0d", p.twice(), h.p.nullh() == null);
    in_task(v);
    $display("task-local %0d", v);
    p.drive(v);
    $display("task %0d", v);
    v = p.hdr_len();
    $display("local %0d null %0d", v, p == null);
    $display("param %0d", gpp.hdr_len());
  end
endmodule
"#;

const EXPECTED: &[&str] = &[
    "formal 42",
    "return 42",
    "fixed 42 queue 42 dyn 42 assoc 42",
    "implicit 42 this 42",
    "typedef 42 derived 43",
    "struct 42 package 42",
    "nested 84 returned 1",
    "task-local 42",
    "task 7",
    "local 42 null 1",
    "param 42",
];

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_null_receiver_{}_{}",
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
fn non_virtual_method_through_null_handle_runs() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let got: Vec<&str> = log
            .lines()
            .filter(|l| {
                !l.starts_with("===") && !l.starts_with("git ") && !l.starts_with("Simulation")
            })
            .collect();
        assert_eq!(got, EXPECTED, "XEZIM_COMPILE_METHODS={compile}:\n{log}");
    }
}
