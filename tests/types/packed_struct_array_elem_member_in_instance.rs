//! §7.2.1 / §7.4.5 / §23.10: a member of an element of a PACKED array of packed
//! structs (`par[i].a`) inside an instantiated module whose array dimension
//! is that module's parameter (`s_t [N-1:0] par`). Element `i` is bits
//! `[6*i+5 : 6*i]` and the first-declared 1-bit member is its MSB. The select
//! resolver sized the declaration against the top-level parameter table,
//! where the child's `N` is keyed `u.N`; the shape collapsed to a 1-bit
//! element and `par[i].a` read and wrote bit `i` of the whole array (#816).
//! Expected values agree with the reference simulator.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_line(msgs: &[String], want: &str) {
    assert!(
        msgs.iter().any(|m| m == want),
        "missing {want:?} in {msgs:?}"
    );
}

const LEAF: &str = "module leaf;
  localparam int N = 2;
  typedef struct packed { logic a; logic [3:0] d; logic u; } s_t;
  typedef struct packed { logic u; logic [3:0] d; logic a; } r_t;
  s_t [N-1:0] par;
  r_t [N-1:0] rev;
  s_t [N-1:0] par_c;
  assign par_c[1].a = 1'b1;
  assign par_c[1].d = 4'b0000;
  assign par_c[1].u = 1'b0;
  assign par_c[0]   = '0;
  initial begin
    par = '0; par[0].a = 1'b1; $display(\"w a0 %b\", par);
    par = '0; par[1].a = 1'b1; $display(\"w a1 %b\", par);
    par = '0; par[1].d = 4'b1010; $display(\"w d1 %b\", par);
    par = '0; par[1].u = 1'b1; $display(\"w u1 %b\", par);
    rev = '0; rev[1].u = 1'b1; $display(\"w rev u1 %b\", rev);
    par = 12'b100000_000001;
    $display(\"r a1=%b a0=%b u0=%b d1=%b e1=%b bits=%0d\", par[1].a, par[0].a, par[0].u, par[1].d, par[1], $bits(par[1].a));
    for (int i = 0; i < 2; i++) begin
      par = '0; par[i].a = 1'b1; $display(\"loop a%0d %b\", i, par);
    end
    #1 $display(\"c %b\", par_c);
  end
endmodule";

#[test]
fn elem_member_select_uses_instance_dimension() {
    let msgs = messages(&format!("{LEAF}\nmodule top; leaf u(); endmodule"));
    assert_line(&msgs, "w a0 000000100000");
    assert_line(&msgs, "w a1 100000000000");
    assert_line(&msgs, "w d1 010100000000");
    assert_line(&msgs, "w u1 000001000000");
    assert_line(&msgs, "w rev u1 100000000000");
    assert_line(&msgs, "r a1=1 a0=0 u0=1 d1=0000 e1=100000 bits=1");
    assert_line(&msgs, "loop a0 000000100000");
    assert_line(&msgs, "loop a1 100000000000");
    assert_line(&msgs, "c 100000000000");
}

#[test]
fn elem_member_select_ignores_same_named_top_param_and_nests() {
    // The top's `N` is 3, the leaf's 2: the leaf's own binding sizes `par`.
    let msgs = messages(&format!(
        "{LEAF}\nmodule mid; leaf v(); endmodule\nmodule top; localparam int N = 3; mid u(); endmodule"
    ));
    assert_line(&msgs, "w a1 100000000000");
    assert_line(&msgs, "w rev u1 100000000000");
    assert_line(&msgs, "r a1=1 a0=0 u0=1 d1=0000 e1=100000 bits=1");
    assert_line(&msgs, "loop a1 100000000000");
    assert_line(&msgs, "c 100000000000");
}
