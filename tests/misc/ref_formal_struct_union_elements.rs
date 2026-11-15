//! §13.5.2: a `ref` formal names its actual's storage for the whole call.
//! An unpacked struct or union formal is aliased leaf by leaf (a property of
//! an object, a module variable), so a parked task reads a later write and
//! its own write is visible before it returns. Two activations of one task
//! each keep the layout of their own packed struct formal. A `ref` handle
//! formal bound to an element of an array or a queue reaches the object. A
//! member of an element of an array of interface instances is aliased.
//! §7.8.7: an associative-array entry that does not exist is allocated when
//! it is the actual of a `ref` argument, with the array's default value
//! (`dflt[3]` starts at 5; the reference simulator starts it at 0, the other
//! lines are its output).
use std::process::Command;

const DESIGN: &str = r#"
package rq;
  typedef struct { int iv; byte b; } st_t;
  typedef union { int iv; byte bb[4]; } un_t;
  typedef struct packed { logic [3:0] a; logic [3:0] b; } pk_t;
  class res_c; int val; bit got; endclass
  class hold_c; st_t stv; un_t un; pk_t pk; endclass
  task automatic rd_st(ref st_t s, input res_c r); #10; r.val = s.iv; r.got = 1; endtask
  task automatic rd_un(ref un_t u, input res_c r); #10; r.val = u.iv; r.got = 1; endtask
  task automatic rd_pk(ref pk_t s, input res_c r); #10; r.val = int'(s.a); r.got = 1; endtask
  task automatic wr_st(ref st_t s); #20; s.iv = 77; #10; endtask
  task automatic wr_un(ref un_t u); #20; u.iv = 78; #10; endtask
  task automatic rd_int(ref int x, input res_c r); #10; r.val = x; r.got = 1; endtask
  // `r` takes the direction of the formal before it: a `ref` handle.
  task automatic rd_ref(ref int x, res_c r); #10; r.val = x; r.got = 1; endtask
  task automatic bump(ref int x); x = x + 1; endtask
  function automatic int by_val(input un_t u); return u.iv; endfunction
endpackage
interface rq_if; int iv; endinterface
module top;
  import rq::*;
  hold_c h = new;
  st_t m_st; un_t m_un; pk_t m_pk;
  int maa[int]; int dflt[int] = '{default: 5};
  int mi, mj;
  rq_if u_ifa [2] ();
  res_c s[12]; res_c q[$];
  initial begin
    foreach (s[i]) s[i] = new;
    begin res_c t; t = new; q.push_back(t); end
  end
  // An unpacked struct or union formal: a property and a module variable.
  initial rd_st(h.stv, s[0]);     initial #5 h.stv.iv = 20;
  initial rd_un(h.un, s[1]);      initial #5 h.un.iv = 248;
  initial rd_st(m_st, s[2]);      initial #5 m_st.iv = 21;
  initial rd_un(m_un, s[3]);      initial #5 m_un.iv = 249;
  // Two activations of one task at once, each with its own packed struct.
  initial rd_pk(m_pk, s[4]);      initial begin #5 m_pk.a = 4'd7; m_pk.b = 4'd3; end
  initial rd_pk(h.pk, s[5]);      initial #5 h.pk.a = 4'd9;
  // An associative-array entry that does not exist at the call.
  initial rd_int(maa[7], s[6]);   initial #5 maa[7] = 212;
  // A member of an element of an array of interface instances.
  initial rd_int(u_ifa[1].iv, s[7]); initial #5 u_ifa[1].iv = 204;
  // A `ref` handle formal bound to an element of an array or a queue.
  initial rd_ref(mi, s[8]);       initial #5 mi = 9;
  initial rd_ref(mj, q[0]);       initial #5 mj = 10;
  initial wr_st(h.stv);  initial wr_st(m_st);  initial wr_un(h.un);  initial wr_un(m_un);
  initial begin
    #2 $display("alloc size=%0d exists=%0d", maa.size(), maa.exists(7));
    #23 $display("mid %0d %0d %0d %0d", h.stv.iv, m_st.iv, h.un.iv, m_un.iv);
    #75;
    foreach (s[i]) if (i < 9) $display("s[%0d] got=%0d val=%0d", i, s[i].got, s[i].val);
    $display("q[0] got=%0d val=%0d", q[0].got, q[0].val);
    bump(dflt[3]);
    $display("dflt[3]=%0d num=%0d by_val=%0d", dflt[3], dflt.num(), by_val(m_un));
    $finish;
  end
endmodule
"#;

const EXPECTED: &[&str] = &[
    "alloc size=1 exists=1",
    "mid 77 77 78 78",
    "s[0] got=1 val=20",
    "s[1] got=1 val=248",
    "s[2] got=1 val=21",
    "s[3] got=1 val=249",
    "s[4] got=1 val=7",
    "s[5] got=1 val=9",
    "s[6] got=1 val=212",
    "s[7] got=1 val=204",
    "s[8] got=1 val=9",
    "q[0] got=1 val=10",
    "dflt[3]=6 num=1 by_val=78",
];

fn run(compile: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "xezim_ref_struct_union_{}_{}",
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
fn ref_struct_union_and_element_formals_alias_their_actuals() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let got: Vec<&str> = log
            .lines()
            .filter(|l| {
                ["alloc ", "mid ", "s[", "q[", "dflt["]
                    .iter()
                    .any(|p| l.starts_with(p))
            })
            .collect();
        assert_eq!(got, EXPECTED, "XEZIM_COMPILE_METHODS={compile}:\n{log}");
    }
}
