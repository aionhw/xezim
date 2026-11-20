//! class-perf Step 9a: string-typed class members on the compiled-method
//! surface. `uvm_object::get_name` (the single largest remaining bail source
//! in UVM — `return m_leaf_name;` ran 31 868 AST-interpreted times on the
//! objection anchor regression) lowers now: the heap stores a string member
//! as a plain byte-vector Value and the interpreter's dotted read passes it
//! through verbatim, so `LoadClassMember` is as faithful for a string as for
//! an integral member. Bare reads (`return m_leaf_name;`), dotted reads
//! (`other.m_leaf_name`) and string equality against a member join the
//! compiled path; everything else (writes, indexing, string methods on the
//! member) keeps the AST interpreter.
//!
//! Expectations are reference-simulator-validated (the method_vm27 shape and
//! its reference run print identical values). The gate is forced ON so a
//! plain `cargo test` exercises the compiled path.
use xezim::simulate;

fn gate_on() -> bool {
    super::compiled_method_test_env::eager()
}

fn s(sim: &xezim::compiler::Simulator, n: &str) -> String {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_sv_string()
}

/// The method_vm27 surface: bare + dotted + inherited string-member reads,
/// string equality against a member, reads through a handle formal.
#[test]
fn string_member_reads_end_to_end() {
    if !gate_on() {
        return;
    }
    let src = r#"
class base_obj;
  string m_leaf_name;
  int m_inst_id;
  function new(string name = "");
    m_leaf_name = name;
  endfunction
  function string get_name();
    return m_leaf_name;
  endfunction
  function void set_name(string name);
    m_leaf_name = name;
  endfunction
  function string describe();
    if (m_leaf_name == "") begin
      return "(unset)";
    end
    return m_leaf_name;
  endfunction
  function string via_handle(base_obj other);
    if (other == null) begin
      return "null";
    end
    return other.m_leaf_name;
  endfunction
endclass
class child_obj extends base_obj;
  function new(string name); super.new(name); endfunction
  function string child_describe();
    return m_leaf_name;
  endfunction
endclass
module top;
  string r1, r2, r3, r4, r5, r6, r7, r8;
  initial begin
    base_obj b;
    child_obj c;
    base_obj e;
    b = new("alpha");
    c = new("beta");
    e = new();
    r1 = b.get_name();
    r2 = c.get_name();
    r3 = c.child_describe();
    b.set_name("gamma");
    r4 = b.get_name();
    r5 = b.describe();
    r6 = e.describe();
    r7 = b.via_handle(c);
    r8 = b.via_handle(null);
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulation should run");
    assert_eq!(s(&sim, "r1"), "alpha");
    assert_eq!(s(&sim, "r2"), "beta");
    assert_eq!(s(&sim, "r3"), "beta");
    assert_eq!(s(&sim, "r4"), "gamma");
    assert_eq!(s(&sim, "r5"), "gamma");
    assert_eq!(s(&sim, "r6"), "(unset)");
    assert_eq!(s(&sim, "r7"), "beta");
    assert_eq!(s(&sim, "r8"), "null");
}

/// The member is declared on the BASE class; the read happens in a SUBCLASS
/// method (`child_obj::get_name`). The gate's extends-chain walk must find
/// `m_leaf_name` there.
#[test]
fn string_member_read_through_inherited_chain() {
    if !gate_on() {
        return;
    }
    let src = r#"
class base_obj;
  string m_leaf_name;
  function new(string name = "");
    m_leaf_name = name;
  endfunction
endclass
class child_obj extends base_obj;
  function new(string name); super.new(name); endfunction
  function string get_name();
    return m_leaf_name;
  endfunction
endclass
module top;
  string r;
  initial begin
    child_obj c;
    c = new("leaf");
    r = c.get_name();
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulation should run");
    assert_eq!(s(&sim, "r"), "leaf");
}

/// A string-member WRITE (`m_leaf_name = name;` in `set_name`) keeps the AST
/// interpreter — but the value must still be readable back by a COMPILED
/// read (the write goes through `fit_class_prop`, which never resizes a
/// string member, so the bytes round-trip).
#[test]
fn string_member_written_by_ast_read_by_compiled() {
    if !gate_on() {
        return;
    }
    let src = r#"
class base_obj;
  string m_leaf_name;
  function new(string name = "");
    m_leaf_name = name;
  endfunction
  function void set_name(string name);
    m_leaf_name = name;
  endfunction
  function string describe();
    return m_leaf_name;
  endfunction
endclass
module top;
  string r1, r2, r3;
  initial begin
    base_obj b;
    b = new("start");
    r1 = b.describe();
    b.set_name("next-value");
    r2 = b.describe();
    b.set_name("");
    r3 = b.describe();
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulation should run");
    assert_eq!(s(&sim, "r1"), "start");
    assert_eq!(s(&sim, "r2"), "next-value");
    assert_eq!(s(&sim, "r3"), "");
}
