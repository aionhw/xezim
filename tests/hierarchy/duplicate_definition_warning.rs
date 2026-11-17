//! §3.13, §25.3: the definitions name space (modules, interfaces, programs,
//! primitives) warns when a name is redefined and lets the later definition
//! overwrite the earlier one. Distinct name spaces (package + interface,
//! class + module, `interface class` + `interface`) and un-adopted `-v`
//! library definitions stay silent.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xezim_dup_def_warn_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

fn run_sv(tag: &str, files: &[(&str, &str)], extra_args: &[&str]) -> (bool, String, String) {
    let dir = scratch(tag);
    let mut paths = Vec::new();
    for (name, body) in files {
        let p = dir.join(name);
        std::fs::write(&p, body).expect("write sv");
        paths.push(p);
    }
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.args(["--simulate", "--no-cache"]);
    for p in &paths {
        cmd.arg(p);
    }
    cmd.args(extra_args);
    let out = cmd.output().expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn duplicate_interface_program_primitive_and_cross_kind_warn_and_last_wins() {
    let (ok, stdout, stderr) = run_sv(
        "iface",
        &[(
            "dup_iface.sv",
            "interface bus_if; logic a; initial $display(\"iface#1\"); endinterface\n\
             interface bus_if; logic a; logic b; initial $display(\"iface#2\"); endinterface\n\
             module tb; bus_if u(); initial #1 $display(\"done\"); endmodule\n",
        )],
        &[],
    );
    assert!(ok, "stderr:\n{stderr}");
    assert!(
        stdout.contains("iface#2") && !stdout.contains("iface#1"),
        "{stdout}"
    );
    assert!(
        stderr.contains("[xezim][warning] interface 'bus_if' redefined"),
        "{stderr}"
    );

    let (ok, stdout, stderr) = run_sv(
        "prog",
        &[(
            "dup_prog.sv",
            "program pg; initial $display(\"pg#1\"); endprogram\n\
             program pg; initial $display(\"pg#2\"); endprogram\n\
             module tb; pg u(); endmodule\n",
        )],
        &[],
    );
    assert!(ok, "stderr:\n{stderr}");
    assert!(
        stdout.contains("pg#2") && !stdout.contains("pg#1"),
        "{stdout}"
    );
    assert!(
        stderr.contains("[xezim][warning] program 'pg' redefined"),
        "{stderr}"
    );

    let (ok, stdout, stderr) = run_sv(
        "udp",
        &[(
            "dup_udp.sv",
            "primitive u(output o, input a); table 0:1; 1:0; endtable endprimitive\n\
             primitive u(output o, input a); table 0:0; 1:1; endtable endprimitive\n\
             module tb; wire o; reg a=0; u g(o,a); initial #1 $display(\"o=%b\", o); endmodule\n",
        )],
        &[],
    );
    assert!(ok, "stderr:\n{stderr}");
    assert!(stdout.contains("o=0"), "{stdout}");
    assert!(
        stderr.contains("[xezim][warning] primitive 'u' redefined"),
        "{stderr}"
    );

    let (ok, stdout, stderr) = run_sv(
        "cross",
        &[(
            "dup_iface_mod.sv",
            "interface x; logic a; initial $display(\"iface x\"); endinterface\n\
             module x; initial $display(\"module x\"); endmodule\n\
             module tb; x u(); initial #1 $display(\"done\"); endmodule\n",
        )],
        &[],
    );
    assert!(ok, "stderr:\n{stderr}");
    assert!(
        stdout.contains("module x") && !stdout.contains("iface x"),
        "{stdout}"
    );
    assert!(
        stderr.contains("[xezim][warning] module 'x' redefines earlier interface 'x'"),
        "{stderr}"
    );
}

#[test]
fn distinct_namespaces_and_unadopted_library_interface_stay_silent() {
    let (ok, _stdout, stderr) = run_sv(
        "pkg_if",
        &[(
            "pkg_if.sv",
            "package x; localparam int K = 7; endpackage\n\
             interface x; logic a; endinterface\n\
             module tb; x xi(); initial $display(\"K=%0d\", x::K); endmodule\n",
        )],
        &[],
    );
    assert!(ok, "stderr:\n{stderr}");
    assert!(
        !stderr.contains("redefined") && !stderr.contains("redefines"),
        "{stderr}"
    );

    let (ok, stdout, stderr) = run_sv(
        "cls_mod",
        &[(
            "cls_mod.sv",
            "class cm; int a = 9; endclass\n\
             module cm; initial $display(\"mod cm\"); endmodule\n\
             module tb; cm u(); cm h = new; initial #1 $display(\"a=%0d\", h.a); endmodule\n",
        )],
        &[],
    );
    assert!(ok, "stderr:\n{stderr}");
    assert!(
        stdout.contains("mod cm") && stdout.contains("a=9"),
        "{stdout}"
    );
    assert!(
        !stderr.contains("redefined") && !stderr.contains("redefines"),
        "{stderr}"
    );

    let (ok, stdout, stderr) = run_sv(
        "ic_if",
        &[(
            "ic_if.sv",
            "interface class ic; pure virtual function int f(); endclass\n\
             interface ic; logic a; initial $display(\"if ic\"); endinterface\n\
             class impl implements ic; virtual function int f(); return 4; endfunction endclass\n\
             module tb; ic u(); impl o = new; initial #1 $display(\"f=%0d\", o.f()); endmodule\n",
        )],
        &[],
    );
    assert!(ok, "stderr:\n{stderr}");
    assert!(
        stdout.contains("if ic") && stdout.contains("f=4"),
        "{stdout}"
    );
    assert!(
        !stderr.contains("redefined") && !stderr.contains("redefines"),
        "{stderr}"
    );

    let dir = scratch("lib_if");
    let main_p = dir.join("main_if.sv");
    let lib_p = dir.join("lib.v");
    std::fs::write(
        &main_p,
        "interface bus_if; logic a; initial $display(\"main bus_if\"); endinterface\n\
         module tb; bus_if u(); lib_m l(); endmodule\n",
    )
    .unwrap();
    std::fs::write(
        &lib_p,
        "interface bus_if; logic a; initial $display(\"LIB bus_if\"); endinterface\n\
         typedef logic [3:0] lt;\n\
         package p; endpackage\n\
         module lib_m; initial $display(\"lib_m\"); endmodule\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "--no-cache"])
        .arg(&main_p)
        .arg("-v")
        .arg(&lib_p)
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "stderr:\n{stderr}");
    assert!(
        stdout.contains("main bus_if") && stdout.contains("lib_m"),
        "{stdout}"
    );
    assert!(!stdout.contains("LIB bus_if"), "{stdout}");
    assert!(
        !stderr.contains("redefined") && !stderr.contains("redefines"),
        "{stderr}"
    );
}
