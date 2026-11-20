//! Issue #283: IEEE 1800-2023 §11.13 lets a `let` be declared in a module,
//! interface, program, checker, clocking block, package, compilation-unit
//! scope, generate block, sequential or parallel block, or subroutine. A
//! `let` in a subroutine body or a block was rejected at parse time
//! ("expected expression, found KwLet").
//!
//! A let instance is the let body with each formal replaced by its actual
//! (a typed formal casts the actual to its type; an omitted actual takes the
//! default), and the whole body takes part in the context width of the
//! expression around the instance. A let in a block hides an outer one of
//! the same name and is not visible outside the block. Every expected value
//! below comes from the reference simulator.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|") || l.starts_with("PASS") || l.starts_with("FAIL"))
        .collect()
}

fn sim_error(src: &str) -> String {
    match simulate(src, 100) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e,
    }
}

/// The issue's reproducer: a `let` in a function body.
#[test]
fn issue_283_reproducer() {
    assert_eq!(
        t_lines(
            r#"
// let_block.sv
module top;
    function automatic void f(output bit ok);
        let max(a, b) = (a > b) ? a : b;
        ok = (max(1, 2) == 2);
    endfunction

    initial begin
        bit ok;
        f(ok);
        if (ok) $display("PASS: let inside a function body");
        else    $display("FAIL: let inside a function body");
    end
endmodule
"#
        ),
        ["PASS: let inside a function body"],
    );
}

