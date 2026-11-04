//! IEEE 1800 §§7.4.1, 7.4.6, 8.11, 13.3, 13.4, 23.6: a task's static
//! hierarchical memory read may bind to dense storage without treating its
//! module prefixes as class values. Preserve shadowing and select semantics.

use std::process::Command;
use xezim::simulate;

fn check(source: &str, marker: &str) {
    let sim = simulate(source, 1000).expect("static memory path simulation failed");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .collect();
    assert!(!lines.iter().any(|line| line.contains("FAIL")), "{lines:?}");
    assert!(lines.contains(&marker), "missing {marker:?}: {lines:?}");
}

const BOUNDS: &str = r#"
module memory_leaf;
  logic [79:0] positive [3:6];
  logic [15:0] negative [-1:-4];
  logic [7:0] descending [7:4];
  initial begin
    for (int i = 3; i <= 6; i++) positive[i] = 80'h1122_3344_5566_7788_9900 + i;
    for (int i = -4; i <= -1; i++) negative[i] = 16'h7000 + i;
    for (int i = 4; i <= 7; i++) descending[i] = 8'h80 + i;
  end
endmodule
module memory_middle; memory_leaf ram(); endmodule
module memory_outer; memory_middle sub(); endmodule
module top;
  memory_outer dut();
  task automatic verify(input int index);
    logic [79:0] wide;
    logic [3:0] unknown_index;
    #1;
    wide = dut.sub.ram.positive[index];
    if (wide !== 80'h1122_3344_5566_7788_9900 + index) $display("FAIL wide read");
    if (dut.sub.ram.negative[-index + 2] !== 16'h7000 - index + 2) $display("FAIL negative labels");
    if (dut.sub.ram.descending[index + 1] !== 8'h80 + index + 1) $display("FAIL descending labels");
    unknown_index = 'x;
    if (dut.sub.ram.positive[unknown_index] !== 80'hxxxx_xxxx_xxxx_xxxx_xxxx) $display("FAIL unknown index");
    if (dut.sub.ram.positive[2] !== 80'hxxxx_xxxx_xxxx_xxxx_xxxx) $display("FAIL below range");
    if (dut.sub.ram.positive[7] !== 80'hxxxx_xxxx_xxxx_xxxx_xxxx) $display("FAIL above range");
  endtask
  initial begin
    #1;
    for (int i = 3; i <= 6; i++) verify(i);
    $display("STATIC_BOUNDS_PASS");
    $finish;
  end
endmodule
"#;

#[test]
fn cloned_task_reads_static_memory_with_declared_index_bounds() {
    check(BOUNDS, "STATIC_BOUNDS_PASS");
}

const SHADOWS: &str = r#"
class shadow_bank;
  int memory[4];
  function new(); for (int i = 0; i < 4; i++) memory[i] = 90 + i; endfunction
endclass
class shadow_holder;
  shadow_bank ram;
  function new(); ram = new(); endfunction
endclass
module shadow_leaf;
  int memory[4];
  initial for (int i = 0; i < 4; i++) memory[i] = 10 + i;
endmodule
module shadow_middle; shadow_leaf ram(); endmodule
module top;
  typedef struct packed { logic [3:0][7:0] memory; } bank_t;
  typedef struct packed { bank_t ram; } holder_t;
  shadow_middle dut();
  task automatic class_shadow;
    shadow_holder dut = new();
    #1;
    if (dut.ram.memory[1] != 91) $display("FAIL class local shadow");
  endtask
  task automatic struct_shadow;
    holder_t dut = 32'h4433_2211;
    #1;
    if (dut.ram.memory[1] !== 8'h22) $display("FAIL struct local shadow");
  endtask
  task automatic actual_module;
    #1;
    if (dut.ram.memory[1] != 11) $display("FAIL module after local shadow");
  endtask
  initial begin
    class_shadow(); struct_shadow(); actual_module();
    class_shadow(); actual_module();
    $display("STATIC_SHADOW_PASS"); $finish;
  end
endmodule
"#;

#[test]
fn local_class_and_struct_roots_shadow_a_same_named_module() {
    check(SHADOWS, "STATIC_SHADOW_PASS");
}

/// §8.9/§8.25: a typedef-qualified static member retains class dispatch,
/// including when its spelling is also an enclosing module instance's name.
#[test]
fn typedef_qualified_static_array_does_not_bind_to_a_module_array() {
    check(
        r#"
class static_bank_c;
  static int memory[4];
endclass
typedef static_bank_c dut;
module alias_leaf;
  int memory[4];
  initial for (int i = 0; i < 4; i++) memory[i] = 10 + i;
endmodule
module top;
  alias_leaf dut();
  task automatic verify_alias;
    static_bank_c::memory[1] = 91;
    #1;
    if (dut::memory[1] != 91) $display("FAIL typedef-qualified static memory");
  endtask
  initial begin
    verify_alias();
    $display("STATIC_TYPEDEF_PASS"); $finish;
  end
endmodule
"#,
        "STATIC_TYPEDEF_PASS",
    );
}

const ONCE: &str = r#"
class selected_bank;
  int memory[4];
  function new(); for (int i = 0; i < 4; i++) memory[i] = 70 + i; endfunction
endclass
class selected_holder;
  selected_bank ram;
  int calls = 0;
  function new(); ram = new(); endfunction
  function selected_bank pick(); calls++; return ram; endfunction
endclass
module selected_leaf;
  int memory[4];
  initial for (int i = 0; i < 4; i++) memory[i] = 30 + i;
endmodule
module selected_middle; selected_leaf ram(); endmodule
module top;
  selected_middle dut();
  int index_calls = 0;
  function automatic int pick_index(); index_calls++; return 1; endfunction
  task automatic verify;
    selected_holder object = new();
    int value;
    #1;
    value = dut.ram.memory[pick_index()];
    if (value != 31 || index_calls != 1) $display("FAIL static index evaluated again");
    value = object.pick().memory[1];
    if (value != 71 || object.calls != 1 || index_calls != 1) $display("FAIL effectful receiver");
    value = dut.ram.memory[pick_index()];
    if (value != 31 || index_calls != 2) $display("FAIL static index after object receiver");
  endtask
  initial begin verify(); $display("STATIC_ONCE_PASS"); $finish; end
endmodule
"#;

#[test]
fn static_indices_and_effectful_object_receivers_execute_once() {
    check(ONCE, "STATIC_ONCE_PASS");
}

const PACKED: &str = r#"
module packed_leaf;
  typedef struct packed { logic [1:0][7:0] lane; } packet_t;
  packet_t packet;
  initial packet = 16'h4a39;
endmodule
module packed_middle; packed_leaf ram(); endmodule
module top;
  packed_middle dut();
  task automatic verify(input int index);
    logic [7:0] value;
    #1;
    value = dut.ram.packet.lane[index];
    if (value !== (index == 0 ? 8'h39 : 8'h4a)) $display("FAIL packed member lane");
  endtask
  initial begin
    verify(0); verify(1);
    $display("STATIC_PACKED_PASS"); $finish;
  end
endmodule
"#;

#[test]
fn packed_struct_array_members_keep_their_declared_geometry() {
    check(PACKED, "STATIC_PACKED_PASS");
}

const WORK: &str = r#"
module work_leaf;
  int memory[4];
  initial begin memory[0] = 11; memory[1] = 22; memory[2] = 33; memory[3] = 44; end
endmodule
module work_depth1; work_leaf u(); endmodule
module work_depth2; work_depth1 u(); endmodule
module work_depth3; work_depth2 u(); endmodule
module work_depth4; work_depth3 u(); endmodule
module work_depth5; work_depth4 u(); endmodule
module work_depth6; work_depth5 u(); endmodule
module work_depth7; work_depth6 u(); endmodule
module work_depth8; work_depth7 u(); endmodule
module top;
  work_depth8 dut();
  int checksum = 0;
  task automatic read_word(input int index);
    #1;
    checksum += dut.u.u.u.u.u.u.u.u.memory[index];
  endtask
  initial begin
    #1;
    for (int i = 0; i < 64; i++) read_word(i & 3);
    if (checksum != 1760) $display("FAIL work checksum %0d", checksum);
    $display("STATIC_WORK_PASS checksum=%0d", checksum);
    $finish;
  end
endmodule
"#;

#[test]
fn static_module_paths_do_not_repeat_speculative_receiver_lookups() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("static_memory_work_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("work.sv");
    std::fs::write(&file, WORK).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "--no-cache",
            "-s",
            "top",
            file.to_str().unwrap(),
        ])
        .env("XEZIM_NAME_STATS", "1")
        .env("XEZIM_JIT", "0")
        .output()
        .expect("run static memory work counter");
    let _ = std::fs::remove_dir_all(&dir);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{text}");
    assert!(
        text.contains("STATIC_WORK_PASS checksum=1760") && !text.contains("FAIL"),
        "{text}"
    );
    let stats = text
        .lines()
        .find(|line| line.starts_with("[NAME-STATS]"))
        .unwrap_or_else(|| panic!("missing name lookup counters: {text}"));
    for key in ["get_signal_by_name=", "resolve_hier_name="] {
        let count: u64 = stats
            .split_once(key)
            .unwrap()
            .1
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        // A static 64-read path has bounded bookkeeping. Repeated attempts
        // to evaluate the module prefixes as class values violate this work
        // budget; no wall-clock threshold is used.
        assert!(count <= 4096, "excessive {key}{count}: {text}");
    }
}
