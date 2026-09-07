//! §6.19.6 enum reflection (`.name()`, `.first()`, `.num()`, `next/prev`) on
//! variables whose enum typedef is declared in a SUBMODULE.
//!
//! Submodule enum typedefs never reach `process_typedef`, so `enum_members`
//! had no entry for them: `.num()` read 0, `.first()` was empty, and
//! `.name()` fell to a design-wide BY-VALUE scan that answers with whichever
//! largest enum holds the value — a sibling package's `PB` for our `SB`, or
//! hash-order luck between two same-sized enums. Three pieces:
//! 1. the instance path registers the member list under the INSTANCE-scoped
//!    typedef key (bare = first-wins fallback only), and a submodule
//!    variable's `type_name` is scoped to match, so two modules that both
//!    declare `state_e` with different members each resolve their own;
//! 2. an ANONYMOUS enum variable in a submodule registers its list under
//!    its own (scoped) name, like the top-level path already did;
//! 3. `.name()` gained the variable-keyed fallback that `first/num` already
//!    had, and an inline-enum LOCAL (a typedef'd local arrives folded to its
//!    enum type) registers its list under its own name — so task locals
//!    resolve by list, not by luck.
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
fn submodule_enum_reflection_resolves_the_right_list() {
    let o = out(r#"
package pk; typedef enum logic [1:0] { PA = 1, PB = 2 } p_t; endpackage
module sub ();
  typedef enum logic [1:0] { SA = 1, SB = 2 } s_t;
  s_t s;
  function automatic string nm(s_t v); return v.name(); endfunction
  initial begin s = SB; #1 $display("SUB name=%s fn=%s first=%s num=%0d", s.name(), nm(SA), s.first().name(), s.num()); end
endmodule
module tb;
  import pk::*;
  typedef enum logic [1:0] { TA = 1, TB = 2 } t_t;
  t_t t; p_t p;
  sub u ();
  initial begin t = TB; p = PA; #1 $display("TOP name=%s pkg=%s next=%s", t.name(), p.name(), t.next().name()); end
endmodule
"#);
    for expect in ["SUB name=SB fn=SA first=SA num=2", "TOP name=TB pkg=PA next=TA"] {
        assert!(o.contains(expect), "expected `{expect}` in:\n{o}");
    }
}

#[test]
fn same_named_enums_anonymous_enums_and_task_locals() {
    let o = out(r#"
module ma (output logic [1:0] o);
  typedef enum logic [1:0] { IDLE = 0, RUN = 1, DONE = 2 } state_e;
  state_e st;
  initial begin st = RUN; o = st; #1 $display("MA name=%s next=%s prev=%s last=%s", st.name(), st.next().name(), st.prev().name(), st.last().name()); end
endmodule
module mb (input logic [1:0] i);
  typedef enum logic [1:0] { IDLE = 0, BUSY = 1, HALT = 2 } state_e;   // same name, other members
  state_e st, loc_out;
  task automatic t(); state_e l; l = HALT; loc_out = l; $display("MB task-local name=%s", l.name()); endtask
  initial begin st = state_e'(i); #1 t(); $display("MB name=%s cast=%s num=%0d", st.name(), loc_out.name(), st.num()); end
  enum logic [1:0] { AX = 0, AY = 1 } anon;
  initial begin anon = AY; #2 $display("MB anon=%s", anon.name()); end
endmodule
module tb;
  logic [1:0] w;
  ma a (.o(w));
  mb b (.i(w));
  initial #5 $finish;
endmodule
"#);
    for expect in [
        "MA name=RUN next=DONE prev=IDLE last=DONE",
        "MB task-local name=HALT",
        "MB name=BUSY cast=HALT num=3",
        "MB anon=AY",
    ] {
        assert!(o.contains(expect), "expected `{expect}` in:\n{o}");
    }
}