/// Every block and subroutine place: function, task, class method,
/// `initial` block, nested and named blocks, a fork; shadowing; defaults,
/// typed formals and the context width of the instance.
#[test]
fn let_in_subroutines_and_blocks() {
    assert_eq!(
        t_lines(
            r#"
module top;
  let gmax(a, b) = (a > b) ? a : b;
  let gdef(x, y = 8'd5) = x + y;
  let gtyped(logic [3:0] a, logic [3:0] b) = a + b;
  let gsum = 100;
  int k = 7;
  let usek(x) = x + k;
  function automatic int f(int p);
    let max(a, b) = (a > b) ? a : b;
    let addp(a) = a + p;
    int loc = 3;
    let useloc = loc * 2;
    return max(p, 10) + addp(1) + useloc;
  endfunction
  task automatic t(output int r);
    let sq(a) = a * a;
    r = sq(5);
  endtask
  class C;
    int m = 4;
    function int meth(int z);
      let twice(a) = a * 2;
      let pm = m + z;
      return twice(z) + pm;
    endfunction
  endclass
  int r;
  C c;
  initial begin
    let ib(a) = a - 1;
    let sh = 11;
    $display("T|f=%0d", f(3));
    t(r); $display("T|t=%0d", r);
    c = new; $display("T|meth=%0d", c.meth(5));
    $display("T|ib=%0d", ib(9));
    begin
      let sh = 22;
      $display("T|inner sh=%0d", sh);
      begin
        int sh2;
        sh2 = sh + 1;
        $display("T|inner2 sh2=%0d", sh2);
      end
    end
    $display("T|outer sh=%0d", sh);
    fork
      begin
        let pf(a) = a + 1000;
        $display("T|fork pf=%0d", pf(1));
      end
    join
    $display("T|gmax=%0d gdef1=%0d gdef2=%0d", gmax(3, 4), gdef(1), gdef(1, 2));
    $display("T|gtyped=%0d", gtyped(4'd15, 4'd2));
    $display("T|gtyped_wide=%0d", gtyped(20, 1));
    $display("T|gsum=%0d usek=%0d", gsum, usek(1));
    begin : nb
      let ld(a, b = 3) = a * b;
      let lt(bit [2:0] a) = a;
      $display("T|ld1=%0d ld2=%0d lt=%0d", ld(2), ld(2, 4), lt(13));
    end
    // width: untyped let body context-determined
    begin
      logic [7:0] x8;
      let add8(a, b) = a + b;
      logic [3:0] p4, q4;
      p4 = 15; q4 = 1;
      x8 = add8(p4, q4);
      $display("T|add8=%0d", x8);
      $display("T|add8self=%0d", $bits(add8(p4, q4)));
    end
  end
endmodule
"#
        ),
        [
            "T|f=20",
            "T|t=25",
            "T|meth=19",
            "T|ib=8",
            "T|inner sh=22",
            "T|inner2 sh2=23",
            "T|outer sh=11",
            "T|fork pf=1001",
            "T|gmax=4 gdef1=6 gdef2=3",
            "T|gtyped=1",
            "T|gtyped_wide=5",
            "T|gsum=100 usek=8",
            "T|ld1=6 ld2=8 lt=5",
            "T|add8=16",
            "T|add8self=4",
        ],
    );
}

/// Package, interface and generate-block lets; named and empty actuals;
/// a let calling a let; a port, a block variable and a loop variable that
/// hide a module-scope let of the same name.
#[test]
fn let_in_design_scopes_and_hiding() {
    assert_eq!(
        t_lines(
            r#"
package p;
  let pk(a) = a + 1;
  function automatic int pf(int x);
    return pk(x) * 2;
  endfunction
endpackage
interface ifc;
  logic [7:0] d = 8'd3;
  let dbl = d * 2;
  function automatic int get(); return dbl; endfunction
endinterface
module top;
  import p::*;
  ifc u();
  let gf(logic [3:0] a, b) = a + b;
  let nm(a, b = 2, c = 3) = a * 100 + b * 10 + c;
  let nest(x) = nm(x) + 1;
  let cmp(a, b) = a < b;
  int r;
  genvar gi;
  for (gi = 0; gi < 2; gi++) begin : g
    let gl(x) = x + gi * 10;
    int v;
    initial v = gl(5);
  end
  if (1) begin : gb
    let gk = 77;
    int w;
    initial w = gk;
  end
  function automatic int shadow_port(int nm);
    return nm + 1;
  endfunction
  initial begin
    #1;
    $display("T|gf=%0d", gf(4'd15, 4'd2));
    $display("T|nm=%0d %0d %0d %0d", nm(1), nm(1, 5), nm(.a(1), .c(9)), nm(1, , 7));
    $display("T|nest=%0d", nest(4));
    $display("T|pf=%0d pk=%0d", pf(3), p::pk(10));
    $display("T|ifc=%0d", u.get());
    $display("T|g0=%0d g1=%0d gb=%0d", g[0].v, g[1].v, gb.w);
    $display("T|shadow_port=%0d", shadow_port(5));
    $display("T|cmp=%0d", cmp(-1, 1));
    begin
      int nm;
      nm = 4;
      $display("T|shadow_var=%0d", nm);
    end
    for (int nm = 0; nm < 1; nm++) $display("T|shadow_for=%0d", nm);
    begin
      let mul(a, b) = a * b;
      r = mul(2 + 3, 4);
      $display("T|mul=%0d", r);
    end
  end
endmodule
"#
        ),
        [
            "T|gf=1",
            "T|nm=123 153 129 127",
            "T|nest=424",
            "T|pf=8 pk=11",
            "T|ifc=6",
            "T|g0=5 g1=15 gb=77",
            "T|shadow_port=6",
            "T|cmp=1",
            "T|shadow_var=4",
            "T|shadow_for=0",
            "T|mul=20",
        ],
    );
}

/// Lets the simulator expands at run time: `pkg::name`, an explicit
/// `import pkg::name`, and a compilation-unit let, with defaults, a typed
/// formal and the context width.
#[test]
fn let_from_package_and_compilation_unit() {
    assert_eq!(
        t_lines(
            r#"
package p;
  let pk(a) = a + 1;
  let pd(a, b = 5) = a * b;
endpackage
let ul(x, int y = 2) = x - y;
module top;
  import p::pd;
  logic [3:0] a4 = 15, b4 = 1;
  logic [7:0] r8;
  initial begin
    $display("T|pk=%0d pd=%0d pd2=%0d", p::pk(10), pd(3), p::pd(3, 2));
    $display("T|ul=%0d ul2=%0d", ul(10), ul(10, 4));
    r8 = p::pk(a4);
    $display("T|ctx=%0d", r8);
  end
endmodule
"#
        ),
        ["T|pk=11 pd=15 pd2=6", "T|ul=8 ul2=6", "T|ctx=16"],
    );
}

/// Checker, program and clocking-block lets, and a let in a fork inside a
/// task that calls a module-scope let. (Sorted: the program's `initial`
/// runs in the reactive region, so its line order is not what this checks.)
#[test]
fn let_in_checker_program_clocking_and_fork() {
    let mut got = t_lines(
        r#"
checker chk(logic a, event clk);
  let both(x) = x && a;
  initial assert (both(1'b1)) $display("T|chk both=%0d", both(1'b1));
endchecker
program prg;
  let pl(x) = x * 3;
  initial $display("T|prg pl=%0d", pl(4));
endprogram
module top;
  logic clk = 0, a = 1;
  clocking cb @(posedge clk);
    let cl(x) = x + 40;
  endclocking
  let lp(x) = x + 1;
  task automatic tk(output int r);
    fork
      begin
        let inner = lp(5) * 2;
        r = inner;
      end
    join
  endtask
  chk c1(a, clk);
  prg p1();
  int r;
  initial begin
    tk(r);
    $display("T|tk=%0d", r);
  end
endmodule
"#,
    );
    got.sort();
    assert_eq!(got, ["T|chk both=1", "T|prg pl=12", "T|tk=12"]);
}

/// A let declared in a block is not visible after the block.
#[test]
fn block_let_is_local_to_its_block() {
    let e = sim_error(
        r#"
module top;
  initial begin
    begin
      let q = 1;
    end
    $display("T|q=%0d", q);
  end
endmodule
"#,
    );
    assert!(e.contains("'q'"), "{e}");
}

/// A formal with no actual and no default is an error.
#[test]
fn missing_let_actual_is_an_error() {
    let e = sim_error(
        r#"
module top;
  initial begin
    let two(a, b) = a + b;
    $display("T|x=%0d", two(1));
  end
endmodule
"#,
    );
    assert!(e.contains("missing argument for formal 'b'"), "{e}");
}

/// §11.13 does not list class scope.
#[test]
fn let_in_class_scope_is_rejected() {
    assert!(
        simulate(
            r#"
module top;
  class C;
    let bad = 1;
  endclass
  initial $display("T|x");
endmodule
"#,
            100
        )
        .is_err()
    );
}
