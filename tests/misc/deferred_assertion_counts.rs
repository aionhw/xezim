//! IEEE 1800-2023 §16.4.2: a deferred assertion's report matures in the
//! Observed (`#0`) or Postponed (`final`) region, unless its process resumes
//! in the same time slot first. A resumption is a flush point: the report is
//! discarded and its action does not run. An `always_comb` that runs again in
//! the same slot supersedes its earlier report. Only a report that matures is
//! an attempt, so only it may be counted.
//!
//! xezim ran no action for a flushed report, but counted its outcome when the
//! report was queued, so a flushed failing report counted as a failure in the
//! assertion statistics (`assertion_fail_total`, the `[COV]` summary and the
//! `XEZIM_COV_DB` file) although nothing was reported. The copies of a
//! generate loop also superseded each other's reports in a settle, so a
//! failing copy's action never ran.

use std::path::PathBuf;
use std::process::Command;
use xezim::simulate;

/// Run `src` through the binary with the coverage database in a private
/// directory; return the database's `assertions` list and the output.
fn run_db(tag: &str, src: &str) -> (Vec<serde_json::Value>, String, String) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("deferred_counts_{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.sv");
    std::fs::write(&file, src).unwrap();
    let db = dir.join("cov.json");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .env("XEZIM_COV_DB", &db)
        .current_dir(&dir)
        .arg("--no-cache")
        .arg(&file)
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{stdout}{stderr}");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&db).expect("coverage database")).unwrap();
    let list = json["assertions"].as_array().cloned().unwrap_or_default();
    (list, stdout, stderr)
}

/// `(pass, fail)` of the site named `name`, if it has an entry.
fn site(list: &[serde_json::Value], name: &str) -> Option<(u64, u64)> {
    list.iter()
        .find(|a| a["name"] == name)
        .map(|a| (a["pass"].as_u64().unwrap(), a["fail"].as_u64().unwrap()))
}

const FLUSH: &str = r#"module add(input logic [3:0] a, b, output logic [4:0] s);
  assign s = a + b;
  always_comb c_zero : assert #0 (s == a + b) else $error("c_zero failed");
  always_comb c_final : assert final (s == a + b) else $error("c_final failed");
endmodule

module top;
  logic [3:0] a, b;
  logic [4:0] s;
  int x, y;
  add u (.a, .b, .s);
  always @(x) p_zero : assert #0 (x == y) else $error("p_zero failed");
  always @(x) p_final : assert final (x == y) else $error("p_final failed");
  initial begin
    for (int i = 0; i < 20; i++) begin
      a = i;
      b = 3 * i;
      #1;
    end
    for (int i = 1; i <= 5; i++) begin
      y = i;
      x = -i;  // both `always @(x)` wake and queue a failing report
      #0 x = i;  // they resume in the same time slot: a flush point
      #1;
    end
    $finish;
  end
endmodule
"#;

/// §16.4.2: each `always @(x)` queues a failing report, resumes on the second
/// write and matures a passing one; each `always_comb` matures one passing
/// report per time slot it ran in, however often it ran before the
/// continuous assignment settled. No failure is reported, so none is counted.
#[test]
fn flushed_deferred_reports_are_no_attempts() {
    let (list, out, err) = run_db("flush", FLUSH);
    assert!(!out.contains("failed") && !err.contains("failed"), "{out}{err}");
    assert_eq!(site(&list, "c_zero"), Some((20, 0)), "{list:?}");
    assert_eq!(site(&list, "c_final"), Some((20, 0)), "{list:?}");
    assert_eq!(site(&list, "p_zero"), Some((5, 0)), "{list:?}");
    assert_eq!(site(&list, "p_final"), Some((5, 0)), "{list:?}");
    let sim = simulate(FLUSH, 1000).expect("simulate");
    assert_eq!(sim.assertion_fail_total(), 0);
    assert_eq!(sim.assertion_pass_total(), 50);
}

const FINISH: &str = r#"module top;
  initial begin
    a_end : assert #0 (1) $finish;
    a_bad : assert #0 (0) else $display("a_bad failed");
    c_none : cover #0 (0);
    a_late : assert final (1);
  end
endmodule
"#;

/// §16.4: the slot's `#0` reports mature together in the Observed region,
/// before their actions run, so `a_end`'s `$finish` leaves `a_bad` counted as
/// a failure although its action no longer runs. The Postponed region, where
/// `a_late` would mature, is never reached, so `a_late` makes no attempt.
/// Only `a_bad` had a failure action left, so only it is noted: the cover
/// that did not match has none (§16.14.3).
#[test]
fn finish_in_a_deferred_action_leaves_its_region_counted() {
    let (list, out, err) = run_db("finish", FINISH);
    assert_eq!(site(&list, "a_end"), Some((1, 0)), "{list:?}");
    assert_eq!(site(&list, "a_bad"), Some((0, 1)), "{list:?}");
    assert_eq!(site(&list, "a_late").unwrap_or((0, 0)), (0, 0), "{list:?}");
    assert!(!out.contains("a_bad failed"), "{out}");
    let notes: Vec<&str> = err.lines().filter(|l| l.contains("did not run before $finish")).collect();
    assert_eq!(notes.len(), 1, "{err}");
    assert!(notes[0].contains(":4"), "{err}");
}

const COPIES: &str = r#"module top;
  logic [3:0] x = 4'b1101;
  for (genvar i = 0; i < 4; i++) begin : g
    always_comb begin : chk
      logic b;
      b = x[i];
      a_zero : assert #0 (b) else $display("g[%0d] a_zero failed at %0t", i, $time);
      a_final : assert final (b) else $display("g[%0d] a_final failed at %0t", i, $time);
    end
  end
  initial begin
    #1 x = 4'b1111;
    #1 $finish;
  end
endmodule
"#;

/// §16.4.2: only a re-execution of the same `always_comb` supersedes its
/// pending report. The four generate copies share their assertion
/// statements; at time 0 they run in one settle, so each copy's report must
/// survive the other copies: `g[1]` fails there and reports its failure
/// once. Each copy runs once at time 0 and once at time 1, so each statement
/// has 8 attempts, one of them failing.
#[test]
fn generate_copies_keep_their_own_deferred_reports() {
    let (list, out, _) = run_db("copies", COPIES);
    for name in ["a_zero", "a_final"] {
        assert_eq!(out.matches(&format!("g[1] {name} failed at 0")).count(), 1, "{out}");
        assert_eq!(site(&list, name), Some((7, 1)), "{name}: {list:?}");
    }
    assert_eq!(out.matches("failed").count(), 2, "{out}");
}
