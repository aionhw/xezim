// Regression test: `$cast` of a concrete class instance whose class was
// declared `extends <typedef>` must still walk the typedef'd base's full
// inheritance chain. e.g. UVM sequence libraries register sequences declared
// via `typedef uvm_sequence #(REQ) simple_seq;` then `class seqA extends
// simple_seq;` — `seqA` IS a `uvm_sequence_base`, so `$cast(base_seq, seqA)`
// must succeed.
//
// Previously class_is_a stopped at a typedef base name (not present in the
// class table), so such sequences were never recognized as a base-class
// descendant and every sequence_library add failed `$cast` → [BAD_SEQ_TYPE].
// The fix resolves an `extends` target through any typedef indirection
// (`typedef_unroll`) before continuing the heritage walk.
//
// Verified byte-for-byte against the reference simulator (TAG_PASS).

use std::process::Command;

fn xezim() -> String {
    env!("CARGO_BIN_EXE_xezim").to_string()
}

fn run(src: &str, tag: &str) -> String {
    let path = format!("/tmp/seqcast_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", &path])
        .output()
        .expect("xezim failed to start");
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn cast_through_typedef_extends_base() {
    let out = run(
        r#"module top;
  class uvm_sequence_base;
  endclass
  class uvm_sequence #(type REQ = logic) extends uvm_sequence_base;
  endclass
  class simple_item;
  endclass
  typedef uvm_sequence #(simple_item) simple_seq;
  class seqA extends simple_seq;
  endclass
  uvm_sequence_base seq;
  initial begin
    int ok;
    seqA a = new();
    ok = $cast(seq, a);
    if (ok == 1) $display("TAG_PASS");
    else $display("TAG_FAIL cast_ok=%0d", ok);
  end
endmodule
"#,
        "typedef",
    );
    assert!(
        out.contains("TAG_PASS"),
        "typedef-based extends did not satisfy $cast to base class\n{}",
        out
    );
}