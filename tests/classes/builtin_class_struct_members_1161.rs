//! §7.2, §8.4, §15.3, §15.4, §6.18 (#1161): a `mailbox` or `semaphore`
//! member of an unpacked struct (inline or through a typedef, nested, a
//! class property, in a generate block, local to a block, a task or a
//! function) is constructed by `new()` with its bound or key count and is
//! null until then; an unassigned element of an array of such handles is
//! null; a property whose declaration initializer is `new()` and whose type
//! is a typedef of a built-in class is constructed. An unpacked struct in a
//! generate block is stored member by member. The expected lines are the
//! reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
typedef mailbox #(int) cmbox_t;
typedef semaphore csem_t;
typedef struct { mailbox #(int) mb; } st_inl_t;
typedef struct { cmbox_t mb; int k; } st_td_t;
typedef struct { semaphore s; } ss_inl_t;
typedef struct { st_td_t in; } st_o_t;
class C; st_inl_t si; cmbox_t cm = new(); static cmbox_t sm = new(); static csem_t ss = new(2);
  function void go(); si.mb = new(); void'(si.mb.try_put(5)); $display("C.go %0d", si.mb.num()); endfunction
endclass
module mod_scope;
  st_inl_t si; st_td_t st; ss_inl_t ssi; st_o_t ot;
  cmbox_t arr[2]; mailbox #(int) arr2[2];
  C c = new;
  int v;
  initial begin
    $display("null: %0d %0d %0d %0d | %0d %0d", si.mb == null, st.mb == null, ssi.s == null, ot.in.mb == null, arr[1] == null, arr2[0] == null);
    si.mb = new(); void'(si.mb.try_put(1)); $display("E %0d", si.mb.num());
    st.mb = new(2); void'(st.mb.try_put(1)); void'(st.mb.try_put(2)); $display("F %0d full=%0d", st.mb.num(), st.mb.try_put(3));
    ssi.s = new(1); $display("S %0d %0d", ssi.s.try_get(1), ssi.s.try_get(1));
    ot.in.mb = new(); void'(ot.in.mb.try_put(7)); void'(ot.in.mb.try_get(v)); $display("Y %0d %0d", v, ot.in.mb.num());
    $display("null2: %0d %0d", si.mb == null, st.mb != null);
    c.go();
    c.si.mb = new(); void'(c.si.mb.try_put(1)); $display("AL1 %0d", c.si.mb.num());
    void'(c.cm.try_put(3)); $display("L %0d null=%0d", c.cm.num(), c.cm == null);
    void'(C::sm.try_put(3)); $display("AC %0d %0d", C::sm.num(), C::ss.try_get(2));
  end
endmodule
module gen_scope;
  genvar gi;
  generate if (1) begin : g_blk
    st_inl_t g_si; st_td_t g_st;
    initial begin
      $display("gen null %0d %0d", g_si.mb == null, g_st.mb == null);
      g_si.mb = new(); g_si.mb.put(1); $display("gen num %0d", g_si.mb.num());
      g_st.mb = new(); void'(g_st.mb.try_put(1)); $display("gen num %0d", g_st.mb.num());
    end
  end endgenerate
  for (gi = 0; gi < 2; gi++) begin : g_loop
    st_inl_t s;
    initial begin #1 s.mb = new(); s.mb.put(gi); s.mb.put(5); $display("loop %0d null=%0d num=%0d", gi, s.mb == null, s.mb.num()); end
  end
  task automatic tk(output bit ok); st_inl_t l; st_td_t m; ok = (l.mb == null) && (m.mb == null); l.mb = new(); l.mb.put(3); $display("task num %0d", l.mb.num()); endtask
  function automatic bit fn(); st_inl_t l; return l.mb == null; endfunction
  initial begin
    bit ok;
    begin st_inl_t l; st_td_t m; $display("blk null %0d %0d", l.mb == null, m.mb == null); end
    tk(ok); $display("task null %0d fn null %0d", ok, fn());
  end
endmodule
module top;
  mod_scope u_m ();
  gen_scope u_g ();
  initial #10 $finish;
endmodule
"#;

const EXPECTED: &[&str] = &[
    "null: 1 1 1 1 | 1 1",
    "E 1",
    "F 2 full=0",
    "S 1 0",
    "Y 7 0",
    "null2: 0 1",
    "C.go 1",
    "AL1 1",
    "L 1 null=0",
    "AC 1 1",
    "gen null 1 1",
    "gen num 1",
    "gen num 1",
    "blk null 1 1",
    "task num 1",
    "task null 1 fn null 1",
    "loop 0 null=0 num=2",
    "loop 1 null=0 num=2",
];

fn run(compile: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "xezim_mbx_struct_{}_{}",
        std::process::id(),
        compile
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.sv");
    std::fs::write(&path, DESIGN).unwrap();
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
fn builtin_class_members_of_structs_construct_and_start_null() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let got: Vec<&str> = log.lines().filter(|l| EXPECTED.contains(l)).collect();
        assert_eq!(got, EXPECTED, "XEZIM_COMPILE_METHODS={compile}:\n{log}");
    }
}
