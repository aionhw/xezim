//! §7.4.1 (#1107): an element of a packed array of a named signed type is
//! signed when the array is a class property (`h.m_amb[1]`, or `m_amb[1]`
//! in a method) and when it is an input port of a child module, as it is
//! for a module variable. The reference simulator reads the port element
//! unsigned (1686); the expected lines follow §7.4.1 ("unsigned unless
//! they are of a named type declared as signed").
use std::process::Command;

const DESIGN: &str = r#"
package mp;
  typedef logic signed [10:0] lane_t;
endpackage
class hold_c;
  mp::lane_t [0:8] m_amb;
  function int own(); return 1 * m_amb[1]; endfunction
endclass
module child (input mp::lane_t [-2:2] p, output int w, output int c);
  assign w = 1 * p[1];
  always_comb c = p[1] + 0;
  initial #2 $display("child proc=%0d", 1 * p[1]);
endmodule
module top;
  import mp::*;
  lane_t [-2:2] tab;
  hold_c h = new;
  int w, c, v;
  child u (.p(tab), .w(w), .c(c));
  initial begin
    tab = '0; tab[1] = -362;
    h.m_amb = '0; h.m_amb[1] = -362;
    v = 1 * h.m_amb[1];
    #1 $display("prop=%0d own=%0d cmp=%0d", v, h.own(), h.m_amb[1] < 0);
    #2 $display("port assign=%0d comb=%0d", w, c);
    $finish;
  end
endmodule
"#;

const EXPECTED: &[&str] = &[
    "prop=-362 own=-362 cmp=1",
    "child proc=-362",
    "port assign=-362 comb=-362",
];

fn run(compile: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "xezim_sgn_props_{}_{}",
        std::process::id(),
        compile
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.sv");
    std::fs::write(&path, DESIGN).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_COMPILE_METHODS", compile)
        .env_remove("XEZIM_METHOD_CACHE")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    text
}

#[test]
fn named_signed_elements_of_properties_and_ports_are_signed() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let got: Vec<&str> = log.lines().filter(|l| EXPECTED.contains(l)).collect();
        assert_eq!(got, EXPECTED, "XEZIM_COMPILE_METHODS={compile}:\n{log}");
    }
}
