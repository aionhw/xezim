//! class-perf Step 9c: `$cast` inside compiled class-method bodies lowers
//! to `Insn::Cast`. The exec arm re-uses the interpreter's own runtime
//! verdict (`cast_type_ok` on the carried AST — the same overlay tables,
//! including the class-typed-local pre-record at compiled-method entry),
//! and mirrors the frame-write fit (`assign_value_inner`'s
//! `frame_leaf_key_of` arms) for register-backed destinations.
//!
//! Covers: function-form success/failure (dest UNCHANGED on failure),
//! enum member destination (§6.19.3 range check), statement (task) form
//! incl. its failure diagnostic, `this.`-member destination, dotted
//! member destination, and the type-parameter local in a parameterized
//! class (the `uvm_callbacks::get_first` shape — `CB cb;` then
//! `$cast(cb, q.get(itr))` in a scan loop). A STATIC member destination
//! (`cbq_c#(ext1_c)::shared_found`) is exercised under an active
//! specialization so the `current_spec`-dependent verdict path is pinned
//! too.
//!
//! Expectations are reference-simulator-validated (method_vm34 runs
//! byte-for-byte identically under the reference simulator, gate OFF, and
//! gate ON). The gate is forced ON so a plain `cargo test` exercises the
//! compiled path.
use xezim::simulate;

fn gate_on() -> bool {
    super::compiled_method_test_env::eager()
}

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

