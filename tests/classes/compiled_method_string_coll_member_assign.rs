//! §7.10, §10.10, §6.16: a whole-collection write of a class member that is
//! a queue, dynamic, fixed-size or associative array of `string` assigns
//! every element (`q = {q, other}`, `q = other`, `obj.q = {obj.q, "x"}`).
//! A compiled method took such a member for a scalar `string` property and
//! stored the packed concatenation where no element store reads it, so the
//! member kept its old value (#1269; riscv-dv's `instr_stream = {instr_stream,
//! ...}`). Expected values are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
class item_t;
  int id;
  function new(int i); id = i; endfunction
endclass

class C;
  string q[$];
  string other[$];
  string d[];
  string od[];
  string f[2];
  string of2[2];
  string aa[int];
  string oaa[int];
  int    iq[$];
  int    ioq[$];
  item_t hq[$];
  item_t ohq[$];
  real   rq[$];
  bit [7:0] bq[$];
  static string sq[$];
  C peer;
  function void append();
    for (int i = 0; i < 1; i++) q = {q, other};
  endfunction
  function void copy_q();
    for (int i = 0; i < 1; i++) q = other;
  endfunction
  function void this_append();
    for (int i = 0; i < 1; i++) this.q = {this.q, "x"};
  endfunction
  function void peer_append();
    for (int i = 0; i < 1; i++) peer.q = {peer.q, "y"};
  endfunction
  function void prepend();
    for (int i = 0; i < 1; i++) q = {"p", q};
  endfunction
  function void dyn_ops();
    for (int i = 0; i < 1; i++) begin
      d = od;
      d = {d, "z"};
    end
  endfunction
  function void fixed_ops();
    for (int i = 0; i < 1; i++) f = of2;
  endfunction
  function void assoc_ops();
    for (int i = 0; i < 1; i++) aa = oaa;
  endfunction
  function void int_ops();
    for (int i = 0; i < 1; i++) iq = {iq, ioq};
  endfunction
  function void handle_ops();
    for (int i = 0; i < 1; i++) hq = {hq, ohq};
  endfunction
  function void real_ops();
    for (int i = 0; i < 1; i++) rq.push_back(2.5);
  endfunction
  function void byte_ops();
    for (int i = 0; i < 1; i++) bq = {bq, 8'h41};
  endfunction
  function void static_ops();
    for (int i = 0; i < 1; i++) sq = {sq, "s"};
  endfunction
  function void local_ops(output int n, output string last);
    string lq[$];
    for (int i = 0; i < 1; i++) begin
      lq = {q, "L"};
      lq = {lq, other};
    end
    n = lq.size();
    last = lq[$];
  endfunction
  function void pattern_ops();
    for (int i = 0; i < 1; i++) q = '{"pa", "pb"};
  endfunction
  function void slice_ops();
    for (int i = 0; i < 1; i++) q = q[0:1];
  endfunction
  function void clear_ops();
    for (int i = 0; i < 1; i++) other = {};
  endfunction
endclass

module top;
  task automatic check(string what, string got, string expected);
    if (got == expected) $display("PASS %s = %s", what, got);
    else                 $display("FAIL %s = %s, expected %s", what, got, expected);
  endtask
  initial begin
    automatic C c = new;
    automatic C p = new;
    int n;
    string last;
    c.peer = p;
    c.q.push_back("a");
    c.other.push_back("b");
    c.other.push_back("c");
    c.append();
    check("q = {q, other}", $sformatf("%0d %s", c.q.size(), c.q[$]), "3 c");
    c.this_append();
    check("this.q = {this.q, x}", $sformatf("%0d %s", c.q.size(), c.q[$]), "4 x");
    p.q.push_back("p0");
    c.peer_append();
    check("peer.q = {peer.q, y}", $sformatf("%0d %s", p.q.size(), p.q[$]), "2 y");
    c.prepend();
    check("q = {p, q}", $sformatf("%0d %s", c.q.size(), c.q[0]), "5 p");
    c.local_ops(n, last);
    check("local", $sformatf("%0d %s", n, last), "8 c");
    c.copy_q();
    check("q = other", $sformatf("%0d %s", c.q.size(), c.q[$]), "2 c");
    c.od = new[2];
    c.od[0] = "d0"; c.od[1] = "d1";
    c.dyn_ops();
    check("dynamic", $sformatf("%0d %s", c.d.size(), c.d[c.d.size()-1]), "3 z");
    c.of2[0] = "f0"; c.of2[1] = "f1";
    c.fixed_ops();
    check("fixed", $sformatf("%s %s", c.f[0], c.f[1]), "f0 f1");
    c.oaa[3] = "a3"; c.oaa[9] = "a9";
    c.assoc_ops();
    check("assoc", $sformatf("%0d %s", c.aa.num(), c.aa[9]), "2 a9");
    c.iq.push_back(1); c.ioq.push_back(2); c.ioq.push_back(3);
    c.int_ops();
    check("int", $sformatf("%0d %0d", c.iq.size(), c.iq[$]), "3 3");
    begin item_t it = new(5); c.ohq.push_back(it); end
    c.handle_ops();
    check("handle", $sformatf("%0d %0d", c.hq.size(), c.hq[$].id), "1 5");
    c.real_ops();
    check("real", $sformatf("%0d %.1f", c.rq.size(), c.rq[$]), "1 2.5");
    c.byte_ops();
    check("byte", $sformatf("%0d %h", c.bq.size(), c.bq[$]), "1 41");
    c.static_ops();
    check("static", $sformatf("%0d %s", C::sq.size(), C::sq[$]), "1 s");
    c.pattern_ops();
    check("pattern", $sformatf("%0d %s", c.q.size(), c.q[$]), "2 pb");
    c.q.push_back("pc");
    c.slice_ops();
    check("slice", $sformatf("%0d %s", c.q.size(), c.q[$]), "2 pb");
    c.clear_ops();
    check("clear", $sformatf("%0d", c.other.size()), "0");
    $finish;
  end
endmodule
"#;

#[test]
fn compiled_method_string_collection_member_whole_writes() {
    let _ = super::compiled_method_test_env::policies(&[("1", "0"), ("1", "1000"), ("0", "0")]);
    let sim = simulate(SRC, 1_000).expect("simulate failed");
    let out: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| m.starts_with("PASS") || m.starts_with("FAIL"))
        .collect();
    assert_eq!(
        out,
        [
            "PASS q = {q, other} = 3 c",
            "PASS this.q = {this.q, x} = 4 x",
            "PASS peer.q = {peer.q, y} = 2 y",
            "PASS q = {p, q} = 5 p",
            "PASS local = 8 c",
            "PASS q = other = 2 c",
            "PASS dynamic = 3 z",
            "PASS fixed = f0 f1",
            "PASS assoc = 2 a9",
            "PASS int = 3 3",
            "PASS handle = 1 5",
            "PASS real = 1 2.5",
            "PASS byte = 1 41",
            "PASS static = 1 s",
            "PASS pattern = 2 pb",
            "PASS slice = 2 pb",
            "PASS clear = 0",
        ]
    );
}
