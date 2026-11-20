//! §16.8, §16.12 (#1309): a formal argument of a named sequence or property may declare a
//! default actual argument, and an instance provides an actual only for the formals without
//! one. The parser dropped the defaults, so a formal the instance left out stayed unbound in
//! the expanded body. A default is now kept and takes the place of a formal the instance
//! omits: in an empty or shorter argument list, in an instance without an argument list,
//! beside arguments bound by name (`.y(b)`) or left empty (`p(, b)`), which were not bound
//! either. A default may name an earlier formal. The default of an untyped formal is cast to
//! its self-determined type before it is substituted.

fn check(source: &str, expected: &[&str]) {
    let actual: Vec<_> = xezim::simulate(source, 1_000_000)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(actual, expected);
}

/// The example of the report: `p()` is `p(a, b)`.
#[test]
fn empty_argument_list_takes_the_defaults() {
    check(
        r#"
`timescale 1ns/1ns
module top;
  bit clk=0,a=1,b=0;
  int control=0,defaults=0;
  always #5 clk=~clk;
  property p(x=a,y=b); @(posedge clk) x |-> y; endproperty
  assert property(p(a,b)) else control++;
  assert property(p()) else defaults++;
  initial begin
    #16;
    $display("T|control=%0d defaults=%0d",control,defaults);
    $finish;
  end
endmodule
"#,
        &["T|control=2 defaults=2"],
    );
}

/// Named properties and sequences of a module instance, an interface, a package (imported
/// and package-qualified) and a generate loop, with every way of leaving an actual out.
#[test]
fn defaults_bind_in_every_declaration_scope_and_argument_form() {
    check(
        r#"
`timescale 1ns/1ns
package pk;
  bit pa = 1, pb = 0;
  property pp(x = pa, y = pb); x |-> y; endproperty
  sequence ps(x = pa, y = 1'b1); x ##1 y; endsequence
endpackage
interface ifc(input bit clk);
  bit a = 1, b = 0;
  int n_dflt = 0, n_seq = 0;
  property p(x = a, y = b); @(posedge clk) x |-> y; endproperty
  sequence s(x = a, y = !b); x ##1 y; endsequence
  assert property (p) else n_dflt++;
  cover property (@(posedge clk) s) n_seq++;
endinterface
module sub(input bit clk);
  bit a = 1, b = 0;
  int n_all = 0, n_one = 0, n_named = 0, n_skip = 0, n_noparen = 0, n_chain = 0, n_seq = 0, n_seqd = 0;
  property p(x = a, y = b); @(posedge clk) x |-> y; endproperty
  property q(x, y = x); @(posedge clk) x |-> !y; endproperty
  sequence s(x = a, y = !b); x ##1 y; endsequence
  assert property (p()) else n_all++;
  assert property (p(a)) else n_one++;
  assert property (p(.y(b))) else n_named++;
  assert property (p(, b)) else n_skip++;
  assert property (p) else n_noparen++;
  assert property (q(a)) else n_chain++;
  cover property (@(posedge clk) s()) n_seq++;
  cover property (@(posedge clk) s(.y(b))) n_seqd++;
endmodule
module top;
  import pk::*;
  bit clk = 0;
  always #5 clk = ~clk;
  int n_pkg = 0, n_pkgq = 0, n_pseq = 0;
  sub u(clk);
  ifc i(clk);
  assert property (@(posedge clk) pp()) else n_pkg++;
  assert property (@(posedge clk) pk::pp(.x(pa))) else n_pkgq++;
  cover property (@(posedge clk) ps()) n_pseq++;
  for (genvar g = 0; g < 2; g++) begin : gl
    bit a = 1, b = (g == 1);
    int n = 0;
    property p(x = a, y = b); @(posedge clk) x |-> y; endproperty
    assert property (p()) else n++;
  end
  initial begin
    #36;
    $display("T|sub all=%0d one=%0d named=%0d skip=%0d noparen=%0d chain=%0d seq=%0d seqd=%0d",
             u.n_all, u.n_one, u.n_named, u.n_skip, u.n_noparen, u.n_chain, u.n_seq, u.n_seqd);
    $display("T|ifc dflt=%0d seq=%0d", i.n_dflt, i.n_seq);
    $display("T|pkg=%0d pkgq=%0d pseq=%0d gen0=%0d gen1=%0d", n_pkg, n_pkgq, n_pseq, gl[0].n, gl[1].n);
    $finish;
  end
endmodule
"#,
        &[
            "T|sub all=4 one=4 named=4 skip=4 noparen=4 chain=4 seq=3 seqd=0",
            "T|ifc dflt=4 seq=3",
            "T|pkg=4 pkgq=4 pseq=3 gen0=4 gen1=0",
        ],
    );
}

/// §16.8: `~v`, `c ? ~v : v` and `'1` for an untyped formal keep their own width in
/// `x == 8'hff`; a typed formal gives the default its type.
#[test]
fn untyped_default_is_self_determined() {
    check(
        r#"
`timescale 1ns/1ns
module top;
  bit clk = 0; always #5 clk = ~clk;
  logic [3:0] v = 4'h0; bit c = 1;
  int n_inv_d = 0, n_neg_d = 0, n_cond_d = 0, n_fill_d = 0, n_typed = 0;
  property p_inv(x = ~v); @(posedge clk) x == 8'hff; endproperty
  property p_neg(x = -v); @(posedge clk) (x + 8'd1) == 8'd1; endproperty
  property p_cond(x = c ? ~v : v); @(posedge clk) x == 8'hff; endproperty
  property p_fill(x = '1); @(posedge clk) x == 8'hff; endproperty
  property p_typed(logic [7:0] x = ~v); @(posedge clk) x == 8'hff; endproperty
  assert property (p_inv()) else n_inv_d++;
  assert property (p_neg()) else n_neg_d++;
  assert property (p_cond()) else n_cond_d++;
  assert property (p_fill()) else n_fill_d++;
  assert property (p_typed()) else n_typed++;
  initial begin
    #26;
    $display("T|inv=%0d neg=%0d cond=%0d fill=%0d typed=%0d", n_inv_d, n_neg_d, n_cond_d, n_fill_d, n_typed);
    $finish;
  end
endmodule
"#,
        &["T|inv=3 neg=0 cond=3 fill=3 typed=0"],
    );
}
