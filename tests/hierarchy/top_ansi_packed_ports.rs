//! IEEE 1800-2023 §7.4.1, §7.2, §23.2.2.2: an ANSI port of the top module
//! has the packed shape of its declared type. An element select of a packed
//! multi-D port read one bit, a member of a packed-structure port read x, and
//! a part-select connected to an instance was sized as bits, not elements.

use std::path::Path;
use std::process::Command;

fn run(name: &str, source: &str) -> String {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.sv"));
    std::fs::write(&path, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--no-cache", "-s", "top"])
        .arg(path)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "simulation failed:\n{text}");
    text
}

#[test]
fn top_module_ansi_ports_keep_their_packed_shape() {
    let src = r#"
typedef logic [7:0] byte_t;
typedef struct packed { logic [3:0] hi; logic [3:0] lo; } s_t;
module sink(input logic [15:0] a);
  initial #2 $display("T|%m a=%h", a);
endmodule
module top(output logic [5:0][7:0] w2, output byte_t [5:0] w, output s_t s,
           output s_t [1:0] sa, output logic [0:3][3:0] asc,
           output logic [3:0][5:0][7:0] w3);
  sink u(w3[1][3:2]);
  sink v(w2[3:2]);
  initial begin
    w2 = 48'h0a0b0c0d0e0f; w = 48'h0a0b0c0d0e0f; s = 8'h5a; sa = 16'h1234;
    asc = 16'habcd; w3 = {4{48'h1a1b1c1d1e1f}};
    w2[4] = 8'hee; s.lo = 4'h3;
    #1 $display("T|w2[1]=%h w2[3:2]=%h w2[4]=%h w[1]=%h s.hi=%h s=%h sa[1].lo=%h asc[0]=%h asc[1:2]=%h",
                w2[1], w2[3:2], w2[4], w[1], s.hi, s, sa[1].lo, asc[0], asc[1:2]);
  end
endmodule
"#;
    let text = run("top_ansi_packed_ports", src);
    for want in [
        "T|w2[1]=0e w2[3:2]=0c0d w2[4]=ee w[1]=0e s.hi=5 s=53 sa[1].lo=2 asc[0]=a asc[1:2]=bc",
        "T|top.u a=1c1d",
        "T|top.v a=0c0d",
    ] {
        assert!(
            text.lines().any(|line| line == want),
            "missing `{want}`:\n{text}"
        );
    }
    assert!(!text.contains("port width mismatch"), "{text}");
}
