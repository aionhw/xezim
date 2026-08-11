//! IEEE 1800-2023 §7.4.5 / Table 7-1 conformance: reading a NONEXISTENT
//! element of a class-handle collection must yield `null` (not an X handle).
//!
//! The reference simulator and the standard both read a nonexistent element
//! of a class-typed queue / dynamic array / associative array as `null`. xezim
//! read these as an X-stained handle (`x[0] == null` evaluated to X), which
//! also diverged from the reference output byte-for-byte. The collection
//! element-read fallback now treats a class-handle element as `null` and
//! bypasses the reserved-but-unpopulated OOB cells of a dynamic array.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

const CLASS_OOB_SRC: &str = r#"
module top;
  class K; endclass
  K din[];
  K q[$];
  K a[int];
  K kv;
  initial begin
    din = new[4];
    if (din[0] == null) begin end else $display("D0_NONNULL");
    if (din[9] ==  null) $display("DIN_PASS");  // OOB dynamic -> null
    else                 $display("DIN_FAIL");
    kv = q[3];                                 // empty-queue OOB -> null
    if (kv == null) $display("QUEUE_PASS");
    else            $display("QUEUE_FAIL");
    if (a[99] == null) $display("ASSOC_PASS"); // assoc class OOB -> null
    else               $display("ASSOC_FAIL");
  end
endmodule
"#;

#[test]
fn test_nonexistent_class_elem_reads_null() {
    let sim = simulate(CLASS_OOB_SRC, 200).expect("simulate failed");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "DIN_PASS"),
        "OOB dynamic class element must read null; got {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m == "QUEUE_PASS"),
        "OOB queue class element must read null; got {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m == "ASSOC_PASS"),
        "OOB assoc class element must read null; got {:?}",
        msgs
    );
    assert!(
        !msgs.iter().any(|m| m.ends_with("_FAIL") || m.ends_with("_NONNULL")),
        "unexpected non-null class read: {:?}",
        msgs
    );
}