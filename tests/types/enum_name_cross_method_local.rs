//! §6.19: an enum local's `.name()` reflection must resolve against the
//! CURRENTLY EXECUTING method's own declaration, and an enum value with no
//! matching member reflects an EMPTY name rather than a whitespace-padded
//! string or a heap-object handle name.
//!
//! In UVM, the flat per-name type maps collide across frames: an unrelated
//! method's class-typed `v` (e.g. a visitor adapter's `VISITOR v;` formal)
//! can shadow this method's `uvm_verbosity v;` local, so value 0 reflected an
//! unrelated 0-valued member and non-member values printed a padded string.
//! Assert the resolve-final behavior so both facets stay pinned.

use xezim::simulate;

fn output_of(sim: &xezim::compiler::Simulator) -> String {
    sim.output.iter().map(|o| o.message.as_str()).collect::<Vec<_>>().join("\n")
}

#[test]
fn enum_name_reflects_members_and_empty_for_unmapped_values() {
    const SRC: &str = r#"
module top;
  typedef enum { UVM_NONE=0, UVM_LOW=100, UVM_MEDIUM=200,
                 UVM_HIGH=300, UVM_FULL=400, UVM_DEBUG=500 } verb_t;
  // A larger unrelated enum that also owns a 0-valued member. If `.name()`
  // on the verb_t local fell back to a flat value match, value 0 would
  // reflect THIS member (`BIG0`) instead of `UVM_NONE`.
  typedef enum { BIG0=0, BIG1, BIG2, BIG3, BIG4, BIG5, BIG6, BIG7,
                 BIG8, BIG9, BIG10, BIG11 } big_t;

  class parser;
    function string nm(int val);
      verb_t v;
      v = verb_t'(val);
      return v.name();
    endfunction
  endclass

  initial begin
    parser p = new;
    $display("N0 [%s]",    p.nm(0));
    $display("N100 [%s]",  p.nm(100));
    $display("N400 [%s]",  p.nm(400));
    // 301 is not a declared member (UVM_HIGH == 300, UVM_DEBUG == 500): the
    // name must be EMPTY, not whitespace-padded or a heap-object name.
    $display("N301 [%s]",  p.nm(301));
    $display("N501 [%s]",  p.nm(501));
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 100).expect("sim"));
    for want in [
        "N0 [UVM_NONE]",
        "N100 [UVM_LOW]",
        "N400 [UVM_FULL]",
        "N301 []",
        "N501 []",
    ] {
        assert!(out.contains(want), "missing `{}`:\n{}", want, out);
    }
}