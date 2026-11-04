//! §6.20.2/§8.25/§25.9: specialization arguments use the caller's lexical
//! parameters. A parameter declared by an unrelated class cannot hide a
//! module constant or split the matching virtual-interface specializations.

use xezim::simulate;

fn trace(source: &str) -> Vec<String> {
    simulate(source, 20)
        .expect("parameter-scope test must simulate")
        .output
        .iter()
        .map(|line| line.message.clone())
        .filter(|line| line.starts_with("T|"))
        .collect()
}

#[test]
fn unrelated_class_parameters_do_not_hide_module_constants() {
    let source = r#"
class unrelated #(int WIDTH=99); endclass
class store #(int N=0);
  static int value=0;
  static function void put(int v); value=v; endfunction
  static function int get(); return value; endfunction
endclass
module top;
  localparam int WIDTH=4;
  initial begin
    store#(WIDTH)::put(17);
    store#(WIDTH+1)::put(29);
    $display("T|module|%0d|%0d",store#(4)::get(),store#(5)::get());
    $finish;
  end
endmodule
"#;
    assert_eq!(trace(source), ["T|module|17|29"]);
}

#[test]
fn inherited_vif_alias_matches_the_module_parameter_specialization() {
    let source = r#"
interface data_if #(int DATA_WIDTH=32); logic [DATA_WIDTH-1:0] data; endinterface
class store #(type T=int);
  static int value=0;
  static function void put(int v); value=v; endfunction
  static function int get(); return value; endfunction
endclass
class agent_base #(int DATA_WIDTH=32);
  typedef virtual interface data_if#(32) vif_t;
  virtual function int get(); return store#(vif_t)::get(); endfunction
endclass
class agent_derived #(int DATA_WIDTH=32) extends agent_base#(DATA_WIDTH);
  virtual function int get(); return super.get(); endfunction
endclass
module top;
  localparam int DATA_WIDTH=32;
  data_if#(DATA_WIDTH) bus();
  initial begin
    agent_derived#(32) agent;
    agent=new;
    store#(virtual data_if#(DATA_WIDTH))::put(71);
    $display("T|vif|%0d|%0d",agent.get(),store#(virtual data_if#(16))::get());
    $finish;
  end
endmodule
"#;
    assert_eq!(trace(source), ["T|vif|71|0"]);
}

#[test]
fn active_and_inherited_class_parameters_still_shadow_module_constants() {
    let source = r#"
class store #(int N=0);
  static int value=0;
  static function void put(int v); value=v; endfunction
  static function int get(); return value; endfunction
endclass
class agent_base #(int WIDTH=8);
  function int get(); return store#(WIDTH)::get(); endfunction
  static function int get_static(); return store#(WIDTH)::get(); endfunction
endclass
class agent_derived #(int WIDTH=19) extends agent_base#(8); endclass
module top;
  localparam int WIDTH=4;
  initial begin
    agent_base#(8) base_agent;
    agent_derived#(19) derived_agent;
    base_agent=new;
    derived_agent=new;
    store#(4)::put(17);
    store#(8)::put(41);
    $display("T|class|%0d|%0d|%0d",base_agent.get(),derived_agent.get(),agent_base#(8)::get_static());
    $finish;
  end
endmodule
"#;
    assert_eq!(trace(source), ["T|class|41|41|41"]);
}

#[test]
fn compound_fold_cache_keeps_module_and_class_scopes_separate() {
    let source = include_str!("module_parameter_specialization_874_modes.sv");
    assert_eq!(
        trace(source),
        ["T|module|17", "T|cache|33|77|22|33|77|55|88|44|55|88"]
    );
}

#[test]
fn class_type_parameters_and_body_localparams_keep_their_scope() {
    let source = r#"
class store #(int N=0);
  static int value=0;
  static function void put(int v); value=v; endfunction
  static function int get(); return value; endfunction
endclass
class typed_store #(type T=int);
  static int value=0;
  static function void put(int v); value=v; endfunction
  static function int get(); return value; endfunction
endclass
class type_caller #(type WIDTH=int);
  function int get(); return typed_store#(WIDTH)::get(); endfunction
endclass
class value_caller #(int WIDTH=8);
  localparam int COUNT=WIDTH+1;
  function int get(); return store#(COUNT)::get(); endfunction
endclass
module top;
  localparam int WIDTH=4;
  localparam int COUNT=3;
  initial begin
    type_caller#(int) typed;
    value_caller#(8) numeric;
    typed=new;
    numeric=new;
    typed_store#(int)::put(61);
    store#(9)::put(73);
    $display("T|declarations|%0d|%0d",typed.get(),numeric.get());
    $finish;
  end
endmodule
"#;
    assert_eq!(trace(source), ["T|declarations|61|73"]);
}

#[test]
fn specialization_scopes_match_in_interpreted_and_compiled_methods() {
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SEQUENCE: AtomicU32 = AtomicU32::new(0);
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "parameter_scope_874_{}_{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).expect("create mode-test directory");
    std::fs::write(
        directory.join("t.sv"),
        include_str!("module_parameter_specialization_874_modes.sv"),
    )
    .expect("write mode-test source");
    for (name, value) in [("XEZIM_COMPILE_METHODS", "0"), ("XEZIM_METHOD_TIER", "0")] {
        let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
            .current_dir(&directory)
            .args(["--no-cache", "--max-time", "20", "-s", "top", "t.sv"])
            .env_remove("XEZIM_COMPILE_METHODS")
            .env_remove("XEZIM_METHOD_TIER")
            .env(name, value)
            .output()
            .expect("run method-mode regression");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{name}: {stdout}{stderr}");
        let actual: Vec<_> = stdout
            .lines()
            .filter(|line| line.starts_with("T|"))
            .collect();
        assert_eq!(
            actual,
            ["T|module|17", "T|cache|33|77|22|33|77|55|88|44|55|88"],
            "{name}: {stdout}{stderr}"
        );
    }
    std::fs::remove_dir_all(directory).expect("remove mode-test directory");
}

#[test]
fn config_db_get_through_a_parameterized_base_keeps_the_interface_handle() {
    let checkout = super::uvm_integration_tests::uvm_dir();
    let source = include_str!("module_parameter_specialization_874.sv");
    for version in ["1.2", "1800.2-2020"] {
        let include = checkout.join(version).join("src");
        let package = std::fs::read_to_string(include.join("uvm_pkg.sv"))
            .expect("read configured UVM package");
        let simulation = xezim::simulate_multi(
            &[package, source.to_string()],
            50_000,
            Some("top"),
            &[include.to_string_lossy().into_owned()],
            &[],
            None,
            false,
            None,
            None,
            &[],
            &[],
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
        .unwrap_or_else(|error| panic!("UVM {version}: {error}"));
        let output = simulation
            .output
            .iter()
            .map(|line| line.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            output.contains("T|config_vif|71"),
            "UVM {version}: {output}"
        );
        let fatal = output.lines().any(|line| {
            let Some(rest) = line.trim_start().strip_prefix("UVM_FATAL") else {
                return false;
            };
            match rest.trim_start().strip_prefix(':') {
                Some(count) => count.trim() != "0",
                None => true,
            }
        });
        assert!(!fatal, "UVM {version}: {output}");
    }
}
