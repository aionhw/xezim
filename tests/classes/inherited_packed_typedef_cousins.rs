//! IEEE 1800-2023 §6.18, §8.13, §8.25, §13.5.1: inherited and parameterized
//! class-scoped packed typedefs and packed structs retain their declaring-scope
//! widths and field offsets across properties, arrays, queues, method/task
//! `input`/`output`/`ref` formals, and specialized scope variables.
use xezim::simulate;

#[test]
fn inherited_packed_typedef_properties_structs_and_formals() {
    if !super::compiled_method_test_env::policies(&[("0", "1000"), ("1", "0")]) {
        return;
    }
    let sim = simulate(
        r#"
module top;
  class base;
    parameter WIDTH = 128;
    typedef logic [WIDTH-1:0] word_t;
    typedef struct packed {
      logic [WIDTH-1:0] data;
      logic [7:0] tag;
    } pkt_t;
    virtual function pkt_t echo_pkt(pkt_t p); return p; endfunction
    virtual task t_run(input word_t a, output word_t b); b = a; endtask
    virtual function word_t bump_ref(ref word_t r);
      word_t old = r;
      r = r ^ 128'hff00;
      return old;
    endfunction
  endclass

  class derived #(type T = byte) extends base;
    word_t prop = '1;
    word_t arr[2] = '{'1, 128'h1234_5678_9abc};
    word_t q[$] = '{128'hdead_beef_cafe};
    pkt_t pk;
    virtual function pkt_t echo_pkt(pkt_t p);
      $display("t2=%0d,%0h,%0h", $bits(p), p.tag, p.data);
      return p;
    endfunction
    virtual task t_run(input word_t a, output word_t b);
      b = a ^ 128'hff;
    endtask
    virtual function word_t bump_ref(ref word_t r);
      $display("tref=%0d,%0h", $bits(r), r);
      r = r ^ 128'h00ff;
      return r;
    endfunction
  endclass

  class pbase #(int W = 64);
    typedef logic [W-1:0] pw_t;
    typedef struct packed {
      logic [W-1:0] data;
      logic [7:0] tag;
    } ppkt_t;
    virtual function pw_t pass(pw_t a); return a; endfunction
    virtual function ppkt_t pass_pkt(ppkt_t p); return p; endfunction
  endclass

  class pderived #(int W = 64) extends pbase #(W);
    virtual function pw_t pass(pw_t a);
      pw_t loc = a;
      $display("p=%0d,%0d,%0d,%0h", W, $bits(a), $bits(loc), loc);
      return loc;
    endfunction
    virtual function ppkt_t pass_pkt(ppkt_t p);
      ppkt_t lp = p;
      $display("pp=%0d,%0d,%0d,%0h,%0h", W, $bits(p), $bits(lp), lp.tag, lp.data);
      return lp;
    endfunction
  endclass

  initial begin
    derived #() d = new;
    base b = d;
    pderived #(96) pd96 = new;
    pbase #(96) pb96 = pd96;
    pderived #(48) pd48 = new;
    pbase #(48) pb48 = pd48;
    pbase #(96)::pw_t pv = 96'h1234_5678_9abc_def0;
    base::word_t out_w;
    base::word_t rw = 128'h8000_0000_0000_0000_0000_0000_0000_1200;
    base::pkt_t pin, pout;
    pbase #(96)::ppkt_t p96in, p96out;
    pbase #(48)::ppkt_t p48in, p48out;

    d.pk.data = 128'h5566_7788_99aa;
    d.pk.tag = 8'h3c;
    $display("cols=%0d,%0h,%0h,%0h,%0h",
      $bits(d.prop), d.prop[15:0], d.arr[0][15:0], d.arr[1], d.q[0]);
    $display("pk=%0d,%0h,%0h", $bits(d.pk), d.pk.tag, d.pk.data);

    pin.data = 128'h1122_3344_5566;
    pin.tag = 8'ha5;
    pout = b.echo_pkt(pin);
    $display("pout=%0h,%0h", pout.tag, pout.data);

    b.t_run(128'h55aa_00ff, out_w);
    $display("trun=%0h,%0h", out_w, b.bump_ref(rw));
    $display("rw_after=%0h", rw);

    $display("pv=%0d,%0h,%0h", $bits(pv), pv, pb96.pass(96'h1234_5678_9abc));
    p96in.data = 96'h1122_3344_5566_7788_99aa;
    p96in.tag = 8'h5a;
    p96out = pb96.pass_pkt(p96in);
    p48in.data = 48'h1234_5678_9abc;
    p48in.tag = 8'hc3;
    p48out = pb48.pass_pkt(p48in);
    $display("pout96=%0h,%0h pout48=%0h,%0h",
      p96out.tag, p96out.data, p48out.tag, p48out.data);
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(
        lines.contains(&"cols=128,ffff,ffff,123456789abc,deadbeefcafe"),
        "{lines:?}"
    );
    assert!(lines.contains(&"pk=136,3c,5566778899aa"), "{lines:?}");
    assert!(lines.contains(&"t2=136,a5,112233445566"), "{lines:?}");
    assert!(lines.contains(&"pout=a5,112233445566"), "{lines:?}");
    assert!(
        lines.contains(&"tref=128,80000000000000000000000000001200"),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"trun=55aa0000,800000000000000000000000000012ff"),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"rw_after=800000000000000000000000000012ff"),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"pv=96,123456789abcdef0,123456789abc"),
        "{lines:?}"
    );
    assert!(lines.contains(&"p=96,96,96,123456789abc"), "{lines:?}");
    assert!(
        lines.contains(&"pp=96,104,104,5a,112233445566778899aa"),
        "{lines:?}"
    );
    assert!(lines.contains(&"pp=48,56,56,c3,123456789abc"), "{lines:?}");
    assert!(
        lines.contains(&"pout96=5a,112233445566778899aa pout48=c3,123456789abc"),
        "{lines:?}"
    );
}

/// §6.18 and §8.25: the typedef uses the parameters of the specialization
/// that declared it, including an expression in the extends clause.
#[test]
fn inherited_packed_typedef_uses_the_declaring_specialization() {
    let sim = simulate(
        r#"
module top;
  class Base #(int W = 4);
    typedef struct packed { bit [W-1:0] payload; bit flag; } word;
  endclass
  class Child #(int N = 3) extends Base#(N+2);
    word member;
    function int check(output word out);
      word local_value = '1;
      out = local_value;
      return $bits(member) * 1000 + $bits(local_value) * 10 + $bits(out);
    endfunction
  endclass
  typedef Child#(8) C;
  initial begin
    C c = new;
    Base#(10)::word o;
    Child#(8)::word s;
    int sizes;
    sizes = c.check(o);
    s = '1;
    $display("sizes=%0d out=%0h scope=%0d %0h", sizes, o, $bits(s), s.payload);
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert_eq!(lines, ["sizes=11121 out=7ff scope=11 3ff"]);
}
