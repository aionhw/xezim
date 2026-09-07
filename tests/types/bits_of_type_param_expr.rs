//! §20.6.2 / §6.20.3: `$bits(T)` where `T` is a TYPE PARAMETER, used in an
//! expression at run time — a continuous assign, a procedural assignment, a
//! loop bound, a replication count, a case scrutinee.
//!
//! The type binding exists only while the instance is inlined (the
//! `saved_type_binds` rail restores the bare typedef slot afterwards), so a
//! `localparam NB = $bits(T)` folded correctly during elaboration while every
//! run-time `$bits(T)` found no `T` and read 1 — silently wrong loop counts
//! and masks. The binding (override OR default) is now folded to a literal
//! while the instance's items are materialized, before the identifier could
//! be prefixed into a nonexistent signal; chained type parameters
//! (`mid #(.T(T))`) resolve through the same per-instance map.
//!
//! Function and task bodies are copied per instance through the same walker,
//! so `$bits(T)` inside a module function folds too. An instance WITHOUT
//! type parameters clears the binding, so a child's own `typedef … T` is not
//! mistaken for the parent's parameter.
//!
//! Every expected value is the reference simulator's.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn bits_of_type_param_in_runtime_expressions() {
    let o = out(r#"
module bottom #(parameter type T = logic [3:0]) (output int lp, ca, pr, lo, rep, cs);
  localparam int NB = $bits(T);
  logic [63:0] m;
  assign lp = NB;                                 // elaboration-time fold
  assign ca = $bits(T);                           // cont-assign expression
  initial begin
    int n = 0;
    pr = $bits(T);                                // procedural expression
    for (int i = 0; i < $bits(T); i++) n++;       // loop bound
    lo = n;
    m = {$bits(T){1'b1}};                         // replication count
    rep = $countones(m);
    case ($bits(T))                               // case scrutinee
      4: cs = 3; 8: cs = 1; 40: cs = 2; default: cs = 0;
    endcase
  end
endmodule
module mid #(parameter type T = logic) (output int lp, ca, pr, lo, rep, cs);
  bottom #(.T(T)) l (.lp(lp), .ca(ca), .pr(pr), .lo(lo), .rep(rep), .cs(cs));
endmodule
module par #(parameter W = 8) (output int o1, o2, o3, o4, o5, o6);
  typedef struct packed { logic [W-1:0] a; logic [W-1:0] b; } rec_t;
  mid #(.T(rec_t)) m (.lp(o1), .ca(o2), .pr(o3), .lo(o4), .rep(o5), .cs(o6));
endmodule
module tb;
  int a[6], b[6], d[6];
  par #(.W(4))  p4  (a[0],a[1],a[2],a[3],a[4],a[5]);
  par #(.W(20)) p20 (b[0],b[1],b[2],b[3],b[4],b[5]);
  bottom dflt (d[0],d[1],d[2],d[3],d[4],d[5]);   // default T = logic [3:0]
  initial begin #1;
    $display("P4 %0d %0d %0d %0d %0d %0d", a[0],a[1],a[2],a[3],a[4],a[5]);
    $display("P20 %0d %0d %0d %0d %0d %0d", b[0],b[1],b[2],b[3],b[4],b[5]);
    $display("DEF %0d %0d %0d %0d %0d %0d", d[0],d[1],d[2],d[3],d[4],d[5]);
  end
endmodule
"#);
    for expect in ["P4 8 8 8 8 8 1", "P20 40 40 40 40 40 2", "DEF 4 4 4 4 4 3"] {
        assert!(o.contains(expect), "expected `{expect}` in:\n{o}");
    }
}

#[test]
fn bits_of_type_param_inside_module_function_and_child_typedef_guard() {
    let o = out(r#"
module inner ();                              // NO type parameter; its own T
  typedef logic [1:0] T;
  int w;
  initial w = $bits(T);                       // must stay 2, not the parent's
endmodule
module leaf #(parameter type T = logic) (output int fn, tk);
  function automatic int fbits(); return $bits(T); endfunction
  task automatic tbits(output int o); o = $bits(T); endtask
  inner i ();
  initial begin fn = fbits(); tbits(tk); end
endmodule
module par #(parameter W = 8) (output int o1, o2);
  typedef struct packed { logic [W-1:0] a; logic [W-1:0] b; } rec_t;
  leaf #(.T(rec_t)) l (.fn(o1), .tk(o2));
endmodule
module tb;
  int a1, a2, b1, b2;
  par #(.W(4))  p4  (a1, a2);
  par #(.W(20)) p20 (b1, b2);
  initial begin #1;
    $display("FN %0d %0d %0d %0d INNER %0d %0d", a1, a2, b1, b2, p4.l.i.w, p20.l.i.w);
  end
endmodule
"#);
    assert!(o.contains("FN 8 8 40 40 INNER 2 2"), "function/task $bits(T) or child guard wrong:
{o}");
}

