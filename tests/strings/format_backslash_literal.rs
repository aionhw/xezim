//! IEEE 1800-2023 §5.9.1, §21.2.1.1, §21.2.1.2, §21.3.3: escape sequences
//! are decoded once by the lexer on string literals, so the runtime format
//! engine must treat `'\\'` as an ordinary literal character rather than
//! consuming `'\\'` + `'%'` or re-unescaping `\\n`/`\\t`/`\\"`/`\\\\`.

use xezim::simulate;

#[test]
fn format_backslash_is_literal_across_tasks_and_compiled_loop() {
    let dir = std::env::temp_dir().join(format!("xezim_fmt_backslash_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out_file = dir.join("fmt.out");
    let out_file_sv = out_file.to_string_lossy().replace('\\', "/");

    let src = format!(
        r#"class Formatter;
  function automatic string loop_fmt(int n);
    string r = "";
    for (int i = 0; i < n; i++) begin
      r = {{r, $sformatf("\\%03o", i)}};
    end
    return r;
  endfunction
  function string straight_fmt();
    return {{$sformatf("\\%03o", 0), $sformatf("\\%03o", 1), $sformatf("\\%03o", 2)}};
  endfunction
endclass

module top;
  Formatter f = new();
  string s, var_fmt, line, file_acc;
  int fd;
  byte ch = 8'd65;
  initial begin
    $display("O=[%s]", $sformatf("\\%03o", 8'd7));
    $display("X=[%s]", $sformatf("\\%0x", 8'hff));
    $display("D=[%s]", $sformatf("\\%0d", 8'd42));
    $display("S=[%s]", $sformatf("\\%s", "ab"));
    $display("C=[%s]", $sformatf("\\%c", ch));
    $display("B=[%s]", $sformatf("\\%0b", 4'b1010));
    $display("D2=[%s]", $sformatf("\\\\%03o", 8'd7));

    $display("BS_N=[a\\nb]");
    $display("BS_T=[a\\tb]");
    $display("BS_Q=[a\\\"b]");
    $display("ONCE=[%0d][%s]", "\101\x41\n\t\"".len(), "\101\x41");
    $display("PCT=[%s]", $sformatf("100\\%%"));
    $display("TRAIL=[%s] len=%0d", $sformatf("x\\"), $sformatf("x\\").len());

    $display("DISP=[\\%03o]", 8'd7);
    $write("WRIT=[\\%03o]\n", 8'd7);
    $sformat(s, "\\%03o", 8'd7);
    $display("SFMT=[%s]", s);
    $swrite(s, "\\%03o", 8'd7);
    $display("SWR=[%s]", s);
    s = $psprintf("\\%03o", 8'd7);
    $display("PSPR=[%s]", s);

    var_fmt = "\\%03o:%s";
    $display("VAR=[%s] flen=%0d", $sformatf(var_fmt, 8'd7, "ok"), var_fmt.len());

    fd = $fopen("{out_file_sv}", "w");
    $fdisplay(fd, "\\%03o", 8'd7);
    $fwrite(fd, "\\%0x\n", 8'hff);
    $fclose(fd);
    fd = $fopen("{out_file_sv}", "r");
    file_acc = "";
    while ($fgets(line, fd)) begin
      file_acc = {{file_acc, "|", line}};
    end
    $fclose(fd);
    $display("FILE=%s", file_acc);

    $display("CMP loop=[%s] straight=[%s] eq=%0d",
      f.loop_fmt(3), f.straight_fmt(), f.loop_fmt(3) == f.straight_fmt());
    $finish;
  end
endmodule
"#
    );

    let sim = simulate(&src, 1000).expect("simulate failed");
    let _ = std::fs::remove_dir_all(&dir);
    let lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .collect();

    let expected = [
        r"O=[\007]",
        r"X=[\ff]",
        r"D=[\42]",
        r"S=[\ab]",
        r"C=[\A]",
        r"B=[\1010]",
        r"D2=[\\007]",
        r"BS_N=[a\nb]",
        r"BS_T=[a\tb]",
        r#"BS_Q=[a\"b]"#,
        "ONCE=[5][AA]",
        r"PCT=[100\%]",
        r"TRAIL=[x\] len=2",
        r"DISP=[\007]",
        r"WRIT=[\007]",
        r"SFMT=[\007]",
        r"SWR=[\007]",
        r"PSPR=[\007]",
        r"VAR=[\007:ok] flen=8",
        "FILE=|\\007\n|\\ff",
        r"CMP loop=[\000\001\002] straight=[\000\001\002] eq=1",
    ];
    let joined = lines.join("\n");
    for exp in &expected {
        if exp.contains('\n') {
            assert!(
                joined.contains(exp),
                "missing multi-line output {exp:?}:\n{joined}"
            );
        } else {
            assert!(
                lines.iter().any(|l| l == exp),
                "missing line {exp:?}:\n{joined}"
            );
        }
    }
}
