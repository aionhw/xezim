//! Syntax 21-20 (§21.7.2.1, not in Annex A) uses an identifier for a VCD
//! reference. An unescaped p::W is a scoped name (§5.6, §5.6.1);
//! §21.7.2.3 describes the identifier as the variable name in the model.
//! Package (§26), compilation-unit (§3.12.1) and class (§8.3, §8.23) constants
//! must not become module-owned waveform leaves through lookup aliases.

use std::collections::BTreeMap;
use std::fs;
use std::io::BufReader;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use fst_reader::{
    FstFilter, FstHierarchyEntry, FstReader, FstScopeType, FstSignalValue, FstVarType,
};

const PACKAGE: &str = r#"
`timescale 1ns/1ps
package p;
  parameter int W = 4;
  localparam int PL = 6;
endpackage
module leaf;
  import p::*;
  logic [W-1:0] y = 0;
  initial #2 y = PL;
endmodule
module top;
  import p::*;
  logic [W-1:0] x = 0;
  leaf u();
  initial begin
    $dumpfile("out.vcd"); $dumpvars(0, top);
    #1 x = PL;
    #2 $dumpall;
    #1 $finish;
  end
endmodule
"#;

const SHADOW: &str = r#"
`timescale 1ns/1ps
package p;
  parameter int W = 4;
  localparam int PL = 6;
endpackage
module leaf #(parameter int W = 1);
  localparam int L = W + 1;
  logic [W-1:0] y = 0;
  initial #2 y = '1;
endmodule
module top;
  import p::*;
  parameter int W = 9;
  localparam int L = W + 1;
  logic [W-1:0] x = 0;
  leaf #(.W(p::W + 1)) u();
  // §5.6.1 / §27.5: escaped scope names do not qualify W, L or M.
  leaf #(.W(4)) \u::x ();
  if (1) begin : \c::d
    localparam int M = 2;
  end
  initial begin
    $dumpfile("out.vcd"); $dumpvars(0, top);
    #1 x = L;
    #2 $dumpall;
    #1 $finish;
  end
endmodule
"#;

const OTHER_SCOPES: &str = r#"
`timescale 1ns/1ps
parameter int U = 7;
int v = 8;
package pkg;
  localparam int LIM[2] = '{3, 5};
  int pv = 2;
endpackage
class C;
  parameter int P = 5;
  localparam int Q = 6;
  static int member = 3;
endclass
module constant_leaf;
  class C; parameter int P = 8; endclass
  int y = 0;
  initial #2 y = C::P;
endmodule
module top;
  parameter int U = 11;
  int v = 12, pv = 13;
  int a = $unit::U;
  int b;
  int c = C::P + C::Q;
  int d;
  int e = pkg::LIM[0] + pkg::LIM[1];
  int f;
  // §23.3.3.5: brackets in instance paths do not index the constant.
  constant_leaf la[1:0]();
  initial begin
    b = $unit::v; d = C::member; f = pkg::pv;
    $dumpfile("out.vcd"); $dumpvars(0, top);
    #1 a = a + 1;
    #2 $dumpall;
    #1 $finish;
  end
endmodule
"#;

const VARIABLE_SHADOW: &str = r#"
`timescale 1ns/1ps
parameter int U = 7;
package p;
  parameter int W = 4;
  localparam logic [3:0] PL = 4'd6;
endpackage
class C;
  parameter int P = 5;
endclass
module leaf;
  int W = 50;
  logic [p::W-1:0] y = 0;
  initial #2 y = p::PL;
endmodule
module top;
  int W = 100, P = 102, U = 103;
  logic [3:0] PL = 4'd1;
  logic [p::W-1:0] x = 0;
  int e = p::W + p::PL + C::P + $unit::U;
  leaf u();
  initial begin
    $dumpfile("out.vcd"); $dumpvars(0, top);
    #1 x = p::PL; W = W + 1;
    #2 $display("e=%0d x=%0d W=%0d P=%0d U=%0d PL=%0d y=%0d", e, x, W, P, U, PL, u.y);
    $finish;
  end
endmodule
"#;

struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// (kind, width, optional range), indexed by the full scope and reference.
type Declarations = BTreeMap<String, (String, u32, String)>;

// Ignore repeated checkpoint values; preserve every actual value transition.
fn record(values: &mut BTreeMap<String, Vec<(u64, String)>>, name: &str, time: u64, bits: &str) {
    let bits = bits.trim_start_matches('0');
    let bits = if bits.is_empty() { "0" } else { bits };
    let history = values.entry(name.into()).or_default();
    if history.last().is_none_or(|(_, prior)| prior != bits) {
        history.push((time, bits.into()));
    }
}

