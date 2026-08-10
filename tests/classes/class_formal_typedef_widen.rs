//! Class-method formal of a TYPEDEF'd integral type adopts the typedef's
//! width, sign-extending a narrower actual.
//!
//! §6.18 / §13.5.1: a formal `input i64_t x` (where `i64_t` is a packed
//! 64-bit typedef) is a VARIABLE of that type, so binding a 32-bit `int`
//! actual (e.g. `-3`) must sign-extend it to 64 bits. Before this fix a
//! class-method formal declared via a `typedef` bound the 32-bit actual
//! as-is and read x in the high half — which is exactly how
//! `uvm_packer::pack_field_int(uvm_integral_t value, int size)` corrupted
//! packed `$realtobits(shortreal)` and negative values passed to a 64-bit
//! packer formal (UVM 03data pack/record group).

use xezim::simulate;

fn result(sim: &xezim::compiler::Simulator) -> u64 {
    let msg = sim
        .output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with("RESULT="))
        .expect("no RESULT line");
    let raw: String = msg
        // Everything after "RESULT=" (strip a possible 0x prefix and any
        // "%x" padding) is the hex value; keep only hex digits.
        .trim_start_matches("RESULT=")
        .trim_start_matches("0x")
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect();
    assert!(raw.len() <= 16, "unexpected hex width in RESULT: {}", msg);
    u64::from_str_radix(&raw, 16).expect("result is not a valid u64")
}

fn tag(sim: &xezim::compiler::Simulator) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with("TAG_"))
        .unwrap_or_else(|| panic!("no TAG line"))
}

#[test]
fn typedef_formal_sign_extends_class_method_arg() {
    const SRC: &str = r#"
module top;
  typedef logic signed [63:0] i64t;
  class c;
    function automatic i64t m(input i64t x); return x; endfunction
  endclass
  int vv;
  i64t r;
  c cc;
  initial begin
    cc = new;
    vv = -3;
    r = cc.m(vv);
    $display("RESULT=%x", r);
    if (r === -3) $display("TAG_PASS"); else $display("TAG_FAIL");
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    // The 64-bit sign-extended value of -3 is 0xfffffffffffffffd.
    assert_eq!(
        result(&sim),
        u64::from_str_radix("fffffffffffffffd", 16).unwrap(),
        "class-method formal of a 64-bit typedef must sign-extend a 32-bit negative actual"
    );
    assert!(tag(&sim) == "TAG_PASS", "must round-trip the sign-extended value");
}