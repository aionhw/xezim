//! §8.11 / §13.5: integral formals shadow same-named string properties.
//! Check numeric comparison with both a cold and a warm compiled-method cache.
use std::process::Command;

#[test]
fn cached_method_formals_keep_numeric_comparison() {
    let dir = std::env::temp_dir().join(format!(
        "xezim_method_shadow_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("design.sv");
    let cache = dir.join("methods");
    std::fs::write(
        &source,
        r#"
class C;
  string a = "left", b = "right";
  function bit less(input int a, input int b); return a < b; endfunction
endclass
module top;
  C c;
  initial begin
    c = new;
    $display("T|negative=%0d", c.less(-1, 0));
    $display("T|positive=%0d", c.less(1, 2));
    $finish;
  end
endmodule
"#,
    )
    .unwrap();
    for phase in ["cold", "warm"] {
        let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
            .args(["--sv2017", "--no-cache", "-s", "top"])
            .arg(&source)
            .env("XEZIM_COMPILE_METHODS", "1")
            .env("XEZIM_METHOD_TIER", "0")
            .env("XEZIM_METHOD_CACHE", &cache)
            .env("XEZIM_JIT", "0")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{phase}: {stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let rows: Vec<_> = stdout
            .lines()
            .filter(|line| line.starts_with("T|"))
            .collect();
        assert_eq!(rows, ["T|negative=1", "T|positive=1"], "{phase}");
        assert!(
            std::fs::read_dir(&cache).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|ext| ext == "bin")
            }),
            "{phase}: method cache was not populated"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
