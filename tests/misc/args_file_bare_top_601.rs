//! A bare design-unit name inside a `-f` argument file names a top, as it
//! does on the command line (the help lists it among the spellings accepted
//! inside `-f` files). It was taken for a file name: "file 'small_top' not
//! found" (#601, reported by amal-khailtash).
use std::process::Command;

#[test]
fn bare_top_name_in_an_args_file() {
    let dir = std::env::temp_dir().join(format!("xezim_args_top_601_{}", std::process::id()));
    let sub = dir.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(
        sub.join("top.sv"),
        "module small_top;\n  initial begin $display(\"RAN\"); $finish; end\nendmodule\n\
         module other; initial $display(\"OTHER\"); endmodule\n",
    )
    .unwrap();
    std::fs::write(sub.join("args.f"), "top.sv\nsmall_top\n").unwrap();
    std::fs::write(dir.join("lib.f"), "sub/top.sv\nwork.small_top\n").unwrap();
    let run = |cwd: &std::path::Path, f: &str| {
        let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
            .args(["--no-cache", "-f", f])
            .current_dir(cwd)
            .output()
            .expect("run xezim");
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
    };
    for (cwd, f) in [(&sub, "args.f"), (&dir, "sub/args.f"), (&dir, "lib.f")] {
        let text = run(cwd, f);
        assert!(
            text.contains("RAN") && !text.contains("OTHER"),
            "{f}: {text}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
