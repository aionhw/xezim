//! IEEE 1800-2023 §6.6.7 resolves a user-defined nettype from the complete
//! value of each driver. An unpacked structure can be passed as a whole
//! (§7.2.2), and a continuous assignment drives the entire atomic net (§10.3.2).

use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use xezim::simulate;

fn check_output(src: &str, expected: &[&str]) {
    let sim = simulate(src, 10_000).expect("simulate failed");
    let lines: Vec<_> = sim.output.iter().map(|out| out.message.trim()).collect();
    assert_eq!(lines, expected);
}

#[test]
fn nested_struct_driver() {
    check_output(
        r#"`timescale 1ns/1ps
module top;
  typedef struct { real v; } I;
  typedef struct { I i; } S;

  function automatic S resolve(input S drivers[]);
    resolve.i.v = 0.0;
    foreach (drivers[k])
      resolve.i.v += drivers[k].i.v;
  endfunction

  nettype S N with resolve;
  N n;
  S x;
  assign n = x;

  initial begin
    x.i.v = 7.0;
    #1 $display("v=%g", n.i.v);
    $finish;
  end
endmodule
"#,
        &["v=7"],
    );
}

/// The fixed-size array member belongs to the nested driver value (§7.4.2).
#[test]
fn nested_real_array_driver() {
    check_output(
        r#"`timescale 1ns/1ps
module top;
  typedef struct { real a[1]; } I;
  typedef struct { I i; } S;
  function automatic S resolve(input S drivers[]);
    resolve.i.a[0] = 0.0;
    foreach (drivers[k])
      resolve.i.a[0] += drivers[k].i.a[0];
  endfunction
  nettype S N with resolve;
  N n;
  S x;
  assign n = x;

  initial begin
    x.i.a[0] = 7.0;
    #1 $display("a=%g", n.i.a[0]);
    $finish;
  end
endmodule
"#,
        &["a=7"],
    );
}

#[test]
fn two_nested_struct_levels() {
    check_output(
        r#"`timescale 1ns/1ps
module top;
  typedef struct { real v; } I;
  typedef struct { I i; } M;
  typedef struct { M m; } S;
  function automatic S resolve(input S drivers[]);
    resolve.m.i.v = 0.0;
    foreach (drivers[k])
      resolve.m.i.v += drivers[k].m.i.v;
  endfunction
  nettype S N with resolve;
  N n;
  S x;
  assign n = x;

  initial begin
    x.m.i.v = 7.0;
    #1 $display("v=%g", n.m.i.v);
    $finish;
  end
endmodule
"#,
        &["v=7"],
    );
}

/// Both drivers contribute to the sum when either driver changes (§6.6.7).
#[test]
fn nested_struct_drivers_resolve_again_after_an_update() {
    check_output(
        r#"`timescale 1ns/1ps
module top;
  typedef struct { real v; } I;
  typedef struct { I i; } S;
  function automatic S resolve(input S drivers[]);
    resolve.i.v = 0.0;
    foreach (drivers[k])
      resolve.i.v += drivers[k].i.v;
  endfunction
  nettype S N with resolve;
  N n;
  S x, y;
  assign n = x;
  assign n = y;

  initial begin
    x.i.v = 7.0;
    y.i.v = 2.25;
    #1 $display("v=%g", n.i.v);
    #1 x.i.v = 8.5;
    #1 $display("v=%g", n.i.v);
    $finish;
  end
endmodule
"#,
        &["v=9.25", "v=10.75"],
    );
}

#[test]
fn flat_real_struct_driver() {
    check_output(
        r#"`timescale 1ns/1ps
module top;
  typedef struct { real v; } S;
  function automatic S resolve(input S drivers[]);
    resolve.v = 0.0;
    foreach (drivers[k])
      resolve.v += drivers[k].v;
  endfunction
  nettype S N with resolve;
  N n;
  S x;
  assign n = x;

  initial begin
    x.v = 7.0;
    #1 $display("v=%g", n.v);
    $finish;
  end
endmodule
"#,
        &["v=7"],
    );
}

