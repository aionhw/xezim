//! UVM 00no_macros / 4030 root cause (part 2): `$bits(<builtin type name>)` in
//! a packed dimension must resolve to the intrinsic width. UVM's
//! `uvm_pack_aa_int_intN(VAR, SIZE)` declares `bit[SIZE-1:0] __index__` with
//! `SIZE` lowered to `$bits(shortint)` = 16. `const_eval_i64_with_params` only
//! recognized parameter idents and typedefs for a `$bits(...)` call, so
//! `$bits(shortint)` fell through to `None`, the dimension collapsed to ONE
//! BIT, and the assoc key was truncated on pack/unpack. The reference returns
//! 16; this asserts xezim now does the same for the exact UVM macro shape.

use std::process::Command;

#[test]
fn bits_of_builtin_type_width() {
    let src = r#"
module top;
  class C;
    function void go();
      // The UVM shape: bit[SIZE-1:0] where SIZE = $bits(shortint) -> [15:0] = 16
      bit[$bits(shortint)-1:0] key;
      $display("KW %0d", $bits(key));       // 16
      key = 16'hBEA4;
      $display("KR %04h", key);             // bea4
    endfunction
  endclass
  initial begin
    $display("BB %0d", $bits(byte));        // 8
    $display("BS %0d", $bits(shortint));    // 16
    $display("BI %0d", $bits(int));         // 32
    $display("BL %0d", $bits(longint));     // 64
    $display("BR %0d", $bits(real));        // 64
    $display("BSR %0d", $bits(shortreal));  // 32
    C c = new; c.go();
  end
endmodule
"#;
    let dir = std::env::temp_dir().join(format!("xezim_bits_builtin_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sv_path = dir.join("bits_builtin.sv");
    std::fs::write(&sv_path, src).unwrap();

    let bin = env!("CARGO_BIN_EXE_xezim");
    let out = Command::new(bin)
        .arg("--simulate").arg("-s").arg("top")
        .arg(sv_path.to_str().unwrap())
        .output().expect("failed to run xezim");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("BB 8"),   "$bits(byte) must be 8.\n{combined}");
    assert!(stdout.contains("BS 16"),  "$bits(shortint) must be 16.\n{combined}");
    assert!(stdout.contains("BI 32"),  "$bits(int) must be 32.\n{combined}");
    assert!(stdout.contains("BL 64"),  "$bits(longint) must be 64.\n{combined}");
    assert!(stdout.contains("BR 64"),  "$bits(real) must be 64.\n{combined}");
    assert!(stdout.contains("BSR 32"), "$bits(shortreal) must be 32.\n{combined}");
    assert!(stdout.contains("KW 16"),  "bit[$bits(shortint)-1:0] must be 16 bits.\n{combined}");
    assert!(stdout.contains("KR bea4"),"16-bit local must round-trip 0xBEA4.\n{combined}");
}