//! A compiled method lowers string methods on a string field read through
//! a handle (`d.name.len()`, `h.name.len()`, `p.q[i].name.substr(a, b)`)
//! and treats such a field as a string operand. Each made the whole method
//! decline, which kept loops over a pattern's contents interpreted.
//! Expected T| lines are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class pdata;
  typedef enum { NONE, REF, DEEP } own_e;
  string name;
  own_e ownership_how;
  int width;
  function new(string n, own_e o = REF);
    name = n; ownership_how = o; width = n.len();
  endfunction
endclass

class sub extends pdata;
  function new(string n, own_e o = REF); super.new(n, o); endfunction
endclass

class pattern;
  pdata contents[$];
endclass

class item;
  pdata h;
  string name = "own";
  // Literals of another class, through the class and a subclass.
  function bit owned(pattern p, int ix); return p.contents[ix].ownership_how != pdata::NONE; endfunction
  function bit deep(pdata d); return d.ownership_how == sub::DEEP; endfunction
  function int as_int(); return pdata::DEEP + 1; endfunction
  // String methods on a string field read through a handle.
  function int nlen(pdata d); return d.name.len(); endfunction
  function int hlen(); return h.name.len(); endfunction
  function bit suffix(pattern p, int ix, string sfx);
    return p.contents[ix].name.substr(p.contents[ix].name.len() - sfx.len(), p.contents[ix].name.len() - 1) == sfx;
  endfunction
  function string stem(pattern p, int ix, string sfx);
    return p.contents[ix].name.substr(0, p.contents[ix].name.len() - sfx.len() - 1);
  endfunction
  function string up(pdata d); return d.name.toupper(); endfunction
  function int cnt(pattern p, string sfx);
    cnt = 0;
    foreach (p.contents[i])
      if (p.contents[i].ownership_how == pdata::NONE)
        continue;
      else if (p.contents[i].name.substr(p.contents[i].name.len() - sfx.len(), p.contents[i].name.len() - 1) == sfx)
        cnt += p.contents[i].name.len();
  endfunction
endclass

module top;
  initial begin
    item it;
    pattern p;
    pdata d;
    sub s;
    it = new;
    p = new;
    d = new("alpha_size"); p.contents.push_back(d);
    d = new("beta", pdata::NONE); p.contents.push_back(d);
    s = new("gamma_size", pdata::DEEP); p.contents.push_back(s);
    d = new("_size"); p.contents.push_back(d);
    it.h = s;
    $display("T|own=%0d%0d%0d deep=%0d%0d int=%0d", it.owned(p, 0), it.owned(p, 1), it.owned(p, 2), it.deep(p.contents[0]), it.deep(s), it.as_int());
    $display("T|len=%0d %0d suffix=%0d%0d%0d%0d", it.nlen(d), it.hlen(), it.suffix(p, 0, "_size"), it.suffix(p, 1, "_size"), it.suffix(p, 3, "_size"), it.suffix(p, 0, "alpha_size_x"));
    $display("T|stem=%s/%s/[%s] up=%s cnt=%0d", it.stem(p, 0, "_size"), it.stem(p, 2, "_size"), it.stem(p, 3, "_size"), it.up(s), it.cnt(p, "_size"));
  end
endmodule
"#;

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_string_member_methods_{}_{}",
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
fn string_methods_on_member_fields_compile() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("T|")).collect();
        assert_eq!(
            lines,
            [
                "T|own=101 deep=01 int=3",
                "T|len=5 10 suffix=1010",
                "T|stem=alpha/gamma/[] up=GAMMA_SIZE cnt=25",
            ],
            "XEZIM_COMPILE_METHODS={compile}"
        );
    }
    let log = run("1");
    for m in [
        "item.nlen",
        "item.hlen",
        "item.suffix",
        "item.stem",
        "item.up",
    ] {
        assert!(
            log.contains(&format!("runs-compiled scope={m}")),
            "{m} did not run compiled:\n{log}"
        );
    }
}