fn check(src: &str, parameter_as_wire: bool) -> (Declarations, String, String) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = Temp(std::env::temp_dir().join(format!(
        "xezim_dump_qualified_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    let _ = fs::remove_dir_all(&dir.0);
    fs::create_dir(&dir.0).unwrap();
    fs::write(dir.0.join("dut.sv"), src).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_xezim"));
    command.env_remove("XEZIM_VCD_PARAM_AS_WIRE");
    if parameter_as_wire {
        command.env("XEZIM_VCD_PARAM_AS_WIRE", "1");
    }
    let out = command
        .current_dir(&dir.0)
        .args([
            "--no-cache",
            "--wave",
            "--fst",
            "out.fst",
            "--xtrace",
            "out.xt",
            "-s",
            "top",
        ])
        .arg("dut.sv")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let vcd = fs::read_to_string(dir.0.join("out.vcd")).unwrap();
    let xt = fs::read_to_string(dir.0.join("out.xt")).unwrap();
    let mut scopes = Vec::new();
    let mut declarations = Declarations::new();
    let mut ids = BTreeMap::new();
    let mut values = BTreeMap::new();
    let mut time = 0;
    for line in vcd.lines() {
        let words: Vec<_> = line.split_whitespace().collect();
        match words.first().copied() {
            Some("$scope") => {
                assert_eq!(words[1], "module");
                scopes.push(words[2].to_string());
            }
            Some("$upscope") => {
                scopes.pop().expect("balanced scopes");
            }
            Some("$var") => {
                ids.insert(
                    words[3].to_string(),
                    format!("{}.{}", scopes.join("."), words[4]),
                );
                assert!(
                    !(words[1] == "parameter" && words[4].contains("::")),
                    "{line}"
                );
                declarations.insert(
                    format!("{}.{}", scopes.join("."), words[4]),
                    (
                        words[1].to_string(),
                        words[2].parse().unwrap(),
                        words[5..words.len() - 1].join(" "),
                    ),
                );
            }
            Some(word) if word.starts_with('#') => time = word[1..].parse().unwrap(),
            Some(word) if word.starts_with('b') => {
                record(&mut values, &ids[words[1]], time, &word[1..]);
            }
            _ => {}
        }
    }
    assert!(scopes.is_empty());
    assert!(declarations.contains_key("top.a") || declarations.contains_key("top.x"));
    let mut fst = FstReader::open_and_read_time_table(BufReader::new(
        fs::File::open(dir.0.join("out.fst")).unwrap(),
    ))
    .unwrap();
    let mut fst_declarations = Declarations::new();
    let mut handles = BTreeMap::new();
    fst.read_hierarchy(|entry| match entry {
        FstHierarchyEntry::Scope { name, tpe, .. } => {
            assert_eq!(tpe, FstScopeType::Module);
            scopes.push(name);
        }
        FstHierarchyEntry::UpScope => {
            scopes.pop().expect("balanced FST scopes");
        }
        FstHierarchyEntry::Var {
            name,
            tpe,
            length,
            handle,
            ..
        } => {
            assert!(
                !(tpe == FstVarType::Parameter && name.contains("::")),
                "{name}"
            );
            let (name, range) = name.split_once(' ').unwrap_or((&name, ""));
            handles.insert(handle.get_index(), format!("{}.{}", scopes.join("."), name));
            let kind = if tpe == FstVarType::Parameter {
                "parameter"
            } else {
                "reg"
            };
            fst_declarations.insert(
                format!("{}.{}", scopes.join("."), name),
                (kind.into(), length, range.into()),
            );
        }
        _ => {}
    })
    .expect("decode FST hierarchy");
    assert!(scopes.is_empty());
    // Only VCD applies the optional parameter-as-wire conversion.
    if parameter_as_wire {
        for (kind, _, _) in fst_declarations.values_mut() {
            if kind == "parameter" {
                *kind = "wire".into();
            }
        }
    }
    // These designs contain parameters and procedural variables only.
    assert_eq!(declarations, fst_declarations);
    let mut fst_values = BTreeMap::new();
    fst.read_signals(&FstFilter::all(), |time, handle, value| {
        if let FstSignalValue::String(bits) = value {
            record(
                &mut fst_values,
                &handles[&handle.get_index()],
                time,
                std::str::from_utf8(bits).unwrap(),
            );
        }
        Ok::<(), ()>(())
    })
    .expect("decode FST values");
    assert_eq!(values, fst_values);
    if src.contains("leaf #(.W") {
        for (name, value) in [
            ("top.W", "1001"),
            ("top.L", "1010"),
            ("top.u.W", "101"),
            ("top.u.L", "110"),
            ("top.u::x.W", "100"),
            ("top.u::x.L", "101"),
            ("top.c::d.M", "10"),
        ] {
            assert_eq!(values[name], [(0, value.to_string())]);
        }
        assert_eq!(values["top.x"], [(0, "0".into()), (1000, "1010".into())]);
        assert_eq!(values["top.u.y"], [(0, "0".into()), (2000, "11111".into())]);
    }
    for name in ["p::W", "p::PL", "$unit::U", "C::P", "C::Q"] {
        assert!(!declarations.contains_key(&format!("top.{name}")), "{name}");
        assert!(!xt.contains(name), "{xt}");
    }
    if src == OTHER_SCOPES {
        // §6.20: local U=11; §6.8: static variable initialization precedes
        // the initial process. Qualified-variable naming remains separate.
        for (name, value) in [("top.U", "1011"), ("top.v", "1100"), ("top.pv", "1101")] {
            assert_eq!(values[name], [(0, value.to_string())]);
        }
        for path in ["top.la[0]", "top.la[1]"] {
            assert!(declarations.iter().all(|(name, _)| {
                !(name.starts_with(&format!("{path}.")) && name[path.len() + 1..].contains("::"))
            }));
            assert_eq!(
                values[&format!("{path}.y")],
                [(0, "0".into()), (2000, "1000".into())]
            );
        }
    }
    if src == VARIABLE_SHADOW {
        assert!(
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .any(|line| line == "e=22 x=6 W=101 P=102 U=103 PL=1 y=6")
        );
        // §26.3 / §8.23 / §3.12.1: scoped constant lookup is independent
        // of same-named module variables. Their histories must survive.
        assert_eq!(
            values["top.W"],
            [(0, "1100100".into()), (1000, "1100101".into())]
        );
        for (name, value) in [("top.P", "1100110"), ("top.U", "1100111"), ("top.PL", "1")] {
            assert_eq!(values[name], [(0, value.to_string())]);
        }
    }
    (declarations, vcd, xt)
}

#[test]
fn package_and_child_import_aliases_are_not_module_parameters() {
    let child_only = PACKAGE.replacen("  import p::*;\n  logic [W-1:0] x", "  logic [3:0] x", 1);
    assert_ne!(child_only, PACKAGE, "remove only the top import");
    let child_only = child_only.replace("#1 x = PL;", "#1 x = 6;");
    assert!(!child_only.contains("#1 x = PL;"));
    for source in [PACKAGE.to_string(), child_only] {
        let (vars, vcd, xt) = check(&source, false);
        assert_eq!(vars["top.x"].1, 4);
        assert_eq!(vars["top.u.y"].1, 4);
        assert!(vcd.contains("$dumpall"));
        assert!(!xt.contains("p::W") && !xt.contains("p::PL"), "{xt}");
    }
}

#[test]
fn local_shadow_and_child_override_keep_their_parameter_declarations() {
    let (vars, _, xt) = check(SHADOW, false);
    // §26.3: the local W shadows the wildcard import; §6.20 and §23.10:
    // W=9, L=10 at top and overridden W=5, L=6 in the child.
    for name in [
        "top.W",
        "top.L",
        "top.u.W",
        "top.u.L",
        "top.u::x.W",
        "top.u::x.L",
        "top.c::d.M",
    ] {
        assert_eq!(vars[name], ("parameter".into(), 32, "[31:0]".into()));
    }
    assert_eq!(vars["top.x"], ("reg".into(), 9, "[8:0]".into()));
    assert_eq!(vars["top.u.y"].1, 5);
    assert!(!xt.contains("p::W"));
}

#[test]
fn unit_and_class_parameter_aliases_are_not_module_parameters() {
    let (vars, _, xt) = check(OTHER_SCOPES, false);
    assert_eq!(vars["top.U"].0, "parameter");
    for name in ["$unit::U", "C::P", "C::Q"] {
        assert!(!xt.contains(name), "{xt}");
    }
    for name in ["top.v", "top.pv"] {
        assert_eq!(vars[name].0, "reg");
    }
}

#[test]
fn same_named_module_variables_do_not_make_constant_aliases_dumpable() {
    check(VARIABLE_SHADOW, false);
}

#[test]
fn parameter_as_wire_still_omits_qualified_aliases() {
    let (vars, _, _) = check(SHADOW, true);
    for name in [
        "top.W",
        "top.L",
        "top.u.W",
        "top.u.L",
        "top.u::x.W",
        "top.u::x.L",
        "top.c::d.M",
    ] {
        assert_eq!(vars[name].0, "wire");
    }
}
