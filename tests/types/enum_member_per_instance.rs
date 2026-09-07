//! §6.19/§23.6: an enum member whose VALUE depends on the module's own
//! parameters (`MAGIC = V`) must be per-instance.
//!
//! Members were registered under their bare names only, first-declared wins
//! (deliberately — a submodule's member must not clobber a sibling's
//! same-named member, the UVM `IDLE` shape). But with nothing scoped to point
//! at, every later differently-parameterized instance read the FIRST
//! instance's value: `e = MAGIC` loaded it at run time, and the cont-assign
//! const-folder baked it in at elaboration. Three pieces close it — members
//! now also register under the instance-scoped key, they join the instance's
//! local parameter environment (the fixpoint the localparams use), and they
//! join `local_names` so inlined expressions are rewritten to the scoped key.
//! The bare slot keeps its first-wins fallback: the shadowing test below is
//! the shape the old comment protected.
//!
//! Expected values are the reference simulator's.

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
fn parameterized_member_values_are_per_instance() {
    let o = out(r#"
module m #(parameter int V = 5) (output int o, output int p, output int hit,
                                 output logic [31:0] dimv);
  typedef enum int { MAGIC = V, OTHER = V + 1 } e_t;
  e_t e;
  logic [OTHER:0] wide;              // member in a dimension
  int h;
  initial begin
    e = MAGIC;                       // run-time load
    case (e_t'(V + 1))               // member as a case arm
      MAGIC:   h = 1;
      OTHER:   h = 2;
      default: h = 0;
    endcase
    wide = '1;
  end
  assign o = e;
  assign p = OTHER;                  // elaboration-time const fold
  assign hit = h;
  assign dimv = wide;
endmodule
module tb;
  m #(.V(5)) a ();
  m #(.V(9)) b ();
  initial begin #1;
    $display("EN a=%0d/%0d/%0d/%08x b=%0d/%0d/%0d/%08x",
             a.o, a.p, a.hit, a.dimv, b.o, b.p, b.hit, b.dimv);
  end
endmodule
"#);
    assert!(
        o.contains("EN a=5/6/2/0000007f b=9/10/2/000007ff"),
        "per-instance enum member values wrong:\n{o}"
    );
}

#[test]
fn a_same_named_member_in_a_sibling_still_shadows_correctly() {
    // The protection the bare first-wins slot exists for: two sibling
    // modules declare `IDLE` with DIFFERENT values; each must see its own.
    let o = out(r#"
module producer ();
  typedef enum int { IDLE = 0, BUSY = 1 } st_t;
  st_t s;
  initial s = BUSY;
endmodule
module consumer (output int o);
  typedef enum int { INIT = 0, IDLE = 1, RUN = 2 } cs_t;
  cs_t c;
  initial c = IDLE;
  assign o = c;
endmodule
module tb;
  producer p ();
  consumer q ();
  initial begin #1; $display("SHAD q=%0d", q.o); end
endmodule
"#);
    assert!(o.contains("SHAD q=1"), "sibling member shadowing broke:\n{o}");
}
