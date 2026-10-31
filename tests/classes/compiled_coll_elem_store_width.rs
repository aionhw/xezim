//! §11.6.1: an element of a member collection is the context of the value
//! stored into it, so `d[i] = d[i] & ((1 << w) - 1)` on `bit [1023:0] d[]`
//! masks at 1024 bits. A compiled method evaluated the stored value
//! self-determined, so the mask was built at 32 bits and the element lost
//! everything above bit 31. Expected T| lines are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class cfg_c; int data_width = 64; endclass
class tr;
  cfg_c port_cfg;
  bit [1023:0] data[];
  bit [127:0] wstrb[$];
  int unsigned burst_size = 4;
  bit [99:0] aa[string];
  function void post();
    foreach (data[index])
      data[index] = data[index] & ((1 << (port_cfg.data_width)) - 1);
    foreach (wstrb[index])
      wstrb[index] = (1 << (1 << burst_size)) - 1;
    aa["k"] = (1 << 70) - 1;
  endfunction
endclass
module top;
  initial begin
    tr t; t = new; t.port_cfg = new;
    t.data = new[2]; t.data[0] = {16{64'hfedc_ba98_7654_3210}}; t.data[1] = '1;
    t.wstrb.push_back(0);
    t.post();
    $display("T|%h %h", t.data[0][127:0], t.data[1][127:0]);
    $display("T|%h %h", t.wstrb[0], t.aa["k"]);
  end
endmodule
"#;

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_coll_elem_store_width_{}_{}",
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
fn member_collection_element_store_takes_the_element_width() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("T|")).collect();
        assert_eq!(
            lines,
            [
                "T|0000000000000000fedcba9876543210 0000000000000000ffffffffffffffff",
                "T|0000000000000000000000000000ffff 00000003fffffffffffffffff"
            ],
            "XEZIM_COMPILE_METHODS={compile}"
        );
    }
}
