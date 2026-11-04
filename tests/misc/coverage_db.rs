//! The results file and exit status, end to end: assertion entries carry
//! their file and line (they were keyed by byte offset alone, so two files'
//! statements at the same offset merged into one entry), and an
//! `illegal_bins` hit fails a `--error-exit` run, and a labelled assertion
//! or cover site carries its label.
use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_cov_db_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Two files whose `assert` keywords sit at the same byte offset.
#[test]
fn assertion_entries_carry_file_and_line() {
    let d = scratch("assert");
    let a = d.join("m1.sv");
    let b = d.join("m2.sv");
    std::fs::write(&a, "module m1;\n  initial assert (1);\nendmodule\n").unwrap();
    std::fs::write(
        &b,
        "module m2;\n  initial assert (0);\nendmodule\nmodule top; m1 u1 (); m2 u2 (); endmodule\n",
    )
    .unwrap();
    let db = d.join("cov.json");
    let out = Command::new(xezim())
        .env("XEZIM_COV_DB", &db)
        .arg(&a)
        .arg(&b)
        .output()
        .expect("run xezim");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json = std::fs::read_to_string(&db).expect("coverage file");
    let entry = |f: &std::path::Path, pass: u32, fail: u32| {
        format!(
            "{{\"file\": \"{}\", \"line\": 2, \"span_start\": 21, \"kind\": \"assert\", \"pass\": {}, \"fail\": {}}}",
            f.display(),
            pass,
            fail
        )
    };
    assert!(json.contains(&entry(&a, 1, 0)), "{json}");
    assert!(json.contains(&entry(&b, 0, 1)), "{json}");
    assert!(json.contains("\"assertion_sites\": 2"), "{json}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn illegal_bin_hit_fails_error_exit() {
    let d = scratch("illegal");
    let f = d.join("t.sv");
    std::fs::write(
        &f,
        "module t;\n  bit [1:0] s;\n  covergroup cg; cp : coverpoint s { bins ok = {[0:2]}; \
         illegal_bins bad = {3}; } endgroup\n  cg c = new();\n  initial begin s = 3; c.sample(); end\n\
         endmodule\n",
    )
    .unwrap();
    let run = |extra: &[&str]| {
        Command::new(xezim())
            .env("XEZIM_COV_DB", "/dev/null")
            .args(extra)
            .arg(&f)
            .output()
            .expect("run xezim")
    };
    let plain = run(&[]);
    assert_eq!(plain.status.code(), Some(0));
    let strict = run(&["--error-exit"]);
    assert_eq!(strict.status.code(), Some(1));
    let text = String::from_utf8_lossy(&strict.stdout);
    assert!(
        text.contains("Illegal bin hit at value 3: cg.cp.bad"),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// §16.5 / §16.14: `c_hit: cover property (...)` names the site. The
/// results file gives a labelled site's label as `name`, so a tool can match
/// cover points to a verification plan; an unlabelled site has no `name`.
#[test]
fn labelled_assertion_entries_carry_name() {
    let d = scratch("label");
    let f = d.join("tb_cov.sv");
    std::fs::write(
        &f,
        "module tb_cov;\n\
         \x20 logic clk=0; always #5 clk=~clk;\n\
         \x20 logic [1:0] s=0;\n\
         \x20 always @(posedge clk) s <= s+1;\n\
         \x20 c_hit: cover property (@(posedge clk) s==2);\n\
         \x20 c_miss: cover property (@(posedge clk) s==7);\n\
         \x20 a_lbl: assert property (@(posedge clk) s < 4);\n\
         \x20 cover property (@(posedge clk) s==1);\n\
         \x20 initial begin #60 $finish; end\n\
         endmodule\n",
    )
    .unwrap();
    let db = d.join("cov.json");
    let out = Command::new(xezim())
        .env("XEZIM_COV_DB", &db)
        .arg(&f)
        .output()
        .expect("run xezim");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json = std::fs::read_to_string(&db).expect("coverage file");
    // Clock edges at 5..55 sample s = 0, 1, 2, 3, 0, 1. The span offset is
    // not pinned: an entry is matched by what precedes and follows it.
    let has = |name: &str, line: u32, kind: &str, pass: u32| {
        let head = format!(
            "{{{}\"file\": \"{}\", \"line\": {}, \"span_start\": ",
            name,
            f.display(),
            line
        );
        let tail = format!("\"kind\": \"{}\", \"pass\": {}, \"fail\": 0}}", kind, pass);
        json.lines()
            .map(|l| l.trim().trim_end_matches(','))
            .any(|l| l.starts_with(&head) && l.ends_with(&tail))
    };
    assert!(has("\"name\": \"c_hit\", ", 5, "cover", 1), "{json}");
    assert!(has("\"name\": \"c_miss\", ", 6, "cover", 0), "{json}");
    assert!(has("\"name\": \"a_lbl\", ", 7, "assert", 6), "{json}");
    assert!(has("", 8, "cover", 2), "{json}");
    assert_eq!(json.matches("\"name\": ").count(), 3, "{json}");
    assert!(json.contains("\"assertion_sites\": 4"), "{json}");
    let _ = std::fs::remove_dir_all(&d);
}

/// Every labelled form names its site, not only a clocked concurrent item:
/// a module-level deferred assertion (`assert #0`, `assert final`, §16.4),
/// an unclocked `assert property`, and a labelled procedural statement
/// (§9.3.1), immediate or concurrent, which the parser hosts in a named
/// block. The unclocked property's entry also carries its file, line and
/// kind: it was keyed by a hash of a synthetic name and tagged `cover`.
#[test]
fn every_labelled_assertion_form_carries_name() {
    let d = scratch("forms");
    let f = d.join("tb_cov.sv");
    std::fs::write(
        &f,
        "module tb_cov;\n\
         \x20 logic clk=0; always #5 clk=~clk;\n\
         \x20 logic [1:0] s=0;\n\
         \x20 always @(posedge clk) s <= s+1;\n\
         \x20 c_hit:  cover  property (@(posedge clk) s==2);\n\
         \x20 a_def:  assert #0 (s < 4);\n\
         \x20 a_fin:  assert final (s < 4);\n\
         \x20 a_nck:  assert property (s < 4);\n\
         \x20 always @(posedge clk) begin\n\
         \x20   a_imm: assert (s < 4);\n\
         \x20   c_imm: cover (s == 1);\n\
         \x20   p_lbl: assert property (@(posedge clk) s < 4);\n\
         \x20 end\n\
         \x20 initial begin #60 $finish; end\n\
         endmodule\n",
    )
    .unwrap();
    let db = d.join("cov.json");
    let out = Command::new(xezim())
        .env("XEZIM_COV_DB", &db)
        .arg(&f)
        .output()
        .expect("run xezim");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json = std::fs::read_to_string(&db).expect("coverage file");
    // Clock edges at 5..55 sample s = 0, 1, 2, 3, 0, 1.
    let has = |name: &str, line: u32, kind: &str, pass: u32, fail: u32| {
        let head = format!(
            "{{\"name\": \"{}\", \"file\": \"{}\", \"line\": {}, \"span_start\": ",
            name,
            f.display(),
            line
        );
        let tail = format!(
            "\"kind\": \"{}\", \"pass\": {}, \"fail\": {}}}",
            kind, pass, fail
        );
        json.lines()
            .map(|l| l.trim().trim_end_matches(','))
            .any(|l| l.starts_with(&head) && l.ends_with(&tail))
    };
    assert!(has("c_hit", 5, "cover", 1, 0), "{json}");
    assert!(has("a_def", 6, "assert", 1, 0), "{json}");
    assert!(has("a_fin", 7, "assert", 1, 0), "{json}");
    assert!(has("a_nck", 8, "assert", 1, 0), "{json}");
    assert!(has("a_imm", 10, "assert", 6, 0), "{json}");
    assert!(has("c_imm", 11, "cover", 2, 4), "{json}");
    assert!(has("p_lbl", 12, "assert", 6, 0), "{json}");
    assert_eq!(json.matches("\"name\": ").count(), 7, "{json}");
    assert!(json.contains("\"assertion_sites\": 7"), "{json}");
    let _ = std::fs::remove_dir_all(&d);
}

/// A labelled immediate assertion in an `initial` block keeps its label
/// under `--code-coverage` (§9.3.1, §16.3): the statement marker the
/// instrumentation adds beside it made the label's block look like two
/// statements, so the label was dropped (#586, reported by hernloh).
#[test]
fn labelled_assertions_keep_their_label_under_code_coverage() {
    let d = scratch("initial_lbl");
    let f = d.join("tb3.sv");
    std::fs::write(
        &f,
        "module tb3;\n\
         \x20 logic clk=0; always #5 clk=~clk;\n\
         \x20 logic [1:0] s=0, t=0;\n\
         \x20 always @(posedge clk) s <= s+1;\n\
         \x20 always @(posedge clk) begin m_lbl: assert (s<4); t <= s; end\n\
         \x20 initial begin one_lbl: assert (1); end\n\
         \x20 initial begin #1 after_lbl: assert (1); end\n\
         \x20 initial begin two_lbl: assert (1); #60 $finish; end\n\
         endmodule\n",
    )
    .unwrap();
    let db = d.join("cov.json");
    let out = Command::new(xezim())
        .env("XEZIM_COV_DB", &db)
        .args(["--code-coverage", "-s", "tb3"])
        .arg(&f)
        .output()
        .expect("run xezim");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json = std::fs::read_to_string(&db).expect("coverage file");
    for (name, line) in [
        ("m_lbl", 5),
        ("one_lbl", 6),
        ("after_lbl", 7),
        ("two_lbl", 8),
    ] {
        let head = format!(
            "{{\"name\": \"{}\", \"file\": \"{}\", \"line\": {}, ",
            name,
            f.display(),
            line
        );
        assert!(
            json.lines().any(|l| l.trim().starts_with(&head)),
            "{name}: {json}"
        );
    }
    let _ = std::fs::remove_dir_all(&d);
}
