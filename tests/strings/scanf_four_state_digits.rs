//! `$fscanf`/`$sscanf` integer conversions read 4-state digits
//! (IEEE 1800-2023 §21.3.4.3): the %b digit set is 0, 1, x, X, z, Z, ? and
//! _, and %o and %h add x, X, z, Z and ? to their digits, each one setting
//! its 1, 3 or 4 bits to x or z (`?` is z, as in a literal, §5.7.1). %d
//! takes a single x, X, z, Z or ? in place of its digits, setting every
//! bit.
//!
//! xezim stopped at the first x or z, so `$fscanf(fid, "%b\n", v)` on
//! "xx01" returned 0 and left `v` unchanged. OpenSPARC T1's playback driver
//! reads stimulus vectors that are mostly x this way, and its bench printed
//! a false "Playback PASSED!".

use xezim::simulate;

fn outs(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .collect()
}

fn assert_all_pass(o: &[String], checks: usize) {
    let fails: Vec<&String> = o.iter().filter(|l| l.starts_with("FAIL")).collect();
    assert!(fails.is_empty(), "{fails:#?}\n{o:#?}");
    assert_eq!(
        o.iter().filter(|l| l.starts_with("PASS")).count(),
        checks,
        "{o:#?}"
    );
}

/// A scratch directory this test creates itself, readable by its owner only.
/// `create` (not `create_dir_all`) fails on a path that already exists, so a
/// directory or link someone else made in the shared temporary directory is
/// never used.
fn private_scratch_dir() -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    loop {
        let dir = std::env::temp_dir().join(format!(
            "xezim_scanf_xz_{}_{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        match builder.create(&dir) {
            Ok(()) => return dir,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => panic!("create {}: {e}", dir.display()),
        }
    }
}

#[test]
fn scanf_integer_conversions_read_x_and_z_digits() {
    let dir = private_scratch_dir();
    let path = dir.join("vec.txt").to_string_lossy().replace('\\', "/");
    let src = format!(
        r#"
module tb;
  integer fid, code, d, e;
  reg [3:0] v;
  reg [7:0] h;

  task automatic check(bit ok, string what);
    if (ok) $display("PASS %s", what);
    else $display("FAIL %s", what);
  endtask

  initial begin
    fid = $fopen("{path}", "w");
    $fwrite(fid, "xx01\nzZ10\nXx1?\n");
    $fclose(fid);
    fid = $fopen("{path}", "r");
    code = $fscanf(fid, "%b\n", v);
    check(code == 1 && v === 4'bxx01, "%b reads x digits");
    code = $fscanf(fid, "%b\n", v);
    check(code == 1 && v === 4'bzz10, "%b reads z and Z digits");
    code = $fscanf(fid, "%b\n", v);
    check(code == 1 && v === 4'bxx1z, "%b reads X, and ? as z");
    $fclose(fid);

    code = $sscanf("1_?", "%b", v);
    check(code == 1 && v === 4'b001z, "%b with _ and ?");
    code = $sscanf("x1", "%h", h);
    check(code == 1 && h === 8'bxxxx0001, "%h x digit sets 4 bits");
    code = $sscanf("Zf", "%x", h);
    check(code == 1 && h === 8'bzzzz1111, "%x Z digit sets 4 bits");
    code = $sscanf("1z", "%o", h);
    check(code == 1 && h === 8'b00001zzz, "%o z digit sets 3 bits");

    code = $sscanf("x", "%d", d);
    check(code == 1 && d === 32'bx, "%d lone x sets every bit");
    code = $sscanf("Z", "%d", d);
    check(code == 1 && d === 32'bz, "%d lone Z sets every bit");
    code = $sscanf("? 7", "%d %d", d, e);
    check(code == 2 && d === 32'bz && e == 7, "%d lone ? is a whole value");
    code = $sscanf("12", "%d", d);
    check(code == 1 && d == 12, "%d digits unchanged");
    $finish;
  end
endmodule
"#
    );
    let sim = simulate(&src, 1000).expect("simulate failed");
    let _ = std::fs::remove_dir_all(&dir);
    assert_all_pass(&outs(&sim), 11);
}

/// `$value$plusargs` reads %b, %o and %h values with the same 4-state digits.
#[test]
fn value_plusargs_read_x_and_z_digits() {
    let src = r#"
module tb;
  reg [3:0] b;
  reg [7:0] h, o;
  initial begin
    if ($value$plusargs("B=%b", b) == 1 && b === 4'bxx1z) $display("PASS %%b");
    else $display("FAIL %%b %b", b);
    if ($value$plusargs("H=%h", h) == 1 && h === 8'bxxxx0001) $display("PASS %%h");
    else $display("FAIL %%h %b", h);
    if ($value$plusargs("O=%o", o) == 1 && o === 8'b00001zzz) $display("PASS %%o");
    else $display("FAIL %%o %b", o);
  end
endmodule
"#;
    let plusargs: Vec<String> = ["+B=Xx1?", "+H=x1", "+O=1z"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let sim = xezim::simulate_multi(
        &[src.to_string()],
        100_000,
        None,
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &plusargs,
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
    )
    .expect("simulate failed");
    assert_all_pass(&outs(&sim), 3);
}
