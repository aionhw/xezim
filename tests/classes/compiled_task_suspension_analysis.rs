//! A wait-free loop task whose statement-position calls reach through
//! class typedefs (`cbs_type::get_all(...)` §8.23 static scope), member
//! chains (`h.member.push_back(...)`), and package-scope queue variables
//! (`pkg_q.push_back(...)`) must plan as compiled — the suspension
//! analysis resolves each shape instead of failing closed — and produce
//! identical output in every mode. A sibling task whose callee really
//! suspends keeps the suspend-aware path.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_CASE: AtomicUsize = AtomicUsize::new(0);

struct CaseDir(PathBuf);

impl CaseDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "task_suspend_analysis_{}_{}",
            std::process::id(),
            NEXT_CASE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("create isolated test directory");
        Self(path)
    }

    fn run(&self, source: &str, compiled: bool, tier: &str, sites: bool) -> (String, String) {
        let input = self.0.join("case.sv");
        std::fs::write(&input, source).expect("write test source");
        let binary = std::path::PathBuf::from(env!("CARGO_BIN_EXE_xezim"));
        let mut cmd = Command::new("timeout");
        cmd.args(["--kill-after=5s", "60s"])
            .arg(binary)
            .args(["--simulate", "-s", "top"])
            .arg(&input)
            .env("XEZIM_COMPILE_METHODS", if compiled { "1" } else { "0" })
            .env("XEZIM_METHOD_TIER", tier)
            .env("XEZIM_METHOD_CACHE", "0");
        if sites {
            cmd.env("XEZIM_FALLBACK_SITES", "1");
        }
        let output = cmd.output().expect("run isolated simulator");
        assert!(
            output.status.success(),
            "simulation failed: {:?}\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        (
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|line| line.starts_with("T|"))
                .collect::<Vec<_>>()
                .join("\n"),
            String::from_utf8_lossy(&output.stderr).to_string(),
        )
    }
}

impl Drop for CaseDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn assert_modes(source: &str, expected: &str) {
    let case = CaseDir::new();
    assert_eq!(case.run(source, false, "0", false).0, expected, "AST");
    assert_eq!(
        case.run(source, true, "0", false).0,
        expected,
        "compiled (tier 0)"
    );
    // Default tier: the wait-free loop body takes the first-call boost.
    assert_eq!(
        case.run(source, true, "", false).0,
        expected,
        "compiled (default tier)"
    );
}

const SOURCE: &str = r#"
package pkg;
  // Package-scope queue reached from a class method.
  int pkg_q[$];

  function int pkg_count();
    return pkg_q.size();
  endfunction

  function int pkg_at(int i);
    return pkg_q[i];
  endfunction

  class registry_base;
    int m_this_type[$];
    virtual function void note(int v);
      m_this_type.push_back(v);
      pkg_q.push_back(v);
    endfunction
  endclass

  // Parameterized registry with a static getter, aliased by a typedef.
  class registry #(type T = int);
    typedef registry #(T) this_type;
    static this_type m_inst;
    static int all[$];

    static function this_type get();
      if (m_inst == null) m_inst = new;
      return m_inst;
    endfunction

    static function void get_all(output int q[$]);
      q = all;
    endfunction

    function void register(int v);
      all.push_back(v);
    endfunction
  endclass

  typedef registry #(int) cbs_type;

  class reporter extends registry_base;
    virtual function void note(int v);
      // Member-chain call plus a typedef-scoped static call.
      int q[$];
      super.note(v);
      cbs_type::get_all(q);
      if (q.size() == v)
        $display("T|registry %0d", v);
    endfunction
  endclass
endpackage

module top;
  import pkg::*;
  driver d;
  waiting w;
  pkg::reporter rep;

  class driver;
    pkg::registry_base h;
    cbs_type reg_h;
    task run_phase;
      // Wait-free loop with statement-position calls through the shapes
      // under test: a bare virtual through a class chain, and a property
      // receiver; the typedef-static, member-chain and package-queue
      // shapes live inside the callees, analyzed transitively.
      for (int i = 0; i < 4; i++) begin
        h.note(i);
        reg_h.register(i);
      end
      $display("T|looped %0d %0d", pkg_count(), pkg_at(2));
    endtask
  endclass

  class waiting;
    task handshake;
      #1;
    endtask
    task run_phase;
      for (int i = 0; i < 3; i++) begin
        handshake;
        $display("T|tick %0d", i);
      end
    endtask
  endclass

  initial begin
    rep = new;
    d = new;
    d.h = rep;
    d.reg_h = cbs_type::get();
    d.run_phase;
    w = new;
    w.run_phase;
    pkg_q.delete();
    $display("T|done");
  end
endmodule
"#;

#[test]
fn task_suspension_analysis_resolves_chained_calls() {
    assert_modes(
        SOURCE,
        concat!(
            "T|registry 0\n",
            "T|registry 1\n",
            "T|registry 2\n",
            "T|registry 3\n",
            "T|looped 4 2\n",
            "T|tick 0\n",
            "T|tick 1\n",
            "T|tick 2\n",
            "T|done",
        ),
    );
    // The wait-free loop task must actually reach the compiled path, and
    // the suspending sibling must not.
    let case = CaseDir::new();
    let (_, sites) = case.run(SOURCE, true, "0", true);
    assert!(
        sites.contains("runs-compiled scope=driver.run_phase"),
        "driver.run_phase did not compile:\n{sites}"
    );
    assert!(
        sites.contains("plan-decline reason=task-suspends scope=waiting.run_phase"),
        "waiting.run_phase compiled despite its suspending callee:\n{sites}"
    );
}
