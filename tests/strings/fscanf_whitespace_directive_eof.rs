//! `$fscanf` reads its file as a stream, the way C `fscanf` does
//! (IEEE 1800-2023 §21.3.4.3): a white-space directive in the format, and
//! the white space a conversion skips, read on past line ends up to the
//! next non-white-space character. When that read runs into the end of the
//! file, the end-of-file indicator `$feof` reports is set (§21.3.8).
//!
//! xezim scanned one line at a time, so a trailing `"\n"` directive stopped
//! at the line end: `while (!$feof(f)) $fscanf(f, "%s\n", s);` over a
//! one-line list ran twice, the second pass returning -1 and leaving `s`
//! unchanged. The SyntaCore SCR1 testbench loops over its test list this way
//! and ran the last test twice. Expected values match Verilator and Icarus.

use xezim::simulate;

fn outs(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .collect()
}

#[test]
fn fscanf_whitespace_directive_reaching_eof_sets_feof() {
    let dir = std::env::temp_dir().join(format!("xezim_fscanf_ws_eof_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("list.txt").to_string_lossy().replace('\\', "/");
    let src = format!(
        r#"
module tb;
  integer f, n, code, a, b, c;
  string s, names;

  task automatic put(string text);
    f = $fopen("{path}", "w");
    $fwrite(f, "%s", text);
    $fclose(f);
    f = $fopen("{path}", "r");
  endtask

  task automatic check(bit ok, string what);
    if (ok) $display("PASS %s", what);
    else $display("FAIL %s", what);
  endtask

  initial begin
    // The SCR1 test-list loop: one entry, read once.
    put("hello.hex\n");
    n = 0;
    while (!$feof(f)) begin
      code = $fscanf(f, "%s\n", s);
      n++;
      if (n == 1) check(code == 1 && s == "hello.hex" && $feof(f) != 0,
                        "trailing \\n directive reaches EOF");
      if (n > 3) break;
    end
    check(n == 1, "test-list loop runs once per entry");
    check($fscanf(f, "%s\n", s) == -1 && s == "hello.hex", "next $fscanf returns EOF");
    check($fgets(s, f) == 0 && $fgetc(f) == -1, "$fgets and $fgetc see EOF too");
    $fclose(f);

    // Blank lines after the last entry are white space as well.
    put("a.hex\nb.hex\n\n\n");
    n = 0;
    names = "";
    while (!$feof(f) && n < 4) begin
      code = $fscanf(f, "%s\n", s);
      if (code == 1) names = {{names, s, ";"}};
      n++;
    end
    check(n == 2 && names == "a.hex;b.hex;", "loop skips trailing blank lines");
    $fclose(f);

    // The directive stops at the next word without consuming it.
    put("x\n\ny z\n");
    code = $fscanf(f, "%s\n", s);
    check(code == 1 && s == "x" && $feof(f) == 0, "directive stops before data");
    check($fgets(s, f) == 4 && s == "y z\n", "$fgets reads the rest of the line");
    $fclose(f);

    // White space between conversions spans lines, and so does a
    // conversion's own leading white space.
    put("12\n34\n\n5\n");
    a = 0; b = 0; c = 0;
    code = $fscanf(f, "%d %d", a, b);
    check(code == 2 && a == 12 && b == 34 && $feof(f) == 0, "\"%d %d\" spans a line end");
    code = $fscanf(f, "%d", c);
    check(code == 1 && c == 5 && $feof(f) == 0, "%d skips blank lines, stops at \\n");
    $fclose(f);

    // A literal after a line-spanning directive matches on the next line.
    put("5\n;\n");
    code = $fscanf(f, "%d\n;", a);
    check(code == 1 && a == 5 && $fgetc(f) == 10, "literal after a line end matches");
    $fclose(f);

    // EOF comes from what the format reads, not from a read-ahead: %c of
    // the final character (no newline) does not reach the end; %d must
    // look one past its digits and does.
    put("a");
    code = $fscanf(f, "%c", c);
    check(code == 1 && c == "a" && $feof(f) == 0, "%c of the last char leaves EOF clear");
    check($fscanf(f, "%c", c) == -1 && $feof(f) != 0, "the next %c hits EOF");
    $fclose(f);
    put("12");
    code = $fscanf(f, "%d", a);
    check(code == 1 && a == 12 && $feof(f) != 0, "%d at the end of the file reaches EOF");
    $fclose(f);

    // $sscanf has no EOF flag; its count is unchanged.
    check($sscanf("hello.hex\n", "%s\n", s) == 1 && s == "hello.hex", "$sscanf count");
    $finish;
  end
endmodule
"#
    );
    let sim = simulate(&src, 1000).expect("simulate failed");
    let _ = std::fs::remove_dir_all(&dir);
    let o = outs(&sim);
    let fails: Vec<&String> = o.iter().filter(|l| l.starts_with("FAIL")).collect();
    assert!(fails.is_empty(), "{fails:#?}\n{o:#?}");
    assert_eq!(
        o.iter().filter(|l| l.starts_with("PASS")).count(),
        14,
        "{o:#?}"
    );
}
