//! IEEE 1800-2023 §13.5.2 / §23.6 / §26.3: an `output`/`inout`/`ref`
//! unpacked-array formal whose actual is a hierarchical instance or
//! interface path (`u.A`, `m.s.A`, `i.IA`) or a package-qualified array
//! (`pk::PA`) writes back to that array. The class-property actual
//! resolution (`fill(prop)`, `c.prop`, `a.b.prop`) must leave these
//! spellings on their plain hierarchical name; it once returned no storage
//! for them, so the call bound the formal as a scalar and the output was
//! lost. Expected lines are the reference simulator's output.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn instance_path_array_actuals_write_back() {
    let out = t_lines(
        r#"
module sub;
  int A[string];
  int F[2];
endmodule
module top;
  sub u();
  function automatic void fa(output int p[string]); p["h"] = 8; endfunction
  function automatic void ff(output int p[2]); p[0] = 1; p[1] = 2; endfunction
  function automatic void ia(inout int p[string]); p["i"] = 9; endfunction
  initial begin
    fa(u.A); ff(u.F);
    $display("T| b3 u.A=%p u.F=%p", u.A, u.F);
    ia(u.A);
    $display("T| b3 u.A=%p", u.A);
    fa(top.u.A);
    $display("T| b3 top.u.A num=%0d", top.u.A.num());
  end
endmodule
"#,
    );
    let want = [
        "T| b3 u.A='{\"h\":8} u.F='{1, 2}",
        "T| b3 u.A='{\"h\":8, \"i\":9}",
        "T| b3 top.u.A num=1",
    ];
    assert_eq!(out, want, "{out:?}");
}

#[test]
fn nested_instance_and_interface_array_actuals_write_back() {
    let out = t_lines(
        r#"
interface ifc; int IA[string]; int IF[2]; endinterface
module sub; int A[string]; int F[2]; endmodule
module mid; sub s(); endmodule
module top;
  mid m();
  ifc i();
  function automatic void fa(output int p[string]); p["h"] = 8; endfunction
  function automatic void ff(output int p[2]); p[0] = 1; p[1] = 2; endfunction
  initial begin
    fa(m.s.A); ff(m.s.F);
    $display("T| d3 m.s.A num=%0d F=%0d %0d", m.s.A.num(), m.s.F[0], m.s.F[1]);
    fa(i.IA); ff(i.IF);
    $display("T| d3 i.IA num=%0d IF=%0d %0d", i.IA.num(), i.IF[0], i.IF[1]);
  end
endmodule
"#,
    );
    let want = ["T| d3 m.s.A num=1 F=1 2", "T| d3 i.IA num=1 IF=1 2"];
    assert_eq!(out, want, "{out:?}");
}

#[test]
fn package_array_actuals_write_back() {
    let out = t_lines(
        r#"
package pk; int PA[string]; int PF[2]; endpackage
module top;
  import pk::*;
  function automatic void ga(output int p[string]); p["m"] = 1; endfunction
  function automatic void gf(output int p[2]); p[0] = 3; p[1] = 4; endfunction
  initial begin
    ga(pk::PA); gf(pk::PF);
    $display("T| d4 PA num=%0d PF=%0d %0d", pk::PA.num(), pk::PF[0], pk::PF[1]);
    ga(PA);
    $display("T| d4 imported PA num=%0d", PA.num());
  end
endmodule
"#,
    );
    let want = ["T| d4 PA num=1 PF=3 4", "T| d4 imported PA num=1"];
    assert_eq!(out, want, "{out:?}");
}

/// The same spellings through a blocking task and a class method.
#[test]
fn hier_and_package_actuals_through_tasks_and_methods() {
    let out = t_lines(
        r#"
package pk; int PA[int]; int PF[2]; endpackage
interface ifc; int IA[string]; int IF[3]; endinterface
module sub; int A[string]; int F[2]; endmodule
class K;
  function void fa(output int p[string]); p["k"] = 1; endfunction
  function void ff(ref int p[2]); p[1] = 77; endfunction
endclass
module top;
  sub u();
  ifc i();
  task automatic ta(output int p[string]); #1; p["t"] = 2; endtask
  task automatic tf(inout int p[3]); #1; p[2] = p[2] + 5; endtask
  task automatic tp(output int p[int]); p[-2] = 4; endtask
  initial begin
    K k = new;
    ta(u.A); k.fa(i.IA); k.ff(u.F); tf(i.IF); tp(pk::PA); k.ff(pk::PF);
    $display("T| e3 u.A num=%0d t=%0d F1=%0d", u.A.num(), u.A["t"], u.F[1]);
    $display("T| e3 i.IA num=%0d k=%0d IF2=%0d", i.IA.num(), i.IA["k"], i.IF[2]);
    $display("T| e3 PA=%p PF1=%0d", pk::PA, pk::PF[1]);
  end
endmodule
"#,
    );
    let want = [
        "T| e3 u.A num=1 t=2 F1=77",
        "T| e3 i.IA num=1 k=1 IF2=5",
        "T| e3 PA='{-2:4} PF1=77",
    ];
    assert_eq!(out, want, "{out:?}");
}
