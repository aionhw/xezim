//! §6.12 / §10.7 — a `byte` class-member dynamic/static array element written
//! with a NEGATIVE RHS in a BYTECODE-compiled method must be stored with the
//! element's declared signedness (`byte`), so a later read that widens to a
//! signed value SIGN-extends (`-1`) rather than zero-extends (`255`).
//!
//! The compiled `StoreCollElem` clipped to the declared 8-bit width and kept
//! the RHS's `is_signed` flag. A `foreach` over a dynamic array yields an
//! UNSIGNED index, so `ba[i] = -i` produced an unsigned RHS and was stored as
//! unsigned 8'hFF; a later unsigned widening read of `ba[1]` then printed 255
//! instead of -1. The AST element store stamps the DECLARED element
//! signedness (`assignments.rs::assign_queue_elem_value`); the compiled path
//! must mirror it. This is the shape surfaced by the printer regression that
//! printed 255 for `ba[1]`/`bsa[1]` (queues stored by `push_back` never lost
//! signedness, hence only the array elements diverged).
use std::process::Command;

const DESIGN: &str = r#"
module tb;
  class myobject;
    int  ia[];
    byte ba[];
    byte bsa[4];
    byte bq[$];
    function new();
      ia = new[4];
      ba = new[4];
      foreach (ia[i]) begin
        ba[i]  = -i;
        bsa[i] = -i;
        bq.push_back(-i);
      end
    endfunction
    function void check();
      int bad = 0;
      for (int k = 0; k < 4; k++) begin
        if (ba[k] != -k) bad++;
        if (bsa[k] != -k) bad++;
        if (bq[k]  != -k) bad++;
      end
      if (bad == 0) $display("TAG_PASS");
      else $display("TAG_FAIL bad=%0d ba1=%0d bsa1=%0d bq1=%0d",
                    bad, ba[1], bsa[1], bq[1]);
    endfunction
  endclass
  initial begin
    myobject o;
    o = new();
    o.check();
  end
endmodule
"#;

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_bytesign_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, DESIGN).expect("write design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "tb", path.to_str().unwrap(), "--no-cache"])
        .env("XEZIM_COMPILE_METHODS", compile)
        .env("XEZIM_METHOD_TIER", "0")
        .env_remove("XEZIM_METHOD_CACHE")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    text
}

/// A `byte` element written negative (unsigned foreach index) in a COMPILED
/// method must read back sign-extended, identically to the interpreted path.
#[test]
fn compiled_byte_array_element_preserves_signedness() {
    for compile in ["0", "1"] {
        let log = run(compile);
        assert!(
            log.contains("TAG_PASS"),
            "XEZIM_COMPILE_METHODS={compile}: byte array element not sign-extended:\n{log}"
        );
    }
}