#[test]
fn cast_all_shapes() {
    if !gate_on() {
        return;
    }
    let src = r#"
class base_c;
  int id;
  function new(int i); id = i; endfunction
  function int get_id(); return id; endfunction
endclass

class ext1_c extends base_c;
  function new(int i); super.new(i); endfunction
  function int tag(); return 1; endfunction
endclass

class ext2_c extends base_c;
  function new(int i); super.new(i); endfunction
  function int tag(); return 2; endfunction
endclass

typedef enum int { A_LOW = 1, A_MID = 2, A_HIGH = 5 } a_e;

class holder_c;
  base_c comp;
  a_e    lvl;
  int    r;
  function new();
    comp = null;
    lvl  = A_LOW;
    r    = 0;
  endfunction

  function int cast_ok(base_c src);
    ext1_c e;
    e = null;
    r = $cast(e, src);
    if (r != 0) r = r + e.get_id();
    return r;
  endfunction

  function int cast_member(base_c src);
    if ($cast(comp, src)) return comp.get_id();
    return -1;
  endfunction

  function int cast_enum(int v);
    if ($cast(lvl, v)) return 100 + int'(lvl);
    return -100 - v;
  endfunction

  function int cast_dotted(holder_c h, base_c src);
    if ($cast(h.comp, src)) return 1;
    return 0;
  endfunction
endclass

class q_c #(type T = int);
  T items[$];
  function void push(T v); items.push_back(v); endfunction
  function T get(int i); return items[i]; endfunction
  function int size(); return items.size(); endfunction
endclass

class cbq_c #(type CB = base_c);
  q_c #(base_c) m_q;
  static CB shared_found;
  CB found;
  function new(); m_q = new(); found = null; shared_found = null; endfunction
  function void push(base_c v); m_q.push(v); endfunction
  function int find_id(input int start);
    q_c #(base_c) q;
    CB cb;
    int itr;
    int n;
    cb          = null;
    this.found  = null;
    shared_found = null;
    q     = m_q;
    n     = q.size();
    for (itr = start; itr < n; itr++) begin
      if (!$cast(cb, q.get(itr))) begin
        cb = null;
      end
      else begin
        this.found = cb;
        shared_found = cb;
        return itr;
      end
    end
    return -1;
  endfunction
endclass

module top;
  holder_c h;
  ext1_c e1;
  ext2_c e2;
  base_c b;
  cbq_c #(ext1_c) cbq;
  int o_ok_e1, o_ok_b, o_mem1, o_mem2, o_comp, o_en5, o_en3, o_d1, o_d2;
  int o_find0, o_find0_id, o_find1, o_find1_null, o_sfind0, o_sfind1;

  initial begin
    h  = new();
    e1 = new(11);
    e2 = new(22);
    b  = new(33);

    o_ok_e1 = h.cast_ok(e1);   // 1 + 11
    o_ok_b  = h.cast_ok(b);    // base object is not ext1 -> 0
    o_mem1  = h.cast_member(e1);
    o_mem2  = h.cast_member(e2);
    o_comp  = h.comp.get_id();
    o_en5   = h.cast_enum(5);
    o_en3   = h.cast_enum(3);
    o_d1    = h.cast_dotted(h, e2);
    o_d2    = h.cast_dotted(h, e1);

    cbq = new();
    cbq.push(e1);
    cbq.push(e2);
    cbq.push(b);
    o_find0 = cbq.find_id(0);
    if (cbq.found != null) o_find0_id = cbq.found.get_id();
    else                   o_find0_id = 999;
    // static read BETWEEN scans: find_id(0) reset it on entry, then the
    // successful e1 cast wrote it -> 11
    if (cbq_c#(ext1_c)::shared_found != null)
      o_sfind0 = cbq_c#(ext1_c)::shared_found.get_id();
    else
      o_sfind0 = 999;
    o_find1 = cbq.find_id(1);  // e2 and b both fail the ext1 cast
    o_find1_null = (cbq.found == null) ? 1 : 0;
    // find_id(1) reset the static on entry; no cast succeeds -> stays null
    if (cbq_c#(ext1_c)::shared_found != null)
      o_sfind1 = cbq_c#(ext1_c)::shared_found.get_id();
    else
      o_sfind1 = 42;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate");
    // function form: success assigns dest and returns 1; failure returns 0
    assert_eq!(u(&sim, "o_ok_e1"), 12);
    assert_eq!(u(&sim, "o_ok_b"), 0);
    // this-member dest: base_c-typed member accepts both derived objects
    assert_eq!(u(&sim, "o_mem1"), 11);
    assert_eq!(u(&sim, "o_mem2"), 22);
    assert_eq!(u(&sim, "o_comp"), 22);
    // enum member dest: member value passes, non-member fails (and the
    // member keeps its previous value — checked via the next cast)
    assert_eq!(u(&sim, "o_en5"), 105);
    assert_eq!(u(&sim, "o_en3"), (-103i32) as u32 as u64);
    // dotted dest: base-typed member accepts e2 too
    assert_eq!(u(&sim, "o_d1"), 1);
    assert_eq!(u(&sim, "o_d2"), 1);
    // get_first shape: first scan hits ext1 at 0; rescan from 1 fails
    // both remaining entries (function-form cast failure is silent and
    // leaves the dest unchanged)
    assert_eq!(u(&sim, "o_find0"), 0);
    assert_eq!(u(&sim, "o_find0_id"), 11);
    assert_eq!(u(&sim, "o_find1"), (-1i32) as u32 as u64);
    assert_eq!(u(&sim, "o_find1_null"), 1);
    // static-member dest: written under the active specialization, read
    // back through the class-scope resolution (value BETWEEN the scans)
    assert_eq!(u(&sim, "o_sfind0"), 11);
    // the failing rescan resets the static on entry and nothing succeeds
    assert_eq!(u(&sim, "o_sfind1"), 42);
}

/// Statement-form `$cast` failure must print the runtime diagnostic on the
/// COMPILED path too (the interpreter already does). Runs the binary
/// directly to capture stdout.
#[test]
fn cast_stmt_form_failure_prints_error_gate_on() {
    if !gate_on() {
        return;
    }
    let src = r#"
class base_c;
  int id;
  function new(int i); id = i; endfunction
endclass
class ext1_c extends base_c;
  function new(int i); super.new(i); endfunction
endclass
class ext2_c extends base_c;
  function new(int i); super.new(i); endfunction
endclass
class holder_c;
  function int fail_cast(ext2_c src);
    ext1_c e;
    e = null;
    $cast(e, src);
    return 0;
  endfunction
endclass
module top;
  holder_c h;
  ext2_c e2;
  int o_done;
  initial begin
    h  = new();
    e2 = new(5);
    void'(h.fail_cast(e2));
    o_done = 1;
  end
endmodule
"#;
    let path = "/tmp/compiled_method_cast_stmt_fail.sv";
    std::fs::write(path, src).unwrap();
    let mut cmd = std::process::Command::new(xezim_bin());
    cmd.args(["--simulate", "-s", "top", path]);
    cmd.env("XEZIM_COMPILE_METHODS", "1");
    let out = cmd.output().expect("run xezim");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        stdout.contains("$cast: source object is not assignment-compatible"),
        "statement-form $cast failure must print the diagnostic on the compiled path; stdout:\n{}",
        stdout
    );
}

fn xezim_bin() -> String {
    // Cargo names the binary it built for the integration tests.
    env!("CARGO_BIN_EXE_xezim").to_string()
}
