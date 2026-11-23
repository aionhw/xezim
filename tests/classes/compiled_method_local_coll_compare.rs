//! A compare of two elements of a method-local queue or dynamic array in a
//! compiled method (`a[0] == a[1]`, a sort's `a[j] > a[j+1]`) must compare
//! the elements at their own width.
//!
//! The element read of a local collection was missing from the width oracle
//! a self-determined compare sizes its operands with, so it counted as one
//! bit and the compiled compare resized both elements to their LSBs:
//! `'{3, 1}` compared equal, and a bubble sort in an automatic function left
//! its array unsorted. A looping function compiles on its first call, so
//! each function here loops. Each case is also run with the method-compile
//! tier off (`XEZIM_COMPILE_METHODS=0`): the interpreter must agree.

use std::process::Command;

const SRC: &str = r#"
module top;
  bit r[9];
  logic rx[4];
  function automatic void dyn_cmp();
    int a[];
    a = '{3, 1};
    for (int i = 0; i < 1; i++);
    r[0] = a[0] == a[1];
    r[1] = a[0] != a[1];
    r[2] = a[0] > a[1];
    r[3] = a[0] < a[1];
    r[4] = a[0] <= a[1];
    r[5] = a[0] >= a[1];
    r[6] = a[0] === a[1];
    r[7] = a[0] == a[0];
    r[8] = a[1] >= a[1];
    $display("CMP dyn %0d%0d%0d%0d%0d%0d%0d%0d%0d", r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7], r[8]);
  endfunction
  function automatic void queue_cmp();
    byte q[$];
    q = '{3, 1};
    for (int i = 0; i < 1; i++);
    r[0] = q[0] == q[1];
    r[1] = q[0] != q[1];
    r[2] = q[0] > q[1];
    r[3] = q[0] < q[1];
    r[4] = q[0] <= q[1];
    r[5] = q[0] >= q[1];
    r[6] = q[0] === q[1];
    r[7] = q[0] == q[0];
    r[8] = q[1] >= q[1];
    $display("CMP queue %0d%0d%0d%0d%0d%0d%0d%0d%0d", r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7], r[8]);
  endfunction
  // Signed elements: -1 < 1. At one bit both were 1.
  function automatic void signed_cmp();
    int s[];
    s = '{-1, 1};
    for (int i = 0; i < 1; i++);
    r[0] = s[0] < s[1];
    r[1] = s[0] > s[1];
    r[2] = s[0] == s[1];
    r[3] = s[0] <= s[1];
    r[4] = s[0] >= s[1];
    $display("CMP signed %0d%0d%0d%0d%0d", r[0], r[1], r[2], r[3], r[4]);
  endfunction
  // An x bit above bit 0: == is x, === tells the elements apart.
  function automatic void xz_cmp();
    logic [7:0] x[];
    x = '{8'b0000_00x1, 8'b0000_0001};
    for (int i = 0; i < 1; i++);
    rx[0] = x[0] == x[1];
    rx[1] = x[0] != x[1];
    rx[2] = x[0] === x[0];
    rx[3] = x[0] === x[1];
    $display("CMP xz %b %b %b %b", rx[0], rx[1], rx[2], rx[3]);
  endfunction
  // The search testbench's shape: sort a local dynamic array in place.
  function automatic void sort_dyn();
    int a[];
    int t;
    a = '{5, 3, 7, 1, 6, 2, 8, 4};
    for (int i = 0; i < 8; i++)
      for (int j = 0; j < 7 - i; j++)
        if (a[j] > a[j+1]) begin
          t = a[j]; a[j] = a[j+1]; a[j+1] = t;
        end
    $display("SORT %0d %0d %0d %0d %0d %0d %0d %0d", a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]);
  endfunction
  initial begin
    dyn_cmp();
    queue_cmp();
    signed_cmp();
    xz_cmp();
    sort_dyn();
  end
endmodule
"#;

const WANT: [&str; 5] = [
    "CMP dyn 011001011",
    "CMP queue 011001011",
    "CMP signed 10010",
    "CMP xz x x 1 0",
    "SORT 1 2 3 4 5 6 7 8",
];

fn run(methods: &str) -> String {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("compiled_method_local_coll_compare_{methods}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join("t.sv");
    std::fs::write(&sv, SRC).unwrap();
    // Tier 0 compiles a method on its first call whatever its shape, and no
    // method cache may hand back a block another build compiled.
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .env("XEZIM_COMPILE_METHODS", methods)
        .env("XEZIM_METHOD_TIER", "0")
        .env_remove("XEZIM_METHOD_CACHE")
        .current_dir(&dir)
        .args(["--no-cache"])
        .arg(&sv)
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

#[test]
fn compiled_method_compares_local_collection_elements_at_their_width() {
    let stdout = run("1");
    for w in WANT {
        assert!(stdout.contains(w), "missing {w}:\n{stdout}");
    }
}

#[test]
fn interpreter_agrees_on_local_collection_element_compares() {
    let stdout = run("0");
    for w in WANT {
        assert!(stdout.contains(w), "missing {w}:\n{stdout}");
    }
}
