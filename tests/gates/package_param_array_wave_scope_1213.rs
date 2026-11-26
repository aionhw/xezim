//! IEEE 1800-2023 §26.3, §21.7.2.3: the elements of a package's unpacked
//! array parameter belong to the package, not to the module that reads them.
//! A module that reached `other::LIM[1]` by its qualified name, without an
//! import, still dumped `LIM[0]` and `LIM[1]` among its own variables (#1213).
//! The module's own declarations stay: a local array parameter, and a local
//! variable named like a parameter of a package the module never imported.
use std::{fs, process::Command};

#[test]
fn qualified_package_array_parameter_is_not_dumped_in_module_scope() {
    let dir = std::env::temp_dir().join(format!("xezim_pkg_array_wave_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let src = r#"
`timescale 1ns/1ps
package other;
  int s;
  localparam int LIM [0:1] = '{3, 4};
  localparam int K [0:1] = '{5, 6};
endpackage
package third;
  localparam int T [0:1] = '{7, 8};
endpackage
module top;
  import third::*;
  int s;
  int r;
  int K [0:1];
  localparam int OWN [0:1] = '{1, 2};
  initial begin
    $dumpfile("out.vcd");
    $dumpvars(0, top);
    K[0] = 1;
    #1 s = 3; other::s = 33; r = other::LIM[1] + other::K[1] + T[0] + OWN[1];
    #1 $display("s=%0d other::s=%0d r=%0d K0=%0d", s, other::s, r, K[0]);
    $finish;
  end
endmodule
"#;
    let file = dir.join("top.sv");
    fs::write(&file, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&dir)
        .args(["--no-cache", "--wave", "--simulate", "-s", "top"])
        .arg(&file)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("s=3 other::s=33 r=19 K0=1"),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut names = Vec::new();
    let mut scopes: Vec<String> = Vec::new();
    for line in fs::read_to_string(dir.join("out.vcd")).unwrap().lines() {
        let p: Vec<_> = line.split_whitespace().collect();
        match p.first().copied() {
            Some("$scope") => scopes.push(p[2].to_string()),
            Some("$upscope") => {
                scopes.pop();
            }
            Some("$var") => names.push(format!("{}.{}", scopes.join("."), p[4])),
            _ => {}
        }
    }
    for required in [
        "top.K[0]",
        "top.K[1]",
        "top.OWN[0]",
        "top.OWN[1]",
        "top.r",
        "top.s",
    ] {
        assert!(
            names.iter().any(|n| n == required),
            "missing {required}: {names:?}"
        );
    }
    assert!(
        !names
            .iter()
            .any(|n| n.starts_with("top.LIM[") || n.starts_with("top.T[") || n.contains("::LIM")),
        "{names:?}"
    );
    let _ = fs::remove_dir_all(dir);
}
