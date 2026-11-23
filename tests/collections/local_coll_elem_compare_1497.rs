//! §7.5, §7.10, §11.4.4, §11.6.1 (#1497): a comparison of two elements of a
//! LOCAL dynamic array or queue in a compiled method. A method with a loop
//! compiles on its first call, and an element read of a method-local
//! collection had no width there, so a self-determined compare sized both
//! reads at one bit: `a[0] == a[1]` over `'{3, 1}` was true and a bubble
//! sort did nothing. Signed and unsigned elements, queues, typedef'd
//! collections and a class method are covered.
//!
//! Every expectation was cross-checked against the reference simulator.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 10).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect()
}

#[test]
fn compiled_method_compares_local_collection_elements() {
    let o = lines(
        r#"
module top;
  function automatic bit f();
    int a[];
    a = '{3, 1};
    for (int i = 0; i < 1; i++);
    return a[0] == a[1];
  endfunction
  function automatic bit g();
    int a[];
    a = '{3, 1};
    return a[0] == a[1];
  endfunction
  initial $display("f=%0d g=%0d", f(), g());
endmodule
"#,
    );
    assert!(o.iter().any(|l| l == "f=0 g=0"), "{o:?}");
}

#[test]
fn local_collection_element_compares_keep_width_and_sign() {
    let o = lines(
        r#"
module top;
  typedef int iq_t[$];
  typedef byte bd_t[];
  class C;
    function automatic int m();
      int a[];
      a = '{3, 1};
      for (int i = 0; i < 1; i++);
      return a[0] > a[1];
    endfunction
  endclass
  function automatic bit f_dyn_eq();
    int a[]; a = '{3, 1};
    for (int i = 0; i < 1; i++);
    return a[0] == a[1];
  endfunction
  function automatic bit f_q_ne();
    int a[$]; a = '{3, 1};
    for (int i = 0; i < 1; i++);
    return a[0] != a[1];
  endfunction
  function automatic bit f_signed_lt();
    int a[]; a = '{-1, 1};
    for (int i = 0; i < 1; i++);
    return a[0] < a[1];
  endfunction
  function automatic bit f_unsigned_lt();
    int unsigned a[]; a = '{32'hffff_ffff, 1};
    for (int i = 0; i < 1; i++);
    return a[0] < a[1];
  endfunction
  function automatic bit f_byte_signed();
    byte a[$]; a = '{-8'sd2, 8'sd1};
    for (int i = 0; i < 1; i++);
    return a[0] < a[1];
  endfunction
  function automatic bit f_logic8();
    logic [7:0] a[]; a = '{8'h80, 8'h81};
    for (int i = 0; i < 1; i++);
    return a[0] == a[1];
  endfunction
  function automatic bit f_typedef_q();
    iq_t a; a = '{3, 1};
    for (int i = 0; i < 1; i++);
    return a[0] == a[1];
  endfunction
  function automatic bit f_typedef_byte();
    bd_t a; a = '{-8'sd2, 8'sd1};
    for (int i = 0; i < 1; i++);
    return a[0] < a[1];
  endfunction
  function automatic bit f_ternary();
    int a[]; a = '{3, 1};
    for (int i = 0; i < 1; i++);
    return (a[0] > a[1]) ? 1'b1 : 1'b0;
  endfunction
  function automatic int f_sort();
    int a[]; int t;
    a = '{5, 2, 9, 1};
    for (int i = 0; i < 4; i++)
      for (int j = 0; j < 3 - i; j++)
        if (a[j] > a[j+1]) begin t = a[j]; a[j] = a[j+1]; a[j+1] = t; end
    return a[0]*1000 + a[1]*100 + a[2]*10 + a[3];
  endfunction
  function automatic bit f_concat();
    logic [3:0] a[]; a = '{4'h3, 4'h1};
    for (int i = 0; i < 1; i++);
    return {a[0], a[1]} == 8'h31;
  endfunction
  function automatic bit f_mixed_mod();
    int a[]; a = '{3, 1};
    for (int i = 0; i < 1; i++);
    return a[0] == 3;
  endfunction
  function automatic bit f_shadow();
    int a[]; a = '{3, 1};
    for (int i = 0; i < 1; i++) begin
      bit [1:0] a;
      a = 2'b10;
      return a[1] == a[0];
    end
    return 1;
  endfunction
  initial begin
    C c = new;
    $display("T|dyn_eq=%0d q_ne=%0d slt=%0d ult=%0d byte=%0d l8=%0d", f_dyn_eq(), f_q_ne(), f_signed_lt(), f_unsigned_lt(), f_byte_signed(), f_logic8());
    $display("T|tq=%0d tb=%0d tern=%0d sort=%0d cat=%0d mix=%0d shadow=%0d cls=%0d", f_typedef_q(), f_typedef_byte(), f_ternary(), f_sort(), f_concat(), f_mixed_mod(), f_shadow(), c.m());
  end
endmodule
"#,
    );
    for want in [
        "T|dyn_eq=0 q_ne=1 slt=1 ult=0 byte=1 l8=0",
        "T|tq=0 tb=1 tern=1 sort=1259 cat=1 mix=1 shadow=0 cls=1",
    ] {
        assert!(o.iter().any(|l| l == want), "missing {want:?} in {o:?}");
    }
}
