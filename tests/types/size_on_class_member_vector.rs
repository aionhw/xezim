//! §20.7: `$size`, `$left`, `$high`, ... on a class-member packed vector
//! must report the declared bit width. A `bit[88:0] big` field lives only in
//! the class member tables (not the compact signal/array tables), so the
//! array-query handler's width lookup returned `None` and collapsed to 0 —
//! while `$bits(big)` correctly reported 89. That 0 made `uvm_pack_intN(field,
//! $size(field))` pack nothing (UVM 4572_longpacking_1 `ETHL_PKT_MISMATCH`).
//! Regression: `$size` must equal `$bits` for a class-member vector, both at
//! the call site (`o.big`) and inside a class method (`$size(big)`).

use std::process::Command;

#[test]
fn size_on_class_member_vector_reports_width() {
    let src = r#"
class C;
  bit[88:0] big;   // 89-bit, non-byte-aligned: the case that broke
  bit[63:0] b64;
  bit[7:0]  b8;
  function int ret_sz();  return $size(big);  endfunction
  function int ret_hi();  return $high(big);  endfunction
  function int ret_low(); return $low(big);   endfunction
endclass

module top;
  C c;
  initial begin
    c = new;
    $display("SZB  %0d", $size(c.big));          // 89
    $display("SZ8  %0d", $size(c.b8));           // 8
    $display("SZ64 %0d", $size(c.b64));          // 64
    $display("MTH  %0d", c.ret_sz());            // 89 (inside a class method)
    $display("HI   %0d", c.ret_hi());            // 88
    $display("LO   %0d", c.ret_low());           // 0
    c.big = 89'h1_2345_6789_ABCDE_F;
    $display("EQ   %0d", ($size(c.big) == $bits(c.big)) ? 1 : 0); // 1
  end
endmodule
"#;

    let dir = std::env::temp_dir().join(format!("xezim_sz_member_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sv_path = dir.join("size_on_class_member_vector.sv");
    std::fs::write(&sv_path, src).unwrap();

    let bin = env!("CARGO_BIN_EXE_xezim");
    let out = Command::new(bin)
        .arg("--simulate")
        .arg("-s")
        .arg("top")
        .arg(sv_path.to_str().unwrap())
        .output()
        .expect("failed to run xezim");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("SZB  89"), "89-bit class field $size must be 89.\n{combined}");
    assert!(stdout.contains("SZ8  8"),  "8-bit class field $size must be 8.\n{combined}");
    assert!(stdout.contains("SZ64 64"), "64-bit class field $size must be 64.\n{combined}");
    assert!(stdout.contains("MTH  89"), "$size inside a class method must be 89.\n{combined}");
    assert!(stdout.contains("HI   88"), "$high of bit[88:0] must be 88.\n{combined}");
    assert!(stdout.contains("LO   0"),  "$low of bit[88:0] must be 0.\n{combined}");
    assert!(stdout.contains("EQ   1"),  "$size must equal $bits for a class member.\n{combined}");
}