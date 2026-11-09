//! IEEE 1800-2023 §21.7.1.6: acknowledged waveform flushes.

/// §21.7.1.6: a startup flush must make the header and preceding records
/// readable once time advances, including with the default threaded writer.
/// §21.3.4.3 reads tokens from the free-format VCD (§21.7).
/// Startup checkpoint ordering is xezim compatibility behavior.
#[test]
fn startup_flush_makes_preceding_records_visible() {
    let dir = std::env::temp_dir().join(format!("xezim_startup_flush_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = r#"
`timescale 1ns/1ns
module top;
  logic [3:0] a=0;
  integer fd, n;
  string token;
  bit header=0, checkpoint=0, in_checkpoint=0;
  initial begin
    $dumpfile("w.vcd"); $dumpvars(0, top.a);
    a=5; $dumpall; $dumpflush;
    #1;
    fd=$fopen("w.vcd", "r");
    if (!fd) $fatal(1, "missing dump");
    while (!$feof(fd)) begin
      n=$fscanf(fd,"%s",token);
      if (n == 1 && token == "$enddefinitions") begin
        n=$fscanf(fd,"%s",token);
        if (n == 1 && token == "$end") header=1;
      end
      if (n == 1 && token == "$dumpall") in_checkpoint=1;
      if (n == 1 && token == "$end" && in_checkpoint) begin
        checkpoint=1; in_checkpoint=0;
      end
    end
    $fclose(fd);
    if (!header || !checkpoint) $fatal(1, "flush lost preceding output");
    $display("flush-visible"); $finish;
  end
endmodule
"#;
    std::fs::write(dir.join("test.sv"), source).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&dir)
        .env_remove("XEZIM_DUMP_INLINE")
        .args(["--no-cache", "--wave", "-s", "top", "test.sv"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("flush-visible"));
    let text = std::fs::read_to_string(dir.join("w.vcd")).unwrap();
    // xezim exporter compatibility: this startup checkpoint retains a=5.
    let checkpoint = text
        .split_once("$dumpall")
        .expect("checkpoint")
        .1
        .split_once("$end")
        .expect("complete checkpoint")
        .0;
    assert!(checkpoint.split_whitespace().any(|t| {
        t.strip_prefix('b')
            .is_some_and(|v| v.trim_start_matches('0') == "101")
    }));
    std::fs::remove_dir_all(dir).unwrap();
}

/// §21.7.1.6: a later flush waits for a large queued batch before the reader.
/// §7.4.1 / §21.7.5: a packed array is dumped as one vector.
/// §21.3.4.3 reads VCD tokens.
#[test]
fn flush_waits_for_large_batches() {
    let dir = std::env::temp_dir().join(format!("xezim_batch_flush_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = r#"
`timescale 1ns/1ns
module top;
  logic [3:0][16383:0] w0, w1, w2, w3;
  initial begin : reader
    integer fd, n;
    string token;
    w0 = '0; w1 = '0; w2 = '0; w3 = '0;
    $dumpfile("w.vcd"); $dumpvars(0, top);
    #1 begin w0 = '1; w1 = '1; w2 = '1; w3 = '1; end
    #1 $dumpflush;
    fd = $fopen("w.vcd", "r");
    if (!fd) $fatal(1, "missing dump");
    n = 0;
    while (!$feof(fd)) begin
      if ($fscanf(fd, "%s", token) == 1) n++;
    end
    $fclose(fd);
    $display("visible %0d", n);
    $finish;
  end
endmodule
"#;
    std::fs::write(dir.join("test.sv"), source).unwrap();
    // Wide vectors keep the worker busy with very few interpreter operations.
    // Repetition exercises scheduling; each acknowledged run must be complete.
    for _ in 0..5 {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
            .current_dir(&dir)
            .env_remove("XEZIM_DUMP_INLINE")
            .args(["--no-cache", "--wave", "-s", "top", "test.sv"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let visible: usize = stdout
            .lines()
            .find_map(|l| l.strip_prefix("visible "))
            .expect("reader token count")
            .parse()
            .unwrap();
        let text = std::fs::read_to_string(dir.join("w.vcd")).unwrap();
        // §21.7.1.6 requires the records preceding the call to be visible.
        // Compare only before #2: future block-local tracing may append reader
        // variable changes after it, so it need not be a bare final timestamp.
        let before_finish = text
            .split_once("\n#2\n")
            .map_or(text.as_str(), |(before, _)| before);
        assert_eq!(
            visible,
            before_finish.split_whitespace().count(),
            "{stdout}"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
