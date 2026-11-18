//! §13.4, §13.5, §8.11: what a subroutine call owns (its formals, its locals
//! and `this`) is its own, and the combinational logic a procedural write
//! triggers is a separate evaluation (§4.5, §9.2.2.2, §10.3). A compiled
//! method, task, module function or process loop that waits in an
//! interpreted call kept its live registers in the register file the settle
//! started by that call evaluated a two-state-declined `always_comb` (and a
//! bytecode continuous assignment re-driven by a `release`) on: after the
//! call `this` was another handle, and loop counters, formals, locals and
//! held operands read stray values (#1250). Expected values are the
//! reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
module top;
  int unsigned g;
  string       text;
  int unsigned score;
  logic [31:0] fx;
  wire  [31:0] fy;
  always_comb begin
    text  = $sformatf("%0d", g);
    score = text.len() + g;
  end
  assign fy = fx + {24'd0, text.len()[7:0]};

  task automatic bump(int unsigned v);
    string s = $sformatf("%0d", v);
    g = s.atoi();
  endtask
  function automatic void bumpf(int unsigned v);
    string s = $sformatf("%0d", v);
    g = s.atoi();
  endfunction
  function automatic int poke(int v);
    string s = $sformatf("%0d", v);
    g = s.atoi();
    return v;
  endfunction
  function automatic void frel();
    string s = "x";
    force fy = 32'd77;
    g = g + 1;
    release fy;
  endfunction

  class base_t;
    int bn = 4;
    int bcnt;
    function new();
      bumpf(1);
      for (int i = 0; i < bn; i++) bcnt++;
    endfunction
  endclass
  class widget_t extends base_t;
    int n = 3;
    int cnt;
    int arr[4];
    function new();
      super.new();
      bumpf(5);
      for (int i = 0; i < n; i++) cnt++;
    endfunction
    function int count();
      int c;
      bumpf(6);
      for (int i = 0; i < n; i++) c++;
      return c;
    endfunction
    function int mix(int a, int b);
      int loc = a + b + n;
      bumpf(7);
      return loc * 1000 + a * 10 + b;
    endfunction
    function int held(int base);
      return base + poke(8) * 100 + n;
    endfunction
    function int loop_sum();
      int s = 0;
      for (int i = 1; i <= 3; i++) begin
        bumpf(10 + i);
        s += i;
      end
      return s;
    endfunction
    function int each_sum();
      int s = 0;
      for (int i = 0; i < 4; i++) arr[i] = i + 1;
      foreach (arr[i]) s += arr[i] * 10 + poke(30 + i) - 30;
      return s;
    endfunction
    task t_loop(output int r);
      int acc = 0;
      for (int i = 0; i < 3; i++) begin
        bump(20 + i);
        acc += i + n;
      end
      r = acc;
    endtask
    function int nested(int a);
      int x = a * 2;
      x += inner(a);
      bumpf(3);
      return x + n;
    endfunction
    function int inner(int b);
      int y = b + 1;
      bumpf(4);
      return y + n;
    endfunction
    static function int st(int a);
      int z = a + 1;
      bumpf(5);
      return z * 10 + a;
    endfunction
    function int rel(int a);
      int k = a + n;
      frel();
      return k * 10 + n;
    endfunction
    function string sfn(string s);
      string t = {s, "!"};
      bumpf(6);
      return {t, s};
    endfunction
  endclass

  function automatic int modfn(int a);
    int s = 0;
    for (int i = 0; i < a; i++) begin
      bumpf(40 + i);
      s += i;
    end
    return s;
  endfunction

  initial begin
    widget_t w;
    int r, lp;
    #1;
    w = new();
    $display("T|base ctor loop %0d", w.bcnt);
    $display("T|ctor loop %0d", w.cnt);
    $display("T|function loop %0d", w.count());
    $display("T|formals and local %0d", w.mix(2, 5));
    $display("T|held operand %0d", w.held(4));
    $display("T|loop counter %0d", w.loop_sum());
    $display("T|foreach %0d", w.each_sum());
    w.t_loop(r);
    $display("T|task output %0d", r);
    $display("T|nested %0d", w.nested(5));
    $display("T|static %0d", widget_t::st(3));
    $display("T|release %0d", w.rel(1));
    $display("T|string formal %s", w.sfn("ab"));
    $display("T|module function %0d", modfn(4));
    lp = 0;
    for (int i = 0; i < 3; i++) begin
      bump(60 + i);
      lp += i * 2;
    end
    $display("T|process loop %0d", lp);
    $finish;
  end
endmodule
"#;

#[test]
fn compiled_method_regs_survive_a_settle_in_a_call() {
    let _ = super::compiled_method_test_env::policies(&[("1", "0"), ("1", "1000"), ("0", "0")]);
    let sim = simulate(SRC, 1_000).expect("simulate failed");
    let out: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| m.starts_with("T|"))
        .collect();
    assert_eq!(
        out,
        [
            "T|base ctor loop 4",
            "T|ctor loop 3",
            "T|function loop 3",
            "T|formals and local 10025",
            "T|held operand 807",
            "T|loop counter 6",
            "T|foreach 106",
            "T|task output 12",
            "T|nested 22",
            "T|static 43",
            "T|release 43",
            "T|string formal ab!ab",
            "T|module function 6",
            "T|process loop 6",
        ]
    );
}
