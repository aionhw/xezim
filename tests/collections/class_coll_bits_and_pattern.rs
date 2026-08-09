//! §20.6.2 `$bits` of a class-collection ELEMENT reports the DECLARED element
//! width, not the runtime value's stored width — UVM's pack/unpack macros size
//! each assoc element by `$bits`, so a `shortint aa[shortint]` whose elements
//! were stored as 32-bit ints must still pack at 16 bits. Also pins the
//! whole-assignment pattern `obj.q = '{...}` to a class-collection property
//! from OUTSIDE the class (a `MemberAccess` lvalue the bare-Ident aggregate
//! path misses), which must populate the per-instance `<handle>#<member>`
//! elements and set `.size`.

use xezim::simulate;

fn sim_src(src: &str) -> Vec<String> {
    let sim = simulate(src, 200).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

const BITS_AND_VALUE_PATTERN: &str = r#"
module top;
  class C;
    shortint aa[shortint];   // 16-bit assoc value
    byte q[$];               // queue
    function void set_aa(int k, int v);
      aa[k] = v;
    endfunction
    function void show_aa();
      $display("BITS_AA %0d", $bits(aa[1]));       // declared 16, not stored 32
    endfunction
  endclass
  C c;
  initial begin
    c = new;
    c.set_aa(1, 'h12345678);   // stored int, but $bits must still be 16
    c.show_aa();
    if ($bits(c.aa[1]) == 16) $display("TAG_PASS_BITS"); else $display("TAG_FAIL_BITS %0d", $bits(c.aa[1]));
    // whole-assignment pattern to a queue from OUTSIDE the class:
    c.q = '{1, 2, 3};
    if (c.q.size() == 3 && c.q[0] == 1 && c.q[2] == 3) $display("TAG_PASS_Q"); else $display("TAG_FAIL_Q size=%0d", c.q.size());
  end
endmodule
"#;

#[test]
fn dollar_bits_class_assoc_element_width() {
    let msgs = sim_src(BITS_AND_VALUE_PATTERN);
    assert!(
        msgs.iter().any(|m| m == "BITS_AA 16"),
        "expected $bits(assoc value) == declared 16, got: {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS_BITS"),
        "expected $bits(assoc elem) == declared width, got: {:?}",
        msgs
    );
}

#[test]
fn class_queue_pattern_assign_from_outside() {
    let msgs = sim_src(BITS_AND_VALUE_PATTERN);
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS_Q"),
        "expected outside `obj.q = '{{...}}` to size and populate, got: {:?}",
        msgs
    );
}