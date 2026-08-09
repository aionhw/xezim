//! §7.8.2: an associative-array index is narrowed to the declared key type
//! before keying. `int k; shortint aa[shortint]; aa[k]` with a 32-bit `k`
//! must store the 16-bit-truncated value so a later read (which narrows the
//! same way) matches. Previously xezim stored the full-width key text, so the
//! packed key diverged from the unpacked key and AA round-trips through UVM
//! pack/unpack corrupted (doubled entries, mangled keys).
//!
//! These also pin the related foreach fix: a signed key like -11823 must stay
//! signed through iteration, not be re-parsed as u64 (which fails and yields 0).

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// Module-scope AA: a 32-bit key narrows to the declared shortint (16-bit)
/// index type.
const MODULE_SCOPE_NARROW: &str = r#"
module top;
  shortint aa[shortint];
  initial begin
    int k;
    k = 32'h0000FFFF; // 65535 -> narrows to -1 (signed shortint)
    aa[k] = 42;
    // direct read must narrow consistently
    if (aa[k] != 42) $display("TAG_FAIL read %0d", aa[k]);
    // foreach must see exactly 1 entry with the correct signed key
    int count = 0;
    foreach (aa[i]) begin
      count++;
      $display("KEY %0d %0d", i, aa[i]);
    end
    if (count != 1) $display("TAG_FAIL count %0d", count);
    if (count == 1 && aa[k] == 42) $display("TAG_PASS");
    else $display("TAG_FAIL");
  end
endmodule
"#;

#[test]
fn module_scope_aa_key_narrows() {
    let sim = simulate(MODULE_SCOPE_NARROW, 200).expect("simulate failed");
    let msgs = messages(&sim);
    // The key must be -1 (narrowed from 65535 to signed shortint).
    assert!(
        msgs.iter().any(|m| m == "KEY -1 42"),
        "expected narrowed signed key -1, got: {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "expected TAG_PASS, got: {:?}",
        msgs
    );
}

/// Class-property AA: a 32-bit key narrows to the declared shortint index,
/// and foreach iterates the correctly-narrowed signed keys.
const CLASS_PROP_NARROW: &str = r#"
class item;
  shortint aa[shortint];
  function new(); endfunction
  function void show;
    foreach (aa[i])
      $display("KEY %0d %0d", i, aa[i]);
  endfunction
endclass
module top;
  initial begin
    item it = new;
    it.aa[5] = 11;
    it.aa[-3] = 22;
    it.show();
  end
endmodule
"#;

#[test]
fn class_prop_aa_signed_keys_iterate() {
    let sim = simulate(CLASS_PROP_NARROW, 200).expect("simulate failed");
    let msgs = messages(&sim);
    // Both keys must appear with correct values — the negative key -3 must
    // NOT be clobbered to 0 by a failed u64 parse.
    assert!(
        msgs.iter().any(|m| m == "KEY -3 22"),
        "expected key -3 val 22, got: {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m == "KEY 5 11"),
        "expected key 5 val 11, got: {:?}",
        msgs
    );
}

/// Narrowing consistency: writing with a wide key and reading with the same
/// wide key must find the element (both narrow identically). A DIFFERENT
/// value that narrows to the same key must collide.
const NARROW_CONSISTENCY: &str = r#"
module top;
  byte aa[byte];  // byte index = 8-bit signed, element = byte
  initial begin
    int a, b;
    a = 32'h00000005;  // narrows to 5
    b = 32'h00000105;  // low byte also 5 -> collides with a after narrowing
    aa[a] = 100;
    aa[b] = 50;        // overwrites aa[5] (same narrowed key)
    $display("num=%0d val=%0d", aa.num(), aa[5]);
    if (aa.num() == 1 && aa[5] == 50) $display("TAG_PASS");
    else $display("TAG_FAIL num=%0d val=%0d", aa.num(), aa[5]);
  end
endmodule
"#;

#[test]
fn narrowed_keys_collide_consistently() {
    let sim = simulate(NARROW_CONSISTENCY, 200).expect("simulate failed");
    let msgs = messages(&sim);
    // a=5 and b=0x105 both narrow to byte 5, so they collide: num()==1.
    // The second write (50) overwrites the first (100).
    assert!(
        msgs.iter().any(|m| m == "TAG_PASS"),
        "expected narrowing collision (num=1, val=50), got: {:?}",
        msgs
    );
}
