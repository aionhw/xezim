//! §9.2.2.2.1 (#844): cold and prepared-cache runs keep the dependency on
//! another packed element. Older dependency tables must be rebuilt.

use std::process::Command;

#[test]
fn packed_element_dependency_survives_prepared_cache() {
    let dir = std::env::temp_dir().join(format!("xezim_packed_cache_844_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("top.sv");
    let cache = dir.join("cache");
    std::fs::write(
        &source,
        r#"
module top;
  logic [7:0] a = 8'h01;
  logic [1:0][7:0] ds;
  assign ds[0] = a;
  always_comb ds[1] = ds[0] ^ 8'h01;
  initial begin
    #1 $display("V %h", ds[1]);
    a = 8'h22;
    #1 $display("V %h", ds[1]);
    $finish;
  end
endmodule
"#,
    )
    .unwrap();
    let run = || {
        let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
            .args([
                "--simulate",
                "--cache",
                "--verbose",
                "-s",
                "top",
                "--cache-dir",
            ])
            .arg(&cache)
            .arg(&source)
            .output()
            .expect("run simulator");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(out.status.success(), "{stdout}\n{stderr}");
        let values: Vec<_> = stdout
            .lines()
            .filter(|line| line.starts_with("V "))
            .collect();
        assert_eq!(values, ["V 00", "V 23"], "{stdout}\n{stderr}");
        stderr
    };
    run();
    assert!(run().contains("prepared-comb hit"));
    let prepared = std::fs::read_dir(&cache)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|ext| ext == "xezcomb"))
        .expect("prepared combinational cache");
    let mut bytes = std::fs::read(&prepared).unwrap();
    bytes[..8].copy_from_slice(b"XZCMB028");
    std::fs::write(&prepared, bytes).unwrap();
    assert!(run().contains("bad format marker"));
    assert!(run().contains("prepared-comb hit"));
    std::fs::remove_dir_all(dir).unwrap();
}
