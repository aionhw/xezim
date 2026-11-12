//! class-perf row 2: non-INPUT (output/inout/ref) scalar formals on
//! compiled class methods.
//!
//! The interpreter binds a scalar non-input formal as copy-in/copy-out via
//! `output_bindings` (§13.3: a formal without an explicit direction inherits
//! the preceding one, so `accum(inout int acc, int add)` makes `add` Inout).
//! The compiled path now reproduces this for plain scalars:
//!
//! * the callee's body-final register value is copied into its frame at the
//!   epilogue (register write-back after the trailing label);
//! * a compiled CALLER marshals args and, after the call insn, copies each
//!   arg slot that came from a bare local back into that local — a no-op for
//!   input formals, the write-back for non-input ones;
//! * non-input positions of the CALLEE are additionally seeded as frame
//!   temps by the runtime so an AST-interpreted callee's epilogue has an
//!   lvalue to write (identity-ref semantics still decline collections).
//!
//! Note: output/inout on a class FUNCTION is rejected by strict LRM tools;
//! `ref` is the legal spelling and the one UVM actually uses
//! (`get_first_child(ref string name)`). xezim's interpreter accepts all
//! three uniformly, so the self-test pins ON==OFF behavior for each.

use std::process::Command;

fn xezim() -> String {
    // Cargo names the binary it built for the integration tests.
    env!("CARGO_BIN_EXE_xezim").to_string()
}

fn run(src: &str, gate: bool, tag: &str) -> String {
    let path = format!("/tmp/wb_formal_{tag}_{gate}.sv");
    std::fs::write(&path, src).unwrap();
    let mut cmd = Command::new(xezim());
    cmd.args(["--simulate", "-s", "top", &path]);
    if gate {
        // Safety: environment is inherited by the child only.
        cmd.env("XEZIM_COMPILE_METHODS", "1")
            .env("XEZIM_METHOD_TIER", "0")
            .env(
                "XEZIM_METHOD_CACHE",
                format!("/tmp/wb_formal_mc_{tag}_{gate}"),
            );
    } else {
        cmd.env("XEZIM_COMPILE_METHODS", "0");
    }
    let out = cmd.output().expect("run xezim");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with('R'))
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn check(name: &str, src: &str) {
    let off = run(src, false, name);
    let on = run(src, true, name);
    assert!(!off.is_empty(), "{name}: interpreter produced no output");
    assert_eq!(
        off, on,
        "{name}: compiled ON != interpreter OFF\nOFF:\n{off}\nON:\n{on}"
    );
}

#[test]
fn compiled_method_ref_formals() {
    // T1: scalar output written unconditionally + read result cell.
    // (output/inout on functions is LRM-illegal; xezim accepts them — the
    // interpreter's copy-in/copy-out is the gold the compiled path pins.)
    check(
        "t1_output_scalar",
        r#"
class C;
   function int compute(output int r);
      r = 7;
      return r + 1;
   endfunction
endclass
module top;
   initial begin
      C c = new;
      int r = 0;
      int v;
      v = c.compute(r);
      $display("R1 v=%0d r=%0d", v, r);
   end
endmodule
"#,
    );

    // T2: ref formal, string write-back through a pass-through chain
    // (the uvm_component::get_first_child shape: compiled callee forwards
    // its ref formal to another method's ref formal).
    check(
        "t2_ref_string_chain",
        r#"
class inner;
   int idx;
   function new();
      idx = 0;
   endfunction
   function int first(ref string s);
      if (idx >= 2) return 0;
      if (idx == 0) s = "alpha"; else s = "beta";
      idx++;
      return 1;
   endfunction
endclass
class outer;
   inner h;
   function new();
      h = new;
   endfunction
   function int get_first(ref string s);
      return h.first(s);
   endfunction
endclass
module top;
   initial begin
      outer o = new;
      string s;
      int n = 0;
      while (o.get_first(s)) begin
         $display("R2.%0d %s", n, s);
         n++;
      end
      $display("R2.end n=%0d", n);
   end
endmodule
"#,
    );

    // T3: multiple refs + inputs mixed (arg-slot write-back order).
    check(
        "t3_multi_ref",
        r#"
class C;
   function int pair(ref int a, input int b, ref int c);
      a = a + b;
      c = a * 2;
      return a + c;
   endfunction
endclass
module top;
   initial begin
      C c = new;
      int a = 10, b = 5, cc = 0;
      int rv = c.pair(a, b, cc);
      $display("R3 rv=%0d a=%0d cc=%0d", rv, a, cc);
   end
endmodule
"#,
    );

    // T4: class-handle output formal (handle written by callee).
    check(
        "t4_class_output",
        r#"
class node;
   string tag;
   function new(string t);
      tag = t;
   endfunction
endclass
class C;
   function int make(ref node n);
      n = new("made");
      return 1;
   endfunction
endclass
module top;
   initial begin
      C c = new;
      node n = null;
      int ok = c.make(n);
      if (n == null) $display("R4 null");
      else $display("R4 %0d %s", ok, n.tag);
   end
endmodule
"#,
    );

    // T5: unwritten output formal keeps the entry (copy-in) value; a
    // conditional write leaves the previous value on the untaken path.
    check(
        "t5_conditional_write",
        r#"
class C;
   function int maybe(ref int x, input bit do_it);
      if (do_it) x = 99;
      return x;
   endfunction
endclass
module top;
   initial begin
      C c = new;
      int x = 5;
      int r1 = c.maybe(x, 0);
      int r2 = c.maybe(x, 1);
      int r3 = c.maybe(x, 0);
      $display("R5 %0d %0d %0d %0d", r1, r2, r3, x);
   end
endmodule
"#,
    );

    // T6: direction inheritance — the second formal has no explicit
    // direction and inherits `ref` (§13.3), so it must also write back.
    check(
        "t6_direction_inherit",
        r#"
class C;
   function int accum(ref int acc, int add);
      acc = acc + add;
      return acc;
   endfunction
endclass
module top;
   initial begin
      C c = new;
      int acc = 100;
      int v1 = c.accum(acc, 5);
      int v2 = c.accum(acc, 7);
      $display("R6 %0d %0d %0d", v1, v2, acc);
   end
endmodule
"#,
    );

    // T7: inout-style accumulation from a compiled CALLER passing its own
    // local (arg-slot write-back at the caller side).
    check(
        "t7_compiled_caller_wb",
        r#"
class D;
   function int bump(ref int x);
      x = x * 3;
      return x;
   endfunction
endclass
class E;
   D d;
   function new();
      d = new;
   endfunction
   function int go(ref int y);
      return d.bump(y);
   endfunction
endclass
module top;
   initial begin
      E e = new;
      int y = 14;
      int rv = e.go(y);
      $display("R7 rv=%0d y=%0d", rv, y);
   end
endmodule
"#,
    );
}
