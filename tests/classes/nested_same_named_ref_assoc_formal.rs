//! §13.5.2 (`ref` associative-array formal) + §7.9: an OUTER method with a
//! `ref` assoc formal `list` that CALLS a NESTED method which ALSO has a `ref`
//! assoc formal named `list`, then continues writing to its own `list` after
//! the nested call returns.
//!
//! REGRESSION: the flat `module.associative_arrays` table is shared across
//! call-nesting depths, keyed by bare formal name. The nested call's
//! `bind_assoc_param`/`purge_assoc_param` overwrote and then REMOVED the
//! `list` entry that the outer call had created, so after the nested return the
//! outer's `list` was no longer recognized as an associative array:
//! `list[k]=v` writes stopped landing in the `list[...]` namespace and
//! `list.num()`/`list.size()` read 0.
//!
//! UVM's TLM2 `uvm_port_component::get_provided_to`/`get_connected_to` trip
//! this: both the proxy's `ref uvm_port_list list` and the inner
//! `uvm_port_base::get_provided_to(ref uvm_port_base list)` name their formal
//! `list`, so after the inner call the proxy's `list` silently stayed empty and
//! every `ap.connect(export)` link was reported missing (a hard UVM_ERROR).
use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

fn run(src: &str, tag: &str) -> String {
    let path = format!("/tmp/nested_assoc_{tag}.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", &path])
        .output()
        .expect("run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Outer method `get(ref ltype list)` calls nested `h.fill(ref ltype list)`,
/// then continues writing to `list`; both formals are named `list`.
const NESTED_SAME_NAME: &str = r#"module top;
  typedef int ltype[string];

  class helper;
    function void fill(ref ltype list);   // nested formal ALSO named `list`
      list["n1"] = 91;
      list["n2"] = 92;
    endfunction
  endclass

  class proxy;
    helper h = new;
    function void get(ref ltype list);     // outer formal named `list`
      ltype list1;
      h.fill(list1);                       // nested call, same formal name
      list.delete();
      foreach (list1[k]) begin
        list[k] = list1[k];
      end
      if (list.num() == 2) $display("RESULT PASS inner_num=%0d", list.num());
      else                 $display("RESULT FAIL inner_num=%0d", list.num());
    endfunction
  endclass

  task runit();
    ltype out;
    automatic proxy p = new;
    p.get(out);
    if (out.num() == 2) $display("RESULT PASS out_num=%0d", out.num());
    else                $display("RESULT FAIL out_num=%0d", out.num());
  endtask

  initial begin
    runit();
    #1;
    $finish;
  end
endmodule
"#;

#[test]
fn nested_same_named_ref_assoc_formal_does_not_clobber_outer() {
    let out = run(NESTED_SAME_NAME, "nested");
    assert!(
        out.contains("RESULT PASS"),
        "an outer ref assoc formal must stay associative after a nested\n\
         method with a same-named formal returns (purging the shared flat\n\
         registration must not unregister the outer's array)\noutput:\n{out}"
    );
}