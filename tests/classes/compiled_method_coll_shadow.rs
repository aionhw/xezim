//! class-perf: a bare builtin call on a member collection inside a
//! COMPILED method must resolve the member's instance store, never a
//! same-named local of an enclosing INTERPRETED frame.
//!
//! The compiled `CallCollMethod` bare arm used to feed the builtin
//! dispatcher the raw member name; the dispatcher's §8.10 rewrite consults
//! `dyn_name_lookup` FIRST, so a local `elements[$]` declared by an
//! (interpreted) caller hijacked the resolution and the compiled callee
//! read the caller's empty snapshot instead of its own member queue
//! (observed live on the UVM report-message element-table regression:
//! `uvm_report_message_element_container::size` answered 0 because every
//! report hook declares its own `elements[$]` local). The arm now resolves
//! `instance_assoc_member(member)` — the same scoped store the AST funnel
//! computes for a member receiver — before dispatching.
//!
//! Subprocess test: the tier threshold is process-cached, and the bug
//! needs a specific interpreted/compiled mix (callee compiled while an
//! interpreted caller's same-named local is live), so each scenario runs
//! the binary as a child process.

use std::path::PathBuf;
use std::process::Command;

fn xezim_bin() -> PathBuf {
    // Cargo names the binary it built for the integration tests.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_xezim"))
}

fn write_src(name: &str, body: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("xezim-collshadow-{}.sv", name));
    std::fs::write(&p, body).expect("write src");
    p
}

fn run(src: &PathBuf, methods: &str, tier: &str) -> String {
    let out = Command::new(xezim_bin())
        .arg("--simulate")
        .arg("-s")
        .arg("top")
        .arg(src)
        .env("XEZIM_COMPILE_METHODS", methods)
        .env("XEZIM_METHOD_TIER", tier)
        .output()
        .expect("subprocess failed");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The callee's member queue `elements` holds 3 entries; `probe` declares
/// its OWN empty `elements[$]` local with the same name — exactly the UVM
/// report-hook shape. At tier 3 the call schedule is: size#1 (module,
/// interpreted), size#2 inside probe#1 (both interpreted), size#3 inside
/// probe#2 — the tier boundary fires HERE, so the compiled bare
/// `elements.size()` executes while probe#2's interpreted frame (and its
/// live `elements` local registration) is on the dyn stack. probe#3
/// itself compiles (3rd call), so size#4 runs with the caller's locals in
/// registers instead — the stale-registration window.
const SRC: &str = r#"
class holder;
  protected int elements[$];
  function void add(int v);
    elements.push_back(v);
  endfunction
  function int size();
    return elements.size();
  endfunction
endclass

class caller;
  function int probe(holder h);
    int elements[$];
    return h.size();
  endfunction
endclass

module top;
  holder h;
  caller c;
  int r0;
  int r1;
  int r2;
  int r3;
  int i;
  initial begin
    h = new();
    c = new();
    for (i = 0; i < 3; i = i + 1)
      h.add(i + 5);
    r0 = h.size();   // call 1: interpreted
    r1 = c.probe(h); // call 2: interpreted (probe#1)
    r2 = c.probe(h); // call 3: COMPILED inside interpreted probe#2
    r3 = c.probe(h); // call 4: compiled inside compiled probe#3
    $display("TAG %0d %0d %0d %0d", r0, r1, r2, r3);
    if (r0 == 3 && r1 == 3 && r2 == 3 && r3 == 3)
      $display("TAG_PASS");
    else
      $display("TAG_FAIL");
  end
endmodule
"#;

#[test]
fn bare_member_coll_call_ignores_enclosing_local_shadow() {
    let src = write_src("shadow", SRC);
    // Gate off: interpreter ground truth.
    let off = run(&src, "0", "100");
    assert!(off.contains("TAG 3 3 3 3"), "gate-off run: {}", off);
    assert!(off.contains("TAG_PASS"), "gate-off run: {}", off);

    // Tier 3: the boundary fires on size call 3, inside the interpreted
    // probe#2 whose `elements` local used to hijack the bare resolution.
    let t3 = run(&src, "1", "3");
    assert!(t3.contains("TAG 3 3 3 3"), "tier-3 run: {}", t3);
    assert!(t3.contains("TAG_PASS"), "tier-3 run: {}", t3);

    // Eager: everything compiled — the bare arm must still resolve the
    // member store, and no stale registration may leak in.
    let t0 = run(&src, "1", "0");
    assert!(t0.contains("TAG 3 3 3 3"), "tier-0 run: {}", t0);
    assert!(t0.contains("TAG_PASS"), "tier-0 run: {}", t0);
}
