//! `$bits(<unqualified class member>)` inside a class method must resolve to
//! the class member (per IEEE 1800-2017 §23.8 name resolution: class members
//! shadow module scope), NOT to a same-named module-scope array/typedef.
//!
//! Regression for a bug where `$bits(i)` inside a method returned the size of
//! an unrelated module-scope dynamic array also named `i` (2048) instead of the
//! `int i` class member (32). This surfaced in UVM's `uvm_packer`, where class
//! dynamic-array properties are registered in the global arrays table and
//! collided with common field names — bloating `pack_bytes` output ~16×.

use xezim::simulate;

#[test]
fn test_dollar_bits_class_member_shadows_module_array() {
    const SRC: &str = r#"
module top;
   int i[];            // module-scope dynamic array "i"
   int pass_count;
   class c;
      int i;           // class member "i" (scalar, 32 bits)
      function int query_bits();
         return $bits(i);   // must resolve to the member (32), not the array
      endfunction
   endclass
   initial begin
      c inst = new();
      inst.i = 42;
      i = new[4];      // size the module array so $bits(i[]) would be != 32
      pass_count = 0;
      if (inst.query_bits() == 32) pass_count = 1;
   end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let pc: u64 = sim
        .get_signal("pass_count")
        .unwrap_or_else(|| panic!("signal 'pass_count' not found"))
        .to_u64()
        .unwrap_or(0);
    assert_eq!(pc, 1, "$bits(class member) must shadow a same-named module array");
}
