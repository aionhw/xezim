//! §12.7.3: `foreach (p.contents[i])` over a collection member of ANOTHER
//! class, reached through a typed handle formal or local. The compiled
//! loop admitted only collections of the method's own class, so such a
//! method (and every statement in it) ran interpreted, and an
//! interpreted fallback of the loop was refused because the outer select
//! was counted against the handle `p`. Element facts (handle element,
//! member widths) come from the receiver's declared class, not from a
//! same-named member of the method's class. Expected T| lines are the
//! reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class pdata;
  string name;
  int    value;
  int    owner;
  bit [7:0] width;
  function new(string n, int o, bit [7:0] w = 8);
    name = n; owner = o; width = w;
  endfunction
  virtual function bit has_kw(string k);
    return name == k;
  endfunction
endclass

class sdata extends pdata;
  function new(string n, int o, bit [7:0] w = 8);
    super.new(n, o, w);
  endfunction
  virtual function bit has_kw(string k);
    return k == "stop" && owner == 2;
  endfunction
endclass

class pattern;
  pdata contents[$];
  int   tags[string];
  int   flags[];
endclass

class item;
  // Same names as pattern's collections, other types.
  bit [3:0] contents[$];
  bit [3:0] flags;
  int hits;
  int fail_at = -1;
  string log;
  function bit get_val(string n, output int v);
    v = n.len() * 3;
    return n != "bad";
  endfunction
  // ref handle formal, element member reads/writes, continue/break.
  function bit walk(ref pattern p);
    int pv;
    walk = 1;
    foreach (p.contents[ix]) begin
      if (p.contents[ix].owner == 0)
        continue;
      walk = get_val(p.contents[ix].name, pv);
      if (walk)
        p.contents[ix].value = pv + ix;
      if (p.contents[ix].owner > 1)
        log = {log, $sformatf("%0d:%s/%0d ", ix, p.contents[ix].name, p.contents[ix].width)};
      if (!walk) begin
        fail_at = ix;
        break;
      end
      if (p.contents[ix].has_kw("stop"))
        break;
      hits++;
    end
  endfunction
  // input handle formal, assoc and dynamic members of another class.
  function int sum(pattern p);
    sum = 0;
    foreach (p.tags[k])
      sum += p.tags[k] * k.len();
    foreach (p.flags[i])
      sum += p.flags[i] << i;
    foreach (p.contents[i])
      sum += p.contents[i].width;
    foreach (contents[i])
      sum += contents[i];
  endfunction
endclass

module top;
  function automatic void add(pattern p, string n, int o, bit [7:0] w = 8);
    pdata d = new(n, o, w);
    p.contents.push_back(d);
  endfunction
  initial begin
    item it;
    pattern p;
    sdata sd;
    int s;
    bit ok;
    it = new;
    p = new;
    it.contents.push_back(4'hf);
    it.contents.push_back(4'h3);
    add(p, "alpha", 1);
    add(p, "skip", 0, 4);
    add(p, "gamma", 2, 16);
    add(p, "stop", 1);
    add(p, "never", 1);
    p.tags["ab"] = 2;
    p.tags["xyz"] = 5;
    p.flags = new[3];
    p.flags[0] = 1; p.flags[1] = 3; p.flags[2] = 7;
    for (int r = 0; r < 3; r++) begin
      ok = it.walk(p);
      s = it.sum(p);
      $display("T|r=%0d ok=%0d hits=%0d fail_at=%0d sum=%0d", r, ok, it.hits, it.fail_at, s);
    end
    foreach (p.contents[i])
      $display("T|c[%0d] %s value=%0d", i, p.contents[i].name, p.contents[i].value);
    p.contents[0].name = "bad";
    $display("T|bad ok=%0d fail_at=%0d hits=%0d v0=%0d", it.walk(p), it.fail_at, it.hits, p.contents[0].value);
    sd = new("delta", 2, 200);
    p.contents[0] = sd;
    $display("T|virt ok=%0d hits=%0d sum=%0d", it.walk(p), it.hits, it.sum(p));
    $display("T|log=%s", it.log);
  end
endmodule
"#;

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_foreach_handle_member_{}_{}",
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
fn foreach_over_another_class_collection_compiles() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("T|")).collect();
        assert_eq!(
            lines,
            [
                "T|r=0 ok=1 hits=2 fail_at=-1 sum=116",
                "T|r=1 ok=1 hits=4 fail_at=-1 sum=116",
                "T|r=2 ok=1 hits=6 fail_at=-1 sum=116",
                "T|c[0] alpha value=15",
                "T|c[1] skip value=0",
                "T|c[2] gamma value=17",
                "T|c[3] stop value=15",
                "T|c[4] never value=0",
                "T|bad ok=0 fail_at=0 hits=6 v0=15",
                "T|virt ok=1 hits=6 sum=308",
                "T|log=2:gamma/16 2:gamma/16 2:gamma/16 0:delta/200 "
            ],
            "XEZIM_COMPILE_METHODS={compile}"
        );
    }
    let log = run("1");
    for m in ["item.walk", "item.sum"] {
        assert!(
            log.contains(&format!("runs-compiled scope={m}")),
            "{m} did not run compiled:\n{log}"
        );
    }
}
