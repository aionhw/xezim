//! IEEE 1800-2023 §13.5.2: every `output` / `inout` formal of a class task or
//! function is copied back to its actual when the subroutine returns, at its
//! declared width. These run with bytecode compilation on (first call, and
//! after the default call-count tier) and off; all three must print what the
//! reference simulator prints.
//!
//! Compiled bodies used to lose every output but the last one (the exit
//! markers that keep the output registers live were rewritten into plain
//! copies and then dropped as dead), truncated a packed-typedef output to
//! 32 bits, and dropped the outputs of a task called from compiled code.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

fn all_policies() -> bool {
    super::compiled_method_test_env::policies(&[("1", "0"), ("1", "1000"), ("0", "1000")])
}

#[test]
fn every_output_formal_is_copied_back() {
    if !all_policies() {
        return;
    }
    let src = r#"
class A;
  task ta(output int o, output int p); o = 1; p = 2; endtask
  task tb(input int a, output int o, output int p, output int q); o = a+1; p = a+2; q = a+3; endtask
  task tc(input int a, input int b, output int o, output int p); o = a+b; p = a-b; endtask
  task td(input int a, output int o, input int b, output int p); o = a+b; p = a-b; endtask
  task te(input int a, output int o, output int p); p = a+2; o = a+1; endtask
  function void f3(input int a, output int o, output int p); o = a * 2; p = a + 1; endfunction
  function int wrap3(); int x, y; f3(4, x, y); return x * 10 + y; endfunction
endclass
module top;
  int r[12];
  int s3, r1, r2;
  initial begin
    A a = new;
    a.ta(r[0], r[1]); a.tb(10, r[2], r[3], r[4]); a.tc(5, 3, r[5], r[6]); a.td(5, r[7], 3, r[8]); a.te(10, r[9], r[10]);
    $display("T|r=%p", r);
    for (int k = 0; k < 1500; k++) s3 += a.wrap3();
    a.f3(4, r1, r2);
    $display("T|s3=%0d r=%0d %0d", s3, r1, r2);
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|r='{1, 2, 11, 12, 13, 8, 2, 8, 2, 11, 12, 0}",
            "T|s3=127500 r=8 5"
        ]
    );
}

#[test]
fn outputs_pass_through_nested_recursive_and_super_calls() {
    if !all_policies() {
        return;
    }
    let src = r#"
class A;
  int base = 3;
  task t1(output int o); o = 7; endtask
  task t4(inout int x); x += 5; endtask
  task wrap(output int o); t1(o); endtask
  task wrap2(output int o); int tmp; t1(tmp); o = tmp + 100; endtask
  task automatic rec(int n, output int o);
    int t;
    if (n == 0) begin o = 0; return; end
    rec(n - 1, t);
    o = t + n;
  endtask
endclass
class P;
  virtual function int f(int x); return x + 1; endfunction
  function int g(int x); return x * 2; endfunction
  virtual task t(output int o); o = f(10) + g(10); endtask
endclass
class Q extends P;
  virtual function int f(int x); return x + 100; endfunction
  function int g(int x); return x * 3; endfunction
endclass
class R extends Q;
  virtual function int f(int x); return super.f(x) + 1000; endfunction
  virtual task t(output int o); super.t(o); o += 5; endtask
endclass
class Drv;
  task run(P h, output int o); h.t(o); endtask
  task run2(P h, output int o); int x; h.t(x); o = x; endtask
endclass
module top;
  int s[3], s2;
  initial begin
    A a = new; int r5, r6, r7, ro;
    P arr[3]; P p = new; Q q = new; R r = new; Drv d = new;
    r5 = 1; a.t4(r5); a.wrap(r6); a.wrap2(r7); a.rec(10, ro);
    $display("T|t4=%0d wrap=%0d wrap2=%0d rec=%0d", r5, r6, r7, ro);
    arr[0] = p; arr[1] = q; arr[2] = r;
    for (int k = 0; k < 1500; k++)
      for (int j = 0; j < 3; j++) begin int o; d.run(arr[j], o); s[j] += o; d.run2(arr[j], o); s2 += o; end
    $display("T|s=%0d %0d %0d s2=%0d", s[0], s[1], s[2], s2);
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|t4=6 wrap=7 wrap2=107 rec=55",
            "T|s=46500 195000 1702500 s2=1944000"
        ]
    );
}

#[test]
fn packed_typedef_output_keeps_its_full_width() {
    if !all_policies() {
        return;
    }
    let src = r#"
typedef bit [31:0] M;
class C;
  task tk(output M [1:0] o); o = {32'h1, 32'h2}; endtask
  function void fk(output M [1:0] o); o = {32'h3, 32'h4}; endfunction
endclass
module top;
  M [1:0] v, w;
  initial begin
    C c = new;
    for (int i = 0; i < 1100; i++) begin c.tk(v); c.fk(w); end
    $display("T|tk=%h fk=%h", v, w);
  end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|tk=0000000100000002 fk=0000000300000004"]);
}

#[test]
fn type_param_output_writeback_and_members() {
    if !all_policies() {
        return;
    }
    let src = r#"
class item_c;
  int id;
  function new(int v); id = v; endfunction
endclass

class sqr_c #(type REQ = item_c);
  REQ last_item;
  function void item_done(REQ item = null);
    if (item != null) last_item = item;
  endfunction
endclass

class fifo_c #(type T = item_c);
  T stored;
  function bit try_get(output T val);
    if (stored != null) begin
      val = stored;
      return 1;
    end
    return 0;
  endfunction
  function void put_item(T val);
    stored = val;
  endfunction
endclass

class cb_iter #(type T = item_c, type CB = item_c);
  T m_obj;
  CB m_cb;
  function new(T obj = null);
    m_obj = obj;
  endfunction
  function CB set_and_get(CB cb);
    REQ t;
    m_cb = cb;
    t = m_cb;
    return t;
  endfunction
endclass

module top;
  initial begin
    sqr_c#(item_c) sqr = new();
    fifo_c#(item_c) fifo = new();
    cb_iter#(item_c, item_c) iter = new(new(100));
    item_c it1, it2, it3, out_it;
    bit ok;
    int loop_cnt = 0;

    it1 = new(735);
    it2 = new(9435);
    it3 = new(2025);

    fifo.put_item(it2);

    for (int k = 0; k < 1200; k++) begin
      sqr.item_done(it1);
      ok = fifo.try_get(out_it);
      it3 = iter.set_and_get(it1);
      loop_cnt++;
    end

    $display("T|seq=%0d tx=%0d cb=%0d cnt=%0d ok=%0d",
             sqr.last_item.id, out_it.id, it3.id, loop_cnt, ok);
  end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|seq=735 tx=9435 cb=735 cnt=1200 ok=1"]);
}

