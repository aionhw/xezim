//! IEEE 1800-2017 §22.3: `` `resetall `` is illegal only inside a design
//! element. A module declared on one line (`module empty; endmodule`) was
//! still counted as open, so a `` `resetall `` after it was rejected (#600,
//! reported by amal-khailtash). Every closing keyword on a line now counts.
use std::process::Command;

fn preprocess(tag: &str, src: &str) -> (bool, String) {
    let dir =
        std::env::temp_dir().join(format!("xezim_resetall_600_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("repro.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--preprocess")
        .arg(&path)
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn resetall_after_modules_closed_on_their_own_line() {
    for (tag, src) in [
        (
            "one",
            "module empty; endmodule\n`resetall\nmodule top;\n  initial $finish;\nendmodule\n",
        ),
        (
            "two",
            "module e1; endmodule module e2; endmodule\n`resetall\nmodule top; endmodule\n",
        ),
        (
            "icls",
            "interface class ic; endclass\n`resetall\nmodule top; endmodule\n",
        ),
    ] {
        let (ok, err) = preprocess(tag, src);
        assert!(ok, "{tag}: {err}");
    }
}

/// Still an error inside a module, whatever a comment or string says.
#[test]
fn resetall_inside_a_module_is_still_an_error() {
    for (tag, src) in [
        ("plain", "module m;\n`resetall\nendmodule\n"),
        ("cmt", "module m; // endmodule\n`resetall\nendmodule\n"),
        (
            "str",
            "module m; string s = \"endmodule\";\n`resetall\nendmodule\n",
        ),
    ] {
        let (ok, err) = preprocess(tag, src);
        assert!(
            !ok && err.contains("`resetall is illegal inside a design element"),
            "{tag}: {err}"
        );
    }
}
