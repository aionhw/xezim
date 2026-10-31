//! A compiled method's `this.name = name` — a string member stored through
//! `this` while a same-named formal shadows the bare name — stores to the
//! member's heap slot, like the dotted string read already did. The store
//! declined, so constructors that copy their arguments into string members
//! never compiled. Expected T| lines are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class pd;
  typedef enum {UNDEF, INT, STR} type_enum;
  string name;
  bit [1023:0] value;
  int array_ix;
  type_enum typ;
  string owner;
  function new(string name, bit [1023:0] value, int array_ix = 0, type_enum typ = UNDEF, string owner = "");
    this.name = name;
    this.value = value;
    this.array_ix = array_ix;
    this.typ = typ;
    this.owner = owner;
  endfunction
  function void rename(string name, string owner);
    this.name = {name, "_r"};
    this.owner = owner;
  endfunction
  function string show();
    return $sformatf("%s/%s/%0d/%0d/%0d", name, owner, value[15:0], array_ix, typ);
  endfunction
endclass
module top;
  initial begin
    automatic pd p;
    automatic string acc = "";
    for (int i = 0; i < 4; i++) begin
      p = new($sformatf("f%0d", i), i * 3, i % 3, pd::STR, "own");
      if (i == 2) p.rename("g", "other");
      acc = {acc, p.show(), ";"};
    end
    $display("T|%s", acc);
  end
endmodule
"#;

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_string_member_store_{}_{}",
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
fn this_dotted_string_member_store_compiles() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("T|")).collect();
        assert_eq!(
            lines,
            ["T|f0/own/0/0/2;f1/own/3/1/2;g_r/other/6/2/2;f3/own/9/0/2;"],
            "XEZIM_COMPILE_METHODS={compile}"
        );
    }
    let log = run("1");
    for m in ["pd.new", "pd.rename"] {
        assert!(
            log.contains(&format!("runs-compiled scope={m}")),
            "{m} did not run compiled:\n{log}"
        );
    }
}
