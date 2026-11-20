//! §23.3.2, §23.3.3: a port connection is checked when the instance is
//! elaborated. Another top-level module that `-s` did not select, and what
//! only it instantiates, is never elaborated, so a connection to a port the
//! target does not declare there is not an error. xezim rejected the run
//! ("port 'no_such_port' is not a port of module 'leaf'"); Verilator runs it.
//! A top that is elaborated, selected or found automatically, is still
//! checked.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

const SRC: &str = r#"
module leaf(input a, output y);
  assign y = a;
endmodule
module mid;
  logic a, y;
  leaf u(.a(a), .y(y), .also_missing(1'b0));
endmodule
module tb;
  logic a = 1, y;
  leaf u(.a(a), .y(y));
  initial begin #1 $display("tb: y=%0d", y); $finish; end
endmodule
module other_tb;
  logic a, y;
  leaf u(.a(a), .y(y), .no_such_port(1'b0));
  mid m();
endmodule
"#;

fn run(args: &[&str]) -> (i32, String) {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "unelaborated_top_ports_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("t.sv"), SRC).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&dir)
        .args(["--no-cache", "t.sv"])
        .args(args)
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr),
    )
}

#[test]
fn a_top_that_is_not_selected_has_its_port_connections_unchecked() {
    let (code, out) = run(&["-s", "tb"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("tb: y=1"), "{out}");
    assert!(!out.contains("is not a port of module"), "{out}");
}

#[test]
fn an_elaborated_top_still_has_its_port_connections_checked() {
    for args in [
        &["-s", "other_tb"][..],
        &["-s", "tb", "-s", "other_tb"][..],
        &[][..],
    ] {
        let (code, out) = run(args);
        assert_ne!(code, 0, "{args:?}: {out}");
        assert!(
            out.contains("port 'no_such_port' is not a port of module 'leaf'")
                && out.contains("port 'also_missing' is not a port of module 'leaf'"),
            "{args:?}: {out}"
        );
    }
}
