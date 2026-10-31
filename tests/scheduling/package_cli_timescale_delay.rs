//! IEEE 1800-2017 §3.14.2.3: a package with no `timescale directive takes the
//! default time unit, as a module does; xezim's is the CLI `-timescale`.
//! Counted in global ticks, `#limit` was picoseconds under `-timescale 1ns/1ps`, so a
//! 200 us timeout expired at 200 ns. The module-level delay of the same
//! value stayed 200 us.

use std::process::Command;

#[test]
fn package_delay_uses_cli_timescale_not_the_tick() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "pkg_cli_ts_{}_{}.sv",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(
        &path,
        r#"
package p;
  time limit;
  task wait_limit();
    #limit;
  endtask
endpackage
module t;
  import p::*;
  time local_limit;
  initial begin
    limit = 64'd200_000 * 1ns;
    local_limit = limit;
    wait_limit();
    $display("PKG raw=%0d", $time);
    #local_limit;
    $display("MOD raw=%0d", $time);
    $finish;
  end
endmodule
"#,
    )
    .unwrap();
    let bin = env!("CARGO_BIN_EXE_xezim");
    let out = Command::new(bin)
        .args(["--max-time", "1ms", "-timescale", "1ns/1ps", "-s", "t"])
        .arg(&path)
        .output()
        .expect("run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_file(&path);
    assert!(
        text.contains("PKG raw=200000"),
        "package delay must be 200 us, not 200 ns:\n{text}"
    );
    assert!(
        text.contains("MOD raw=400000"),
        "module delay of the same value must also be 200 us:\n{text}"
    );
}
