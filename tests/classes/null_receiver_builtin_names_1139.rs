//! §8.4, §8.20 (#1139): a non-virtual method called through a null handle
//! runs with a null `this` also when it is named like a built-in array
//! method (`size`, `num`, `delete`), when the call is a static property
//! initializer, and when the receiver is the iterator of an array method's
//! `with` clause. The expected lines are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class pkt_c;
  function int hdr_len(); return 42; endfunction
endclass
class vsize_c;
  function int size(); return 42; endfunction
  function int num(); return 7; endfunction
  function void delete(); $display("user delete ran"); endfunction
endclass
class vstat_c;
  static pkt_c sp;
  static int y = sp.hdr_len();
endclass
module top;
  pkt_c vq[$];
  vsize_c vsz; vsize_c live = new;
  int r;
  initial begin
    vq.push_back(null);
    $display("V2 %0d live %0d num %0d", vsz.size(), live.size(), vsz.num());
    vsz.delete();
    $display("V7 %0d", vstat_c::y);
    $display("V8 %0d", vq.sum() with (int'(item.hdr_len())));
    r = vq.sum() with (item.hdr_len()); $display("V8b %0d", r);
    $finish;
  end
endmodule
"#;

const EXPECTED: &[&str] = &[
    "V2 42 live 42 num 7",
    "user delete ran",
    "V7 42",
    "V8 42",
    "V8b 42",
];

fn run(compile: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "xezim_null_builtin_{}_{}",
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
fn nonvirtual_call_through_null_in_builtin_named_static_init_and_with() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let got: Vec<&str> = log.lines().filter(|l| EXPECTED.contains(l)).collect();
        assert_eq!(got, EXPECTED, "XEZIM_COMPILE_METHODS={compile}:\n{log}");
    }
}
