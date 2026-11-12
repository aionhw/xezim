//! LRM 13.5.2 writable actuals retain their storage across a method call;
//! LRM 8.9 static properties use class storage, including through null handles.
//! Persistent method entries must also change when any external compile-time
//! dependency changes, including package enum values and typedef targets.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_CASE: AtomicUsize = AtomicUsize::new(0);

struct CaseDir(PathBuf);

impl CaseDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "method_admission_{}_{}",
            std::process::id(),
            NEXT_CASE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("create isolated test directory");
        Self(path)
    }

    fn run(&self, source: &str, compiled: bool, fast: bool, cached: bool) -> String {
        let input = self.0.join("case.sv");
        std::fs::write(&input, source).expect("write test source");
        let binary = std::path::PathBuf::from(env!("CARGO_BIN_EXE_xezim"));
        let output = Command::new("timeout")
            .args(["--kill-after=5s", "60s"])
            .arg(binary)
            .args(["--simulate", "-s", "top"])
            .arg(&input)
            .env("XEZIM_COMPILE_METHODS", if compiled { "1" } else { "0" })
            .env("XEZIM_METHOD_TIER", "0")
            .env("XEZIM_FAST_CALLS", if fast { "1" } else { "0" })
            .env(
                "XEZIM_METHOD_CACHE",
                if cached {
                    self.0.join("cache")
                } else {
                    PathBuf::from("0")
                },
            )
            .output()
            .expect("run isolated simulator");
        assert!(
            output.status.success(),
            "simulation failed: {:?}\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| line.starts_with("T|"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Drop for CaseDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn assert_modes(source: &str, expected: &str) {
    let case = CaseDir::new();
    assert_eq!(case.run(source, false, false, false), expected, "AST");
    assert_eq!(case.run(source, true, false, false), expected, "compiled");
    assert_eq!(
        case.run(source, true, true, false),
        expected,
        "direct calls"
    );
    assert_eq!(case.run(source, true, true, true), expected, "cold cache");
    assert_eq!(case.run(source, true, true, true), expected, "warm cache");
}

#[test]
fn nonlocal_writable_actuals_preserve_storage_and_argument_effects() {
    assert_modes(
        r#"
class changer;
  function int change(ref int destination);
    destination = 19;
    return destination;
  endfunction
  function int output_value(output int destination);
    destination = 23;
    return destination;
  endfunction
  function int inout_value(inout int destination);
    destination += 2;
    return destination;
  endfunction
  function int selected_value(input int increment, ref int destination);
    destination = 18 + increment;
    return destination;
  endfunction
endclass
class holder;
  int number = 3;
endclass
class caller extends changer;
  changer operation;
  holder peer;
  int number = 5;
  int slots[2];
  int visits = 0;
  function new();
    operation = new;
    peer = new;
    slots[0] = 7;
    slots[1] = 11;
  endfunction
  function int pick(); visits++; return 1; endfunction
  function int own_member(); return change(this.number); endfunction
  function int other_member(); return operation.change(peer.number); endfunction
  function int selected_member(); return super.selected_value(pick(), slots[1]); endfunction
  function int output_member(); return operation.output_value(this.number); endfunction
  function int inout_member(); return operation.inout_value(peer.number); endfunction
endclass
module top;
  initial begin
    caller agent = new;
    int first, second, third, fourth, fifth;
    first = agent.own_member();
    second = agent.other_member();
    third = agent.selected_member();
    fourth = agent.output_member();
    fifth = agent.inout_member();
    $display("T|results %0d %0d %0d %0d %0d", first, second, third, fourth, fifth);
    $display("T|stored %0d %0d %0d visits=%0d", agent.number, agent.peer.number, agent.slots[1], agent.visits);
    $finish;
  end
endmodule
"#,
        "T|results 19 19 19 23 21\nT|stored 23 21 19 visits=1",
    );
}

#[test]
fn foreign_static_fields_use_class_storage_even_with_null_receiver() {
    assert_modes(
        r#"
class archive;
  static int shared = 37;
  int ordinary = 11;
endclass
class reader;
  int shared = 99;
  function int observe(archive source); return source.shared; endfunction
  function int change(archive source, int value);
    source.shared = value;
    return source.shared;
  endfunction
  function int ordinary(archive source); return source.ordinary; endfunction
endclass
module top;
  initial begin
    archive live = new;
    archive absent = null;
    reader agent = new;
    int first, second;
    $display("T|static %0d %0d", agent.observe(live), agent.observe(absent));
    first = agent.change(absent, 43);
    second = agent.change(live, 47);
    $display("T|changed %0d %0d %0d", first, second, agent.observe(absent));
    $display("T|ordinary %0d own=%0d", agent.ordinary(live), agent.shared);
    $finish;
  end
endmodule
"#,
        "T|static 37 37\nT|changed 43 47 47\nT|ordinary 11 own=99",
    );
}

#[test]
fn persistent_cache_tracks_package_enum_values() {
    let case = CaseDir::new();
    let source = r#"
package constants;
  typedef enum int { TOKEN = 3 } token_t;
endpackage
class decoder;
  function int read_token(); return constants::TOKEN; endfunction
endclass
module top;
  initial begin
    decoder agent = new;
    $display("T|token %0d", agent.read_token());
    $finish;
  end
endmodule
"#;
    assert_eq!(case.run(source, true, true, true), "T|token 3");
    let entries = std::fs::read_dir(case.0.join("cache"))
        .expect("cache created")
        .count();
    assert!(entries > 0, "the cold run must exercise disk caching");
    assert_eq!(case.run(source, true, true, true), "T|token 3");
    assert_eq!(
        std::fs::read_dir(case.0.join("cache")).unwrap().count(),
        entries,
        "unchanged context reuses keys"
    );
    let changed = source.replace("TOKEN = 3", "TOKEN = 7");
    assert_eq!(case.run(&changed, true, true, true), "T|token 7");
    assert_eq!(case.run(&changed, true, true, true), "T|token 7");
}

#[test]
fn persistent_cache_tracks_typedef_targets() {
    let case = CaseDir::new();
    let source = r#"
class first_kind;
  function int value(); return 13; endfunction
endclass
class other_kind;
  function int value(); return 29; endfunction
endclass
typedef first_kind selected_kind;
class factory;
  function int make_value();
    selected_kind result;
    result = new;
    return result.value();
  endfunction
endclass
module top;
  initial begin
    factory agent = new;
    $display("T|value %0d", agent.make_value());
    $finish;
  end
endmodule
"#;
    assert_eq!(case.run(source, true, true, true), "T|value 13");
    let changed = source.replace("typedef first_kind", "typedef other_kind");
    assert_eq!(case.run(&changed, false, false, false), "T|value 29");
    assert_eq!(case.run(&changed, true, true, true), "T|value 29");
    assert_eq!(case.run(&changed, true, true, true), "T|value 29");
}
