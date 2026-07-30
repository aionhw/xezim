//! A class method named like a §6.16 string builtin must not be shadowed by it.
//!
//! xezim's string-method builtins (`compare`, `icompare`, `putc`, `itoa`,
//! `tolower`, `atoi`, …) dispatch by NAME + arg-count on ANY receiver. So a
//! call to a user-defined class method of the same name was silently rerouted
//! to the string builtin instead of the user method. The headline case is UVM:
//! `uvm_object::compare(rhs, comparer=null)` is inherited by every UVM object,
//! and is *always* called with the trailing `comparer` left at its default —
//! i.e. exactly one argument. That matched the `string::compare(s)` builtin,
//! which stringified both handles and returned their lexicographic difference
//! (-1/0/1) instead of invoking the comparison, so two distinct objects
//! "compared equal".
//!
//! The guard walks the receiver's declared type up the `extends` chain: if the
//! class (or any ancestor) declares a method of that name, the string builtins
//! are skipped and normal class-method dispatch runs.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn xezim_bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("test exe path");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim")
}

fn run(src: &str) -> String {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("xezim_strshadow_{}_{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let p = dir.join("t.sv");
    std::fs::write(&p, src).expect("write");
    let bin = xezim_bin();
    if !bin.exists() {
        return String::new(); // binary not built in this profile
    }
    let out = Command::new(bin)
        .arg("--simulate")
        .arg("-s")
        .arg("top")
        .arg(&p)
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// A `compare(rhs, ext=0)` method — shaped exactly like `uvm_object::compare` —
/// called with the trailing default OMITTED must invoke the user method, not
/// `string::compare`. The user method runs only if dispatch reaches it (it sets
/// a flag the test then reads back).
const COMPARE_SHADOW: &str = r#"
module top;
  class obj;
    int tag;
    bit ran;
    function new(int t); tag=t; ran=0; endfunction
    // Mirrors uvm_object::compare(rhs, comparer=null): a trailing defaulted
    // object arg so the call site has exactly ONE argument.
    virtual function bit compare(obj rhs, int ext=0);
      ran = 1;
      compare = (tag == rhs.tag) ? 1 : 0;
    endfunction
  endclass

  obj a, b;
  initial begin
    a = new(5);
    b = new(9);          // different tag -> must compare UNEQUAL
    if (a.compare(b) != 0) begin        // omitting default arg is the trigger
      $display("RESULT equal");          // wrong: string builtin returned >=0
    end else begin
      $display("RESULT unequal");        // correct: user method returned 0
    end
    // Same call WITH the explicit default arg already worked before the fix.
    $display("EXPLICIT=%0d", a.compare(b, 0));
    $display("RAN=%0d", a.ran);
  end
endmodule
"#;

#[test]
fn class_compare_not_shadowed_by_string_builtin() {
    let out = run(COMPARE_SHADOW);
    if out.is_empty() {
        return;
    }
    // The user method must have actually executed.
    assert!(out.contains("RAN=1"), "user compare() never ran; got:\n{out}");
    // Distinct tags -> unequal. The string builtin would stringify both
    // object handles (identical text) and return >= 0, masking the
    // mismatch.
    assert!(
        out.contains("RESULT unequal"),
        "must report unequal for distinct objects; got:\n{out}"
    );
    assert!(
        out.contains("EXPLICIT=0"),
        "explicit-arg form must also be unequal; got:\n{out}"
    );
}

/// The shadowing is per-NAME: a class WITHOUT a `compare` method must still be
/// unaffected (there's nothing to shadow). This guards against an over-broad
/// guard that disabled string builtins for all class receivers.
const NO_METHOD_FALLTHROUGH: &str = r#"
module top;
  // A bare string compare via the integral->string path: the receiver has no
  // class method named `compare`, so the §6.16 builtin must still apply.
  string s = "apple";
  initial begin
    $display("CMP=%0d", s.compare("banana"));  // "apple" < "banana" -> -1
  end
endmodule
"#;

#[test]
fn string_builtin_still_works_on_a_string_receiver() {
    let out = run(NO_METHOD_FALLTHROUGH);
    if out.is_empty() {
        return;
    }
    assert!(
        out.contains("CMP=-1"),
        "string::compare builtin must still run for a string receiver; got:\n{out}"
    );
}

/// Other string builtins named like plausible class methods (`tolower`) must
/// likewise defer to a user method on a class receiver.
const TOLOWER_SHADOW: &str = r#"
module top;
  class cfg;
    int v;
    function new(int v); this.v=v; endfunction
    virtual function int tolower();
      tolower = v + 100;   // clearly NOT the string lowercaser
    endfunction
  endclass
  cfg c;
  initial begin
    c = new(7);
    $display("TL=%0d", c.tolower());   // expect 107, not a stringified handle
  end
endmodule
"#;

#[test]
fn class_tolower_not_shadowed_by_string_builtin() {
    let out = run(TOLOWER_SHADOW);
    if out.is_empty() {
        return;
    }
    assert!(
        out.contains("TL=107"),
        "user tolower() must run, not the string builtin; got:\n{out}"
    );
}
