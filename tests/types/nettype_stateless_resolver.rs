//! IEEE 1800-2023 §6.6.7: "A resolution function shall be automatic (or
//! preserve no state information) and have no side effects." A function
//! declared in a module or package is static by default (§13.4.2), so a
//! static resolver that writes every static variable before reading it in
//! the same call is accepted, and one that reads a value left by the previous
//! call is rejected (#1483).

const SUM_BODY: &str = r#"
    res = 0.0;
    foreach (d[k]) res += d[k];
"#;

const NET_AND_DRIVERS: &str = r#"
  nettype real rn with res;
  rn n;
  real driver;
  assign n = 1.25;
  assign n = driver;
  initial begin
    driver = 2.0;
    #1 $display("n=%0.2f", n);
    #1 driver = 2.75;
    #1 $display("n=%0.2f", n);
  end
endmodule
"#;

fn module_src(module_lifetime: &str, function_lifetime: &str, body: &str) -> String {
    format!(
        r#"`timescale 1ns/1ps
module {module_lifetime}top;
  function {function_lifetime}real res(input real d[]);
{body}
  endfunction
{NET_AND_DRIVERS}"#
    )
}

fn package_src(package_lifetime: &str) -> String {
    format!(
        r#"`timescale 1ns/1ps
package {package_lifetime}rp;
  function real res(input real d[]);
{SUM_BODY}
  endfunction
endpackage
import rp::*;
module top;
{NET_AND_DRIVERS}"#
    )
}

/// Each driver change resolves the current values: 1.25 + 2.0, then
/// 1.25 + 2.75 (values checked on the reference simulator).
fn check_resolution(src: &str) {
    let sim = xezim::simulate(src, 10_000).expect("simulate resolved net");
    let lines: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert_eq!(lines, ["n=3.25", "n=4.00"], "{src}");
}

fn rejection(src: &str) -> String {
    match xezim::simulate(src, 10_000) {
        Ok(_) => panic!("a stateful static resolver was accepted:\n{src}"),
        Err(e) => e,
    }
}

fn check_rejected(body: &str, carried: &str) {
    let src = module_src("", "", body);
    let err = rejection(&src);
    assert!(
        err.contains("is static and reads") && err.contains(carried),
        "{err}\n{src}"
    );
}

#[test]
fn module_static_by_default() {
    check_resolution(&module_src("", "", SUM_BODY));
}

#[test]
fn package_static_by_default() {
    check_resolution(&package_src(""));
}

#[test]
fn explicit_static_with_reset_local() {
    check_resolution(&module_src(
        "",
        "static ",
        r#"
    real total;
    total = 0.0;
    foreach (d[k]) total += d[k];
    return total;
"#,
    ));
}

#[test]
fn explicit_automatic() {
    check_resolution(&module_src("", "automatic ", SUM_BODY));
}

#[test]
fn automatic_from_module() {
    check_resolution(&module_src("automatic ", "", SUM_BODY));
}

#[test]
fn automatic_from_package() {
    check_resolution(&package_src("automatic "));
}

#[test]
fn static_writes_on_both_branches() {
    check_resolution(&module_src(
        "",
        "",
        r#"
    if (d.size() == 0) res = 0.0;
    else begin
      res = d[0];
      for (int i = 1; i < d.size(); i++) res += d[i];
    end
"#,
    ));
}

#[test]
fn static_writes_in_every_case_item() {
    check_resolution(&module_src(
        "",
        "",
        r#"
    case (d.size())
      0: res = 0.0;
      default: begin res = 0.0; foreach (d[k]) res += d[k]; end
    endcase
"#,
    ));
}

#[test]
fn static_calls_stateless_static_function() {
    let src = module_src(
        "",
        "",
        r#"
    real t;
    t = 0.0;
    foreach (d[k]) t += twice(d[k]);
    res = t / 2.0;
"#,
    )
    .replace(
        "  function real res",
        "  function real twice(input real x);\n    twice = 2.0 * x;\n  endfunction\n  function real res",
    );
    check_resolution(&src);
}

#[test]
fn static_struct_resolver_writes_whole_or_each_member() {
    for reset in ["res = '{0.0, 0};", "res.v = 0.0;\n    res.c = 0;"] {
        let src = format!(
            r#"`timescale 1ns/1ps
typedef struct {{ real v; int c; }} rs_t;
module top;
  function rs_t res(input rs_t d[]);
    {reset}
    foreach (d[k]) begin res.v += d[k].v; res.c += d[k].c; end
  endfunction
  nettype rs_t rn with res;
  rn n;
  assign n = '{{1.25, 1}};
  assign n = '{{2.0, 2}};
  initial #1 $display("n=%0.2f %0d", n.v, n.c);
endmodule
"#
        );
        let sim = xezim::simulate(&src, 10_000).expect("simulate resolved net");
        let lines: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
        assert_eq!(lines, ["n=3.25 3"], "{src}");
    }
}

#[test]
fn rejects_accumulating_return_variable() {
    // The reference simulator accepts this and resolves 3.25, then 7.25 by
    // adding to the previous result; §6.6.7 forbids the carried state.
    check_rejected("    foreach (d[k]) res += d[k];\n", "return variable 'res'");
}

#[test]
fn rejects_reset_on_one_branch_only() {
    check_rejected(
        "    if (d.size() > 5) res = 0.0;\n    foreach (d[k]) res += d[k];\n",
        "return variable 'res'",
    );
}

#[test]
fn rejects_local_written_only_inside_loop() {
    // With no drivers the loop body never runs and `t` returns its old value.
    check_rejected(
        r#"
    real t;
    foreach (d[k]) begin
      if (k == 0) t = d[k]; else t = t + d[k];
    end
    return t;
"#,
        "'t' of 'res'",
    );
}

#[test]
fn rejects_explicit_static_local_carried_between_calls() {
    check_rejected(
        r#"
    static real seen = 0.0;
    res = seen;
    foreach (d[k]) res += d[k];
    seen = res;
"#,
        "'seen' of 'res'",
    );
}

#[test]
fn rejects_static_callee_with_state() {
    let src = module_src("", "", SUM_BODY.replace("res += d[k]", "res += keep(d[k])").as_str())
        .replace(
            "  function real res",
            "  function real keep(input real x);\n    real last;\n    keep = x + last - last;\n    last = x;\n  endfunction\n  function real res",
        );
    let err = rejection(&src);
    assert!(err.contains("'last' of 'keep'"), "{err}\n{src}");
}
