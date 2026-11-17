//! IEEE 1800-2023 §26.3 / §21.7.2.3: an import is not a local declaration.
use fst_reader::{FstHierarchyEntry, FstReader};
use std::{fs, io::BufReader, process::Command};
fn check(fst: bool, artifact: bool) {
    let dir = std::env::temp_dir().join(format!(
        "xezim_param_wave_{}_{}_{}",
        fst,
        artifact,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let src = r#"
 package p; parameter int W=4; localparam int A[2]='{3,4}; endpackage
 module child #(parameter W=5);
  import p::*;
  localparam L=W+1;
  if(1) begin:g localparam P=7; wire [P-1:0] x='0; end
  wire [W-1:0] x='0;
 endmodule
 module top;
  import p::*;
  localparam L=W+1;
  localparam int OWN[2]='{7,8};
  logic [W-1:0] cnt=0;
  child #(.W(p::W+1)) u();
  initial begin $dumpfile("out.vcd"); $dumpvars(0,top);
   #1 cnt=p::A[1]; #1 $display("PASS %0d",cnt); $finish; end
 endmodule
 "#;
    let file = dir.join("top.sv");
    fs::write(&file, src).unwrap();
    if artifact {
        let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
            .current_dir(&dir)
            .args(["--compile", "-s", "top", "-o", "design.xezbc"])
            .arg(&file)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.current_dir(&dir).args(["--no-cache", "--wave"]);
    if fst {
        cmd.args(["--fst", "out.fst"]);
    }
    if artifact {
        cmd.args(["--simulate", "design.xezbc"]);
    } else {
        cmd.args(["-s", "top"]).arg(&file);
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut names = Vec::new();
    let mut scopes = Vec::new();
    if fst {
        let mut r = FstReader::open_and_read_time_table(BufReader::new(
            fs::File::open(dir.join("out.fst")).unwrap(),
        ))
        .unwrap();
        r.read_hierarchy(|e| match e {
            FstHierarchyEntry::Scope { name, .. } => scopes.push(name),
            FstHierarchyEntry::UpScope => {
                scopes.pop();
            }
            FstHierarchyEntry::Var { name, .. } => names.push(format!(
                "{}.{}",
                scopes.join("."),
                name.split_whitespace().next().unwrap()
            )),
            _ => {}
        })
        .unwrap();
    } else {
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
    }
    for required in [
        "top.cnt",
        "top.L",
        "top.OWN[0]",
        "top.OWN[1]",
        "top.u.W",
        "top.u.L",
        "top.u.g.P",
    ] {
        assert!(
            names.iter().any(|s| s == required),
            "missing {required}: {names:?}"
        );
    }
    assert!(
        !names
            .iter()
            .any(|n| n.contains("::") || n == "top.W" || n.starts_with("top.A[")),
        "{names:?}"
    );
    let _ = fs::remove_dir_all(dir);
}
#[test]
fn vcd_lists_declarations() {
    check(false, false);
}
#[test]
fn fst_lists_declarations() {
    check(true, false);
}
#[test]
fn loaded_artifact_keeps_parameter_ownership() {
    check(false, true);
}
