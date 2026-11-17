//! §13.5.2: a `ref` formal names its actual's storage, so the subroutine
//! reads a write another process makes while it waits, and its own writes are
//! visible before it returns. Formals bound to a class property (handle,
//! `int`, `string`, `semaphore`, `real`, vector, packed struct), a struct
//! member, an element of a queue, dynamic or associative array, a module
//! string or a hierarchical name read the value the actual had at the call;
//! so did a method of a handle formal (`m.num()`), a chained `ref`, and
//! `wait`/`@` on the formal. A by-value formal keeps its copy (`s2`). The
//! expected lines are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
package rp;
  typedef struct packed { logic [3:0] a; logic [3:0] b; } pk_t;
  class st_c; bit got; int val; endclass
  class hold_c;
    mailbox #(int) mb; mailbox #(int) mb2;
    int iv, iv2, iv3, iv4, iv5, iv6, iv7, iv8, iv9;
    string sv;
    semaphore sem;
    real rv;
    logic [7:0] lv;
    hold_c next;
    pk_t pk;
  endclass
  task automatic poll(ref mailbox #(int) m, st_c st);
    int n, v;
    repeat (8) begin
      n = (m == null) ? 0 : m.num();
      if (n > 0 && !st.got) begin m.get(v); st.got = 1; st.val = v; end
      #15;
    end
  endtask
  task automatic val_poll(mailbox #(int) m, st_c st);
    int v;
    repeat (8) begin
      if (m != null && m.num() > 0 && !st.got) begin m.get(v); st.got = 1; st.val = v; end
      #15;
    end
  endtask
  task automatic rd(ref int x, st_c st);   #10; st.val = x; st.got = 1; endtask
  task automatic wr(ref int x);            #10; x = 77; #10; endtask
  task automatic two(ref int a, ref int b, ref int obs); #10; a = 5; obs = b; endtask
  task automatic hd(ref hold_c hh, st_c st); #10; st.val = (hh == null) ? -1 : hh.iv; st.got = 1; endtask
  task automatic rs(ref string s, st_c st); #10; st.val = s.len(); st.got = 1; endtask
  task automatic sm(ref semaphore s, st_c st);
    repeat (4) begin if (s != null && s.try_get() && !st.got) st.got = 1; #10; end
  endtask
  task automatic rr(ref real r, st_c st);  #10; st.val = int'(r * 2); st.got = 1; endtask
  task automatic rl(ref logic [7:0] v, st_c st); #10; st.val = v[3:0]; st.got = 1; endtask
  task automatic leaf_rd(ref int b, st_c st); #20; st.val = b; st.got = 1; endtask
  task automatic mid_rd(ref int a, st_c st);  leaf_rd(a, st); endtask
  task automatic leaf_wr(ref int b);          #5; b = 88; #15; endtask
  task automatic mid_wr(ref int a);           leaf_wr(a); endtask
  task automatic wt(ref int x, st_c st);      wait (x == 42); st.val = x; st.got = 1; endtask
  task automatic ev(ref int x, st_c st);      @(x); st.val = x; st.got = 1; endtask
  task automatic rpk(ref pk_t p, st_c st); #10; st.val = p.a * 16 + p.b; st.got = 1; p.b = 4'd2; endtask
  class runner_c;
    task run(ref mailbox #(int) m, st_c st);
      int v;
      repeat (8) begin
        if (m != null && m.num() > 0 && !st.got) begin m.get(v); st.got = 1; st.val = v; end
        #15;
      end
    endtask
  endclass
endpackage
interface rif;
  import rp::*;
  int iv;
  task automatic itask(ref int x, st_c st); #10; st.val = x; st.got = 1; endtask
endinterface
module rsub;
  int lv;
endmodule
module top;
  import rp::*;
  typedef struct { int iv; } ist_t;
  mailbox #(int) mb_a;
  hold_c h = new;
  string ms;
  ist_t js;
  int q[$], da[], aa[int];
  int p_mid, q_obs, ab_mid;
  rif u_if();
  rsub u_sub();
  st_c s0 = new, s1 = new, s2 = new, s3 = new, s4 = new, s5 = new, s6 = new, s7 = new,
       s8 = new, s9 = new, s10 = new, s11 = new, s12 = new, s13 = new, s14 = new,
       s15 = new, s16 = new, s17 = new, s18 = new, s19 = new, s20 = new, s21 = new;
  // module-variable mailbox, constructed after the call
  initial #0 poll(mb_a, s0);
  initial begin #1 mb_a = new(); #49 mb_a.put(10); end
  // class property mailbox
  initial #0 poll(h.mb, s1);
  initial begin #1 h.mb = new(); #49 h.mb.put(11); end
  // by-value control: the late new() is not seen
  initial #0 val_poll(h.mb2, s2);
  initial begin #1 h.mb2 = new(); #49 h.mb2.put(12); end
  // class property int, read after the caller's write
  initial #0 rd(h.iv, s3);
  initial begin #5 h.iv = 13; end
  // the callee's write is visible before it returns
  initial #0 wr(h.iv2);
  initial begin #15 p_mid = h.iv2; end
  // two formals, one actual
  initial #0 two(h.iv3, h.iv3, q_obs);
  // class handle, string, semaphore, real, logic vector properties
  initial #0 hd(h.next, s4);
  initial begin #5 h.next = new; h.next.iv = 14; end
  initial #0 rs(h.sv, s5);
  initial begin #5 h.sv = "hello"; end
  initial #0 sm(h.sem, s6);
  initial begin #5 h.sem = new(1); end
  initial #0 rr(h.rv, s7);
  initial begin #5 h.rv = 7.5; end
  initial #0 rl(h.lv, s8);
  initial begin #5 h.lv = 8'h3c; end
  // chained refs, read and write
  initial #0 mid_rd(h.iv4, s9);
  initial begin #5 h.iv4 = 15; end
  initial #0 mid_wr(h.iv5);
  initial begin #10 ab_mid = h.iv5; end
  // wait and event control on a formal aliased to a property
  initial #0 wt(h.iv6, s10);
  initial begin #5 h.iv6 = 42; end
  initial #0 ev(h.iv7, s11);
  initial begin #5 h.iv7 = 16; end
  // interface task and class method callees
  initial #0 u_if.itask(h.iv8, s12);
  initial begin #5 h.iv8 = 17; end
  initial begin runner_c r = new; #0 r.run(h.mb2, s13); end
  // module string, struct member, collection elements, hierarchical names
  initial #0 rs(ms, s14);
  initial begin #5 ms = "abc"; end
  initial #0 rd(js.iv, s15);
  initial begin #5 js.iv = 18; end
  initial begin q.push_back(0); #0 rd(q[0], s16); end
  initial begin #5 q[0] = 19; end
  initial begin da = new[2]; #0 rd(da[1], s17); end
  initial begin #5 da[1] = 20; end
  initial begin aa[7] = 0; #0 rd(aa[7], s18); end
  initial begin #5 aa[7] = 21; end
  initial #0 rd(u_sub.lv, s19);
  initial begin #5 u_sub.lv = 22; end
  initial #0 rpk(h.pk, s21);
  initial begin #5 h.pk.a = 4'd9; h.pk.b = 4'd1; end
  initial #0 rd(u_if.iv, s20);
  initial begin #5 u_if.iv = 23; end
  initial begin
    st_c all[$];
    #200;
    all = '{s0, s1, s2, s3, s4, s5, s6, s7, s8, s9, s10, s11, s12, s13, s14, s15,
            s16, s17, s18, s19, s20, s21};
    foreach (all[i]) $display("s%0d got=%0d val=%0d", i, all[i].got, all[i].val);
    $display("p_mid=%0d q_obs=%0d ab_mid=%0d h.iv2=%0d h.iv5=%0d h.pk=%h", p_mid, q_obs, ab_mid, h.iv2, h.iv5, h.pk);
  end
endmodule
"#;

const EXPECTED: &[&str] = &[
    "s0 got=1 val=10",
    "s1 got=1 val=11",
    "s2 got=0 val=0",
    "s3 got=1 val=13",
    "s4 got=1 val=14",
    "s5 got=1 val=5",
    "s6 got=1 val=0",
    "s7 got=1 val=15",
    "s8 got=1 val=12",
    "s9 got=1 val=15",
    "s10 got=1 val=42",
    "s11 got=1 val=16",
    "s12 got=1 val=17",
    "s13 got=1 val=12",
    "s14 got=1 val=3",
    "s15 got=1 val=18",
    "s16 got=1 val=19",
    "s17 got=1 val=20",
    "s18 got=1 val=21",
    "s19 got=1 val=22",
    "s20 got=1 val=23",
    "s21 got=1 val=145",
    "p_mid=77 q_obs=5 ab_mid=88 h.iv2=77 h.iv5=88 h.pk=92",
];

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_ref_actual_storage_{}_{}",
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
fn ref_formal_reads_and_writes_its_actual_while_the_call_runs() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let got: Vec<&str> = log
            .lines()
            .filter(|l| (l.starts_with('s') && l.contains(" got=")) || l.starts_with("p_mid="))
            .collect();
        assert_eq!(got, EXPECTED, "XEZIM_COMPILE_METHODS={compile}:\n{log}");
    }
}