/// Either spelling of the package item supplies the same struct value
/// to the named member of the pattern (§10.9.2, §26.3).
#[test]
#[ignore = "open: the members of a package unpacked-structure variable are not reachable from a module that imports the package with a wildcard, also on main (§26.3, §7.2)"]
fn package_struct_items_in_driver_patterns() {
    check_output(
        r#"`timescale 1ns/1ps
package pk;
  typedef struct { real v; } I;
  I py;
endpackage

module top;
  import pk::*;
  typedef struct { I i; real w; } S;
  function automatic S resolve(input S drivers[]);
    resolve.i.v = 0.0;
    resolve.w = 0.0;
    foreach (drivers[k]) begin
      resolve.i.v += drivers[k].i.v;
      resolve.w += drivers[k].w;
    end
  endfunction
  nettype S N with resolve;
  N nimp, nqual;
  assign nimp = '{i: py, w: 1.0};
  assign nqual = '{i: pk::py, w: 1.0};

  initial begin
    py.v = 14;
    #1 $display("imp=%g qual=%g", nimp.i.v, nqual.i.v);
    #1 py.v = 20;
    #1 $display("imp=%g qual=%g", nimp.i.v, nqual.i.v);
    $finish;
  end
endmodule
"#,
        &["imp=14 qual=14", "imp=20 qual=20"],
    );
}

/// Replication supplies the same real to both struct members (§10.9.2).
#[test]
fn scalar_replication_in_a_flat_driver_pattern() {
    check_output(
        r#"`timescale 1ns/1ps
module top;
  typedef struct { real i; real j; } S;
  function automatic S resolve(input S drivers[]);
    resolve.i = drivers[0].i;
    resolve.j = drivers[0].j;
  endfunction
  nettype S N with resolve;
  N n;
  real y;
  assign n = '{2{y}};

  initial begin
    y = 7.0;
    #1 $display("v=%g %g", n.i, n.j);
    #1 y = 8.5;
    #1 $display("v=%g %g", n.i, n.j);
    $finish;
  end
endmodule
"#,
        &["v=7 7", "v=8.5 8.5"],
    );
}

/// The resolver copies the reals and the four-state member (§6.6.7, §7.4.2).
/// The VCD lists the six design leaves selected by $dumpvars (§21.7.1.2).
#[test]
fn flat_array_struct_nettype_dumps_only_design_variables() {
    let src = r#"`timescale 1ns/1ps
module top;
  typedef struct { real a[2]; logic [3:0] l; } S;
  function automatic S resolve(input S drivers[]);
    resolve.a[0] = drivers[0].a[0];
    resolve.a[1] = drivers[0].a[1];
    resolve.l = drivers[0].l;
  endfunction
  nettype S N with resolve;
  N n;
  S x;
  assign n = x;

  initial begin
    $dumpfile("dump.vcd");
    $dumpvars(0, top);
    x.a[0] = 7.0;
    x.a[1] = 8.0;
    x.l = 4'b1010;
    #1 x.l = 4'b10xz;
    #1 $display("v=%g %g %b", n.a[0], n.a[1], n.l);
    $finish;
  end
endmodule
"#;

    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "xezim-nettype-wave-{}-{}",
        std::process::id(),
        nonce.as_nanos()
    ));
    fs::create_dir(&dir).expect("create VCD test directory");
    let source = dir.join("top.sv");
    let dump = dir.join("dump.vcd");
    fs::write(&source, src).expect("write VCD fixture");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&dir)
        .args(["--no-cache", "--wave", "top.sv"])
        .output()
        .expect("run xezim");
    let vcd = fs::read_to_string(&dump);
    fs::remove_file(source).expect("remove VCD fixture");
    if dump.exists() {
        fs::remove_file(dump).expect("remove VCD dump");
    }
    fs::remove_dir(dir).expect("remove VCD test directory");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.lines().any(|line| line.trim() == "v=7 8 10xz"));

    let vcd = vcd.expect("read VCD dump");
    let mut scopes = Vec::new();
    let mut vars = Vec::new();
    for line in vcd.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        match fields.as_slice() {
            ["$scope", _, name, ..] => scopes.push(*name),
            ["$upscope", ..] => {
                scopes.pop();
            }
            ["$var", _, _, _, name, ..] => {
                vars.push(format!("{}.{name}", scopes.join(".")));
            }
            ["$enddefinitions", ..] => break,
            _ => {}
        }
    }
    vars.sort_unstable();
    assert_eq!(
        vars,
        [
            "top.n.a[0]",
            "top.n.a[1]",
            "top.n.l",
            "top.x.a[0]",
            "top.x.a[1]",
            "top.x.l",
        ]
    );
}
