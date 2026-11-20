//! §23.3.3: a port connection is warned about when its width differs from
//! the port's. §7.4.3 and §11.5.1: a part-select after an index into a
//! multi-dimensional packed array selects elements of the next dimension,
//! so `.o(a[2][4:2])` on `logic [3:0][5:0][7:0] a` is three 8-bit elements,
//! 24 bits, exactly the port's width. Elaboration counted the labels (3)
//! and warned "port width mismatch … the connection is 3 bit(s)" on every
//! such connection. A connection that really is
//! narrower is still reported.

use std::process::Command;

const SRC: &str = r#"
module drv (output logic [2:0][7:0] o);
  assign o = 24'hc3c2c1;
endmodule
module snk (input logic [2:0][7:0] i, output logic [23:0] y);
  assign y = i;
endmodule
module short_drv (output logic [2:0][7:0] o);
  assign o = 24'hd3d2d1;
endmodule
module tb;
  logic [3:0][5:0][7:0] a, s;
  logic [5:0][7:0] u [2];
  logic [23:0] y0, y1;
  drv d0 (.o(a[2][4:2]));
  snk s0 (.i(a[1][3:1]), .y(y0));
  snk s1 (.i(u[1][5:3]), .y(y1));
  short_drv d1 (.o(s[0][4:3]));
endmodule
"#;

#[test]
fn range_after_index_connection_has_the_port_width() {
    let dir = std::env::temp_dir().join(format!("xezim_pcw_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("pcw.sv");
    std::fs::write(&src, SRC).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--compile", "-s", "tb", src.to_str().unwrap(), "--no-cache"])
        .output()
        .expect("run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "xezim --compile failed ({}):\n{text}",
        out.status
    );

    let mismatches: Vec<&str> = text
        .lines()
        .filter(|l| l.contains("port width mismatch"))
        .collect();
    assert_eq!(
        mismatches.len(),
        1,
        "only the 16-bit connection to short_drv is a mismatch:\n{text}"
    );
    assert!(
        mismatches[0].contains("'short_drv'") && mismatches[0].contains("16 bit(s)"),
        "the real mismatch is reported with the connection's width:\n{text}"
    );
}
