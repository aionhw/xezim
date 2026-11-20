//! Audit round-45 differential finds — all reference-validated.
//!
//! 1. §12.7.3 foreach over a MULTI-packed-dim vector iterates the outermost
//!    packed dimension (4), not the flat bit width (32).
//! 2. §21.2.1.3 `%s` of a packed operand renders ceil(width/8) chars — NUL
//!    bytes as spaces, no truncation of leading NULs; `%0s` is minimal.
//! 3. §13.5.3 blank positional args (`t(10, , o, r)`) take the formal's
//!    default; named args bind ref formals by NAME (both were rejected).
//! 4. §19.5.2 `bins b[] = {[lo:hi]}` creates one sub-bin per value; coverage
//!    is hit/total (2 of 4 = 50%), not 100% on first hit.
//! 5. §21.3.5 `$feof` is a sticky flag set when a read HITS end-of-file —
//!    a read consuming exactly up to the final newline leaves it clear.

use xezim::simulate;

fn outs(sim: &xezim::compiler::Simulator) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Reference: fe=4 (outermost dim), order 3,2,1,0 for a [3:0] dim.
#[test]
fn foreach_multidim_packed_iterates_outer_dim() {
    let src = r#"
module top;
  logic [3:0][7:0] w;
  int n = 0;
  int first = -1;
  initial begin
    w = 32'hDEADBEEF;
    foreach (w[i]) begin
      if (first == -1) first = i;
      n++;
    end
    $display("T|fe=%0d first=%0d", n, first);
  end
endmodule
"#;
    let out = outs(&simulate(src, 10).expect("sim"));
    assert!(
        out.contains("T|fe=4 first=3"),
        "outermost packed dim, left-to-right:\n{out}"
    );
}

/// Reference: `[  Hi]` (leading NULs as spaces), `[Hi]` for %0s, `[A B]`
/// (mid NUL as space), `[ ]` for 8'h00; string vars unchanged.
#[test]
fn percent_s_packed_width_and_nul_spaces() {
    let src = r#"
module top;
  logic [31:0] w = 32'h00004869;
  string s = "Hi";
  initial begin
    $display("T|[%s][%0s][%s][%s]", w, w, 24'h410042, 8'h00);
    $display("T|[%s]", s);
  end
endmodule
"#;
    let out = outs(&simulate(src, 10).expect("sim"));
    assert!(
        out.contains("T|[  Hi][Hi][A B][ ]"),
        "packed %s widths:\n{out}"
    );
    assert!(out.contains("T|[Hi]"), "string var stays minimal:\n{out}");
}

/// Reference: o=15 r=101 (blank arg takes default b=5), o=27 r=201 (named
/// binding incl. the ref formal), f defaults 23/93/28.
#[test]
fn blank_and_named_args_bind_defaults_and_ref() {
    let src = r#"
module top;
  int o1 = 0, r1 = 0, o2 = 0, fsum = 0;
  task automatic t1(input int a, input int b = 5, output int o, ref int r);
    o = a + b;
    r = r + 100;
  endtask
  function automatic int f1(int x = 2, int y = 3);
    return x * 10 + y;
  endfunction
  initial begin
    int o, r;
    r = 1;
    t1(10, , o, r);
    o1 = o; r1 = r;
    t1(.r(r), .o(o), .a(20), .b(7));
    o2 = o;
    fsum = f1() * 1 + f1(9) * 0 + f1(.y(8)) * 0;
    $display("T|%0d %0d %0d %0d %0d %0d", o1, r1, o2, r, f1(9), f1(.y(8)));
  end
endmodule
"#;
    let out = outs(&simulate(src, 10).expect("sim"));
    assert!(out.contains("T|15 101 27 201 93 28"), "arg binding:\n{out}");
}

/// Reference: cov=50.00 after hitting 2 of 4 array sub-bins, 100.00 after 4.
#[test]
fn covergroup_array_bins_count_per_value() {
    let src = r#"
module top;
  bit [1:0] s;
  covergroup cg;
    cp: coverpoint s { bins b[] = {[0:3]}; }
  endgroup
  cg c1;
  initial begin
    c1 = new;
    s = 0; c1.sample();
    s = 1; c1.sample();
    $display("T|cov=%0.2f", c1.get_inst_coverage());
    s = 2; c1.sample();
    s = 3; c1.sample();
    $display("T|cov2=%0.2f", c1.get_inst_coverage());
  end
endmodule
"#;
    let out = outs(&simulate(src, 10).expect("sim"));
    assert!(out.contains("T|cov=50.00"), "2/4 sub-bins = 50%:\n{out}");
    assert!(out.contains("T|cov2=100.00"), "4/4 sub-bins = 100%:\n{out}");
}

