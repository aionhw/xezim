//! Counts must describe declared invocations across dispatch and suspension,
//! independently of whether a 5 ms timer happens to sample their execution.

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

struct Fixture(PathBuf);
impl Fixture {
    fn new(source: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "xezim_methods_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("design.sv"), source).unwrap();
        Self(path)
    }

    fn run(&self, profile: bool, options: &[(&str, &str)]) -> (Vec<String>, Option<Value>) {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("XEZIM_") {
                cmd.env_remove(key);
            }
        }
        cmd.current_dir(&self.0).args([
            "--simulate",
            "--error-exit",
            "--no-cache",
            "-s",
            "top",
            "design.sv",
        ]);
        let path = self.0.join("methods.json");
        if profile {
            cmd.env("XEZIM_METHOD_PROFILE_JSON", &path);
        }
        for (key, value) in options {
            cmd.env(key, value);
        }
        let out = cmd.output().expect("run method fixture");
        assert!(
            out.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let payload = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|s| s.starts_with("CHECK") || s.starts_with("DONE"))
            .map(str::to_string)
            .collect();
        let report =
            profile.then(|| serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap());
        (payload, report)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn rows(report: &Value, method: &str) -> Vec<Value> {
    report["methods"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["method"] == method)
        .cloned()
        .collect()
}
fn row(report: &Value, class: &str, method: &str) -> Value {
    let found: Vec<_> = rows(report, method)
        .into_iter()
        .filter(|r| r["class"] == class)
        .collect();
    assert_eq!(found.len(), 1, "{class}::{method}: {report}");
    found[0].clone()
}
fn n(row: &Value, counter: &str) -> u64 {
    row[counter].as_u64().unwrap()
}

const NESTED: &str = r#"
class A;
  int value;
  static int hits;
  virtual function int same(int x); return x+1; endfunction
  virtual function int read_value(); read_value=value; endfunction
  virtual function void write_value(int x); value=x; endfunction
  static function int stat(); hits++; return 7; endfunction
  virtual function int defaults(int x=3); return x+2; endfunction
  function int run(int x);
    write_value(x); return same(read_value()) + A::stat() + defaults() + free_add(x);
  endfunction
endclass
class B;
  function int same(int x); return x+2; endfunction
endclass
class C #(int W=8);
  function int same(int x); return x+W; endfunction
endclass
class D extends C #(8);
  function int via_super(int x); return super.same(x); endfunction
endclass
module top;
  function int free_add(int x); return x+4; endfunction
  initial begin
    A a=new; B b=new; C #(8) c8=new; C #(16) c16=new; D d=new;
    int sum=0;
    for(int i=0;i<5;i++) begin
      sum += a.run(i) + b.same(i) + c8.same(i) + c16.same(i) + d.via_super(i);
    end
    if(a.read_value() != 4) $fatal(1,"accessor result");
    if(A::hits != 5) $fatal(1,"static function executed %0d times",A::hits);
    if(sum != 315) $fatal(1,"bad sum %0d",sum);
    $display("CHECK sum=%0d",sum);
    $display("DONE");
  end
endmodule
"#;

#[test]
fn scoped_counts_are_exact_across_fast_slow_and_tiered_dispatch() {
    let f = Fixture::new(NESTED);
    for (compiled, tier, fast) in [
        ("0", "0", "0"),
        ("1", "1000000", "1"),
        ("1", "2", "1"),
        ("1", "0", "0"),
        ("1", "0", "1"),
    ] {
        let options = [
            ("XEZIM_COMPILE_METHODS", compiled),
            ("XEZIM_METHOD_TIER", tier),
            ("XEZIM_FAST_CALLS", fast),
        ];
        let (normal, _) = f.run(false, &options);
        let (observed, report) = f.run(true, &options);
        assert_eq!(observed, normal);
        assert_eq!(observed, ["CHECK sum=315", "DONE"]);
        let report = report.unwrap();
        for (class, method, calls) in [
            ("A", "same", 5),
            ("B", "same", 5),
            ("A", "run", 5),
            ("A", "read_value", 6),
            ("A", "write_value", 5),
            ("A", "stat", 5),
            ("A", "defaults", 5),
            ("D", "via_super", 5),
            ("__free_fn__", "free_add", 5),
        ] {
            let r = row(&report, class, method);
            assert_eq!(n(&r, "calls"), calls, "{compiled}/{tier}/{fast}: {r}");
            assert_eq!(
                n(&r, "ast_body_entries") + n(&r, "vm_body_entries") + n(&r, "accessor_calls"),
                calls,
                "{r}"
            );
            if compiled == "0" {
                assert_eq!(n(&r, "compilation_disabled"), calls, "{r}");
            }
        }
        let mut specs = rows(&report, "same")
            .into_iter()
            .filter(|r| r["class"] == "C")
            .collect::<Vec<_>>();
        specs.sort_by_key(|r| r["specialization"].to_string());
        assert_eq!(specs.len(), 2, "{report}");
        let mut counts: Vec<_> = specs.iter().map(|r| n(r, "calls")).collect();
        counts.sort();
        assert_eq!(
            counts,
            [5, 10],
            "inherited C#8 shares its declaring specialization: {specs:?}"
        );
        assert_ne!(specs[0]["specialization"], specs[1]["specialization"]);
        if compiled == "1" && tier == "0" {
            assert_eq!(n(&row(&report, "A", "run"), "vm_body_entries"), 5);
        }
        if tier == "2" {
            let r = row(&report, "A", "run");
            assert_eq!(n(&r, "ast_body_entries"), 2);
            assert_eq!(n(&r, "vm_body_entries"), 3);
        }
        if tier == "1000000" {
            assert_eq!(n(&row(&report, "A", "run"), "tier_pending"), 5);
            assert_eq!(n(&row(&report, "A", "run"), "compile_attempts"), 0);
        }
    }
}

const TASKS: &str = r#"
`timescale 1ns/1ns
class T;
  int total;
  task inner(); #1; total++; #0; total++; endtask
  task outer(); inner(); #1; total++; endtask
  static task stat(); #1; endtask
endclass
module top;
  task automatic free_wait(); #1; #0; endtask
  initial begin
    T t=new;
    t.outer(); t.outer(); T::stat(); free_wait();
    if(t.total != 6) $fatal(1,"task result %0d",t.total);
    $display("CHECK total=%0d",t.total);
    $display("DONE");
  end
endmodule
"#;

#[test]
fn suspended_tasks_count_bindings_once_and_report_cached_declines() {
    let f = Fixture::new(TASKS);
    let options = [("XEZIM_METHOD_TIER", "0")];
    let (normal, _) = f.run(false, &options);
    let (observed, report) = f.run(true, &options);
    assert_eq!(observed, normal);
    assert_eq!(observed, ["CHECK total=6", "DONE"]);
    let report = report.unwrap();
    for (method, calls) in [("inner", 2), ("outer", 2), ("stat", 1)] {
        let r = row(&report, "T", method);
        assert_eq!(n(&r, "calls"), calls, "{r}");
        assert_eq!(n(&r, "ast_body_entries"), calls, "{r}");
        assert_eq!(n(&r, "vm_body_entries"), 0, "{r}");
        assert!(n(&r, "scheduled_segments") >= calls, "{r}");
    }
    for method in ["inner", "outer"] {
        let r = row(&report, "T", method);
        assert_eq!(r["declines"]["task-suspends"], 1, "{r}");
        assert_eq!(n(&r, "cached_rejections"), 1, "{r}");
        assert_eq!(r["cached_decline_reasons"]["task-suspends"], 1, "{r}");
    }
    let r = row(&report, "__task__", "free_wait");
    assert_eq!(n(&r, "calls"), 1);
    assert_eq!(n(&r, "ast_body_entries"), 1);
    assert!(n(&r, "scheduled_segments") > 1);
}

#[test]
fn package_task_names_remain_distinct_for_synchronous_and_scheduled_calls() {
    let f = Fixture::new(
        r#"
`timescale 1ns/1ns
package P;
  task same(); #1; endtask
  task quick(); endtask
endpackage
package Q;
  task same(); #1; endtask
  task quick(); endtask
endpackage
module top;
  initial begin
    P::same(); Q::same(); P::quick(); Q::quick();
    $display("DONE");
  end
endmodule
"#,
    );
    let (normal, _) = f.run(false, &[]);
    let (observed, report) = f.run(true, &[]);
    assert_eq!(observed, normal);
    assert_eq!(observed, ["DONE"]);
    let report = report.unwrap();
    for class in ["__task__P", "__task__Q"] {
        for method in ["same", "quick"] {
            let found: Vec<_> = report["methods"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| {
                    r["class"] == class
                        && r["method"].as_str().unwrap().rsplit("::").next() == Some(method)
                })
                .collect();
            assert_eq!(found.len(), 1, "{class}/{method}: {report}");
            assert_eq!(n(found[0], "calls"), 1);
            assert_eq!(n(found[0], "ast_body_entries"), 1);
            assert_eq!(found[0]["compilation_bypassed"]["free_task_dispatch"], 1);
        }
    }
}
