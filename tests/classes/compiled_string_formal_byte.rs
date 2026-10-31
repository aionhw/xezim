//! §6.16: `s[i]` on a string FORMAL of a compiled method reads byte i of
//! the bound text, 0 out of range, as on a string local. The select made
//! the whole method decline compilation, so a character-scanning loop over
//! a name argument (`while (s[k] != pat[0]) k++;`) always ran interpreted.
//! An index that reads the formal only through `.len()` (`s[s.len()-1]`)
//! is integral and compiles too.
//! Expected T| lines are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class splitter;
  function int find(string s, string pat);
    int pos = -1;
    int k = 0;
    while ((k < s.len()) && (pos < 0)) begin
      if (s[k] == pat[0]) pos = k;
      k++;
    end
    return pos;
  endfunction
  function int count_dots(string s);
    int n = 0;
    for (int i = 0; i < s.len(); i++) if (s[i] == ".") n++;
    return n;
  endfunction
  function int edges(string s);
    return (s[0] == 0) + 2 * (s[s.len()] == 0) + 4 * (s[-1] == 0) + 8 * (s[1] == "b");
  endfunction
  function void split(string s, ref string parts[$]);
    string rest = s;
    parts.delete();
    while (rest.len() > 0) begin
      int p = find(rest, ".");
      if (p > 0) begin
        parts.push_back(rest.substr(0, p - 1));
        rest = rest.substr(p + 1, rest.len() - 1);
      end else if (p == 0) begin
        parts.push_back("");
        rest = rest.substr(1, rest.len() - 1);
      end else begin
        parts.push_back(rest);
        rest = "";
      end
    end
  endfunction
endclass
module top;
  initial begin
    automatic splitter sp = new;
    automatic string parts[$];
    $display("T|find %0d %0d %0d %0d", sp.find("a.b", "."), sp.find("abc", "."), sp.find("", "."), sp.find(".x", "."));
    $display("T|dots %0d %0d %0d", sp.count_dots("a.b.c"), sp.count_dots(""), sp.count_dots("..."));
    $display("T|edges %0d %0d", sp.edges("ab"), sp.edges(""));
    sp.split("cfg.port.width", parts);
    $display("T|split %0d %s %s %s", parts.size(), parts[0], parts[1], parts[2]);
    sp.split("single", parts);
    $display("T|split1 %0d %s", parts.size(), parts[0]);
  end
endmodule
"#;

const EXPECTED: [&str; 5] = [
    "T|find 1 -1 -1 0",
    "T|dots 2 0 3",
    "T|edges 14 7",
    "T|split 3 cfg port width",
    "T|split1 1 single",
];

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_string_formal_byte_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, DESIGN).expect("write design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_COMPILE_METHODS", compile)
        .env("XEZIM_METHOD_TIER", "0")
        .env("XEZIM_FALLBACK_SITES", "1")
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
fn string_formal_byte_select_compiles() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("T|")).collect();
        assert_eq!(lines, EXPECTED, "XEZIM_COMPILE_METHODS={compile}");
    }
    let log = run("1");
    for m in ["splitter.find", "splitter.count_dots", "splitter.edges"] {
        assert!(
            log.contains(&format!("runs-compiled scope={m}")),
            "{m} did not run compiled:\n{log}"
        );
    }
}