/// Reference: eof=0 after reading the final newline-terminated line;
/// eof2=1 only after the next read attempt comes up empty.
#[test]
fn feof_is_sticky_not_positional() {
    // The fixture must NOT land in the repo working tree — a CWD-relative
    // $fopen recreated `audit45_feof_tmp.txt` at the repo root on every test
    // run, and blanket `git add -A` commits kept re-tracking it.
    let tmp = std::env::temp_dir().join(format!("xezim_audit45_feof_{}.txt", std::process::id()));
    let tmp_path = tmp.to_string_lossy().replace('\\', "/");
    let src = format!(
        r#"
module top;
  int fd, n, a, b;
  string line;
  initial begin
    fd = $fopen("{tmp_path}", "w");
    $fwrite(fd, "12 34\n");
    $fwrite(fd, "line-two 99\n");
    $fclose(fd);
    fd = $fopen("{tmp_path}", "r");
    n = $fscanf(fd, "%d %d", a, b);
    void'($fgets(line, fd));
    void'($fgets(line, fd));
    $display("T|n=%0d eof=%0d", n, $feof(fd) != 0);
    void'($fgets(line, fd));
    $display("T|eof2=%0d", $feof(fd) != 0);
    $fclose(fd);
  end
endmodule
"#
    );
    let out = outs(&simulate(&src, 10).expect("sim"));
    let _ = std::fs::remove_file(&tmp);
    assert!(
        out.contains("T|n=2 eof=0"),
        "no flag before a failed read:\n{out}"
    );
    assert!(
        out.contains("T|eof2=1"),
        "flag after the failed read:\n{out}"
    );
}

/// §21.3.4.3, §21.3.8: file input spans lines. EOF records a read attempt,
/// including a trailing whitespace directive looking for the next token.
#[test]
fn fscanf_directives_cross_lines_and_set_eof_on_attempted_read() {
    let path = std::env::temp_dir().join(format!("xezim_fscanf_1376_{}.txt", std::process::id()));
    let path_sv = path.to_string_lossy().replace('\\', "/");
    let src = format!(
        r#"
module top;
  int fd, n, a, b, loops;
  byte c;
  string word;
  initial begin
    fd = $fopen("{path_sv}", "w");
    $fwrite(fd, "hello.hex\n");
    $fclose(fd);
    fd = $fopen("{path_sv}", "r");
    loops = 0;
    while (!$feof(fd) && loops < 3) begin
      n = $fscanf(fd, "%s\n", word);
      loops++;
    end
    $display("T|words=%0d n=%0d word=%s eof=%0d", loops, n, word, $feof(fd));
    $fclose(fd);
    fd = $fopen("{path_sv}", "w");
    $fwrite(fd, "12\n34\n;!");
    $fclose(fd);
    fd = $fopen("{path_sv}", "r");
    n = $fscanf(fd, "%d %d\n;", a, b);
    $display("T|nums=%0d,%0d n=%0d eof=%0d", a, b, n, $feof(fd));
    n = $fscanf(fd, "%c", c);
    $display("T|last=%0d n=%0d eof=%0d", c, n, $feof(fd));
    n = $fscanf(fd, "%c", c);
    $display("T|past=%0d n=%0d eof=%0d", c, n, $feof(fd));
    $fclose(fd);
  end
endmodule"#
    );
    let result = simulate(&src, 10);
    let _ = std::fs::remove_file(path);
    let out = outs(&result.expect("sim"));
    for expected in [
        "T|words=1 n=1 word=hello.hex eof=1",
        "T|nums=12,34 n=2 eof=0",
        "T|last=33 n=1 eof=0",
        "T|past=33 n=-1 eof=1",
    ] {
        assert!(out.lines().any(|line| line == expected), "{out}");
    }
}
