//! §13.5.1, §7.10, §10.10: an actual bound to an unpacked-array formal is
//! the whole collection: an unpacked array concatenation `{"a", "bc"}` is a
//! two-element queue, and a returned queue keeps its elements. A compiled
//! method passes each actual of a call as one register, so a literal arrived
//! as its packed bits split into one-byte elements and a returned collection
//! arrived empty, for method, static, `super`, task, free-function and
//! constructor calls (#1257). Expected values are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
typedef string sq_t[$];
typedef int    iq_t[$];

function automatic string ff_s(string s[$]);
  return $sformatf("%0d:%s:%s", s.size(), s[0], s[$]);
endfunction
function automatic string ff_i(int s[]);
  return $sformatf("%0d:%0d:%0d", s.size(), s[0], s[s.size()-1]);
endfunction
function automatic string ff_t(sq_t s);
  return $sformatf("%0d:%s:%s", s.size(), s[0], s[$]);
endfunction
function automatic string ff_fa(int s[3]);
  return $sformatf("%0d:%0d:%0d", $size(s), s[0], s[2]);
endfunction

class base_t;
  string log[$];
  function new(string init[$] = {});
    foreach (init[i]) log.push_back(init[i]);
  endfunction
  function string show(string s[$]);
    return $sformatf("%0d:%s:%s", s.size(), s[0], s[$]);
  endfunction
  static function string sshow(string s[$]);
    return $sformatf("%0d:%s:%s", s.size(), s[0], s[$]);
  endfunction
  function string ishow(int s[$]);
    return $sformatf("%0d:%0d:%0d", s.size(), s[0], s[$]);
  endfunction
  function string tshow(sq_t s);
    return $sformatf("%0d:%s:%s", s.size(), s[0], s[$]);
  endfunction
  function string ashow(int s[string]);
    return $sformatf("%0d:%0d", s.size(), s["b"]);
  endfunction
  task tk(string s[$], output string r);
    r = $sformatf("%0d:%s:%s", s.size(), s[0], s[$]);
  endtask
endclass

class der_t extends base_t;
  string mq[$];
  function new();
    super.new({"x", "yz"});
  endfunction
  function sq_t mk();
    sq_t r = {"m", "kk"};
    return r;
  endfunction
  task run();
    string loc[$];
    string a = "p", b = "qq";
    string r;
    int iq[$];
    base_t o;
    loc = {"l1", "l2"};
    mq = {"m1", "m2", "m3"};
    r = show({"nop"});
    $display("T|lit1 %s", r);
    r = show({"a", "bc"});
    $display("T|lit2 %s", r);
    r = show({a, b});
    $display("T|vars %s", r);
    r = show(loc);
    $display("T|local %s", r);
    r = show(mq);
    $display("T|member %s", r);
    r = this.show({"t", "uu"});
    $display("T|this %s", r);
    r = super.show({"s", "vv"});
    $display("T|super %s", r);
    r = base_t::sshow({"c", "dd"});
    $display("T|static %s", r);
    r = show({loc, "z"});
    $display("T|concat %s", r);
    r = show(mk());
    $display("T|call %s", r);
    r = show(a == "p" ? loc : mq);
    $display("T|tern %s", r);
    r = ishow({1, 2, 3});
    $display("T|ints %s", r);
    r = ishow('{4, 5});
    $display("T|ipat %s", r);
    r = tshow({"e", "ff"});
    $display("T|typedef %s", r);
    r = ashow('{"a": 1, "b": 2});
    $display("T|assoc %s", r);
    tk({"k", "ll"}, r);
    r = r;
    $display("T|task %s", r);
    r = ff_s({"f", "gg"});
    $display("T|free %s", r);
    r = ff_i('{7, 8, 9});
    $display("T|freei %s", r);
    r = ff_t({"h", "ii"});
    $display("T|freet %s", r);
    r = ff_fa('{1, 2, 3});
    $display("T|freefa %s", r);
    o = new({"n1", "n2"});
    $display("T|ctor %0d %s", o.log.size(), o.log[$]);
    $display("T|superctor %0d %s", log.size(), log[$]);
    for (int i = 0; i < 2; i++) iq.push_back(i);
  endtask
endclass

module top;
  task automatic mtk(string s[$], output string r);
    r = $sformatf("%0d:%s:%s", s.size(), s[0], s[$]);
  endtask
  initial begin
    der_t d = new;
    string r;
    d.run();
    for (int i = 0; i < 2; i++) begin
      r = ff_s({"p", "qq"});
      if (i == 1) $display("T|proc free %s", r);
      mtk({"t", "uu"}, r);
      if (i == 1) $display("T|proc task %s", r);
      r = base_t::sshow({"v", "ww"});
      if (i == 1) $display("T|proc static %s", r);
    end
    $finish;
  end
endmodule
"#;

#[test]
fn compiled_call_binds_unpacked_formal_actuals_whole() {
    let _ = super::compiled_method_test_env::policies(&[("1", "0"), ("1", "1000"), ("0", "0")]);
    let sim = simulate(SRC, 1_000).expect("simulate failed");
    let out: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|m| m.starts_with("T|"))
        .collect();
    assert_eq!(
        out,
        [
            "T|lit1 1:nop:nop",
            "T|lit2 2:a:bc",
            "T|vars 2:p:qq",
            "T|local 2:l1:l2",
            "T|member 3:m1:m3",
            "T|this 2:t:uu",
            "T|super 2:s:vv",
            "T|static 2:c:dd",
            "T|concat 3:l1:z",
            "T|call 2:m:kk",
            "T|tern 2:l1:l2",
            "T|ints 3:1:3",
            "T|ipat 2:4:5",
            "T|typedef 2:e:ff",
            "T|assoc 2:2",
            "T|task 2:k:ll",
            "T|free 2:f:gg",
            "T|freei 3:7:9",
            "T|freet 2:h:ii",
            "T|freefa 3:1:3",
            "T|ctor 2 n2",
            "T|superctor 2 yz",
            "T|proc free 2:p:qq",
            "T|proc task 2:t:uu",
            "T|proc static 2:v:ww",
        ]
    );
}
