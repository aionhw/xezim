//! §23.11 + §23.3.2 — a `bind` whose target PATH traverses an instance whose
//! module type comes from a `-v` library file.
//!
//! Binds were applied before `-v`/`-y` library modules were adopted, so the
//! path walker hit a half-populated definition map: the intermediate module
//! was "not a module", the bind was dropped with a warning, and the checker
//! never instantiated. The identical source passes when the library file is
//! handed over as an ordinary source, which is what made this look like a path
//! bug rather than an ordering one (xezim bind.path.inst.bug report; the
//! commercial reference accepts both invocations).
//!
//! Fixed by DEFERRING unresolvable binds to a retry after library adoption.
//! The first attempt is quiet so a library-provided target does not warn
//! spuriously; the retry reports anything still unresolvable — pinned below,
//! because a fix that merely silenced the diagnostic would also pass the
//! positive cases.

use std::process::Command;

fn xezim_bin() -> std::path::PathBuf {
    // Cargo names the binary it built for the integration tests.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_xezim"))
}

const LIB: &str = r#"
module dsp_mem_bfm_inst(input logic clk, output logic ready);
    mem_subunit mem_subunit ();
    assign ready = 1'b1;
endmodule
module mem_subunit(input logic clk);
    logic dummy;
    assign dummy = 1'b0;
endmodule
"#;

const TB: &str = r#"
module testbench;
    logic clk;
    initial clk = 0;
    always #5 clk = ~clk;
    dsp_mem_bfm_inst dsp_mem_bfm(.clk(clk), .ready());
    initial begin
        #100;
        $display("TB_DONE");
        $finish;
    end
endmodule
// Printing from the bound instance is what proves it was INSTANTIATED, and
// `%m` proves it landed at the right hierarchy path. (An upward hierarchical
// WRITE from the bound instance is a separate mechanism and would confound
// this test.)
module checker_mod;
    initial $display("BIND_OK %m");
endmodule
bind testbench.dsp_mem_bfm.mem_subunit checker_mod chk();
"#;

/// A bind path crossing a library-provided module type resolves the same way
/// whether the library arrives as `-v` or as an ordinary source file.
#[test]
fn bind_path_through_v_library_module_resolves() {
    let dir = std::env::temp_dir().join("xezim_bind_lib_path");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let tb = dir.join("tb.sv");
    let lib = dir.join("lib.sv");
    std::fs::write(&tb, TB).expect("write tb");
    std::fs::write(&lib, LIB).expect("write lib");

    for as_library in [false, true] {
        let mut cmd = Command::new(xezim_bin());
        cmd.arg("--simulate").arg("-s").arg("testbench").arg(&tb);
        if as_library {
            cmd.arg("-v").arg(&lib);
        } else {
            cmd.arg(&lib);
        }
        let out = cmd.output().expect("run xezim");
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            combined.contains("BIND_OK testbench.dsp_mem_bfm.mem_subunit.chk"),
            "bind through a {} module did not instantiate:\n{combined}",
            if as_library { "-v library" } else { "source" }
        );
        assert!(
            !combined.contains("bind ignored"),
            "bind was dropped:\n{combined}"
        );
    }
}

/// A genuinely bad bind path must still report — the deferral makes the FIRST
/// attempt quiet, so the retry has to carry the diagnostic.
#[test]
fn unresolvable_bind_path_still_reports() {
    let dir = std::env::temp_dir().join("xezim_bind_lib_path");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let bad = dir.join("bad.sv");
    std::fs::write(
        &bad,
        "module chk; endmodule\nmodule sub; endmodule\n\
         module tb; sub u_s(); initial #1 $finish; endmodule\n\
         bind tb.u_nosuch chk c();\n",
    )
    .expect("write");
    let out = Command::new(xezim_bin())
        .arg("--simulate")
        .arg("-s")
        .arg("tb")
        .arg(&bad)
        .output()
        .expect("run xezim");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("bind ignored"),
        "an unresolvable bind path was silently dropped:\n{combined}"
    );
}
