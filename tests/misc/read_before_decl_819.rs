//! §6.5: module data must be declared before a read or write, including
//! expressions inside calls, timing controls and assertions (#819).
//! Scoped and hierarchical names and names declared in inner scopes keep
//! their own binding. Relaxation is tested only in child processes because
//! the parser's policy switches are process-global.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

const ISSUE: &str = include_str!("../issue_cases/read_before_decl_819.sv");
const RELAXED: &str = include_str!("../issue_cases/read_before_decl_819_relaxed.sv");
const CLASS_SCOPES: &str = include_str!("../issue_cases/read_before_decl_819_scopes.sv");
const RELAX_ENV: &str = "XEZIM_RELAX_USE_BEFORE_DECL";

fn rejects(case: &str, source: &str, name: &str) {
    let error = match xezim::simulate(source, 20) {
        Ok(result) => panic!("{case}: accepted forward use: {:?}", result.output),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains(&format!("'{name}'")) && error.contains("before its declaration"),
        "{case}: expected the declaration-order diagnostic for {name}:\n{error}"
    );
}

fn rejects_body(case: &str, body: &str) {
    rejects(
        case,
        &format!(
            "module t; logic [31:0] result = 0; logic clk = 0;\n\
             typedef int word_t; int q[$];\n\
             function automatic int identity(input int value); return value; endfunction\n\
             task automatic take(input int value); result = value; endtask\n\
             {body}\nint later = 1; endmodule"
        ),
        "later",
    );
}

fn passes(source: &str, expected: &str) {
    let result = xezim::simulate(source, 20).expect("legal scope must elaborate and run");
    let messages: Vec<_> = result
        .output
        .iter()
        .map(|line| line.message.as_str())
        .collect();
    assert!(
        messages.contains(&expected),
        "expected {expected:?}: {messages:?}"
    );
    assert!(
        !messages.iter().any(|line| line.contains("FAIL")),
        "legal scope resolved to the wrong declaration: {messages:?}"
    );
}

#[test]
fn expression_and_statement_reads_require_prior_declarations() {
    for (case, body) in [
        ("continuous", "assign result = later;"),
        ("net_initializer", "wire observed = later;"),
        ("variable_initializer", "int observed = later;"),
        ("always_comb", "always_comb result = later;"),
        ("always_ff", "always_ff @(posedge clk) result <= later;"),
        ("always_star", "always @* result = later;"),
        ("blocking", "initial result = later;"),
        ("nonblocking", "initial result <= later;"),
        ("nested_block", "initial begin : blk result = later; end"),
        ("fork", "initial fork result = later; join"),
        ("task_body", "task read_it; result = later; endtask"),
        (
            "function_body",
            "function int read_it(); return later; endfunction",
        ),
        ("lvalue_index", "initial result[later] = 1;"),
        ("part_select", "initial result[later +: 2] = 1;"),
        ("concatenation", "initial result = {later, 1'b0};"),
        ("replication", "initial result = {2{later}};"),
        ("ternary", "initial result = later ? 1 : 0;"),
        ("if_condition", "initial if (later) result = 1;"),
        ("while_condition", "initial while (later) break;"),
        ("do_condition", "initial do result++; while (later);"),
        ("repeat_count", "initial repeat (later) result++;"),
        ("wait_condition", "initial wait (later) result = 1;"),
        (
            "case_expression",
            "initial case (later) 1: result = 1; endcase",
        ),
        (
            "case_item",
            "initial case (result) later: result = 1; endcase",
        ),
        (
            "randcase_weight",
            "initial randcase later: result = 1; endcase",
        ),
        (
            "for_initializer",
            "initial for (int i = later; i < 2; i++) ;",
        ),
        ("for_condition", "initial for (int i = 0; i < later; i++) ;"),
        ("for_step", "initial for (int i = 0; i < 2; i += later) ;"),
        (
            "generate_read",
            "if (1) begin : g assign result = later; end",
        ),
        ("generate_write", "if (1) begin : g initial later = 2; end"),
        (
            "generate_wrapper",
            "generate assign result = later; endgenerate",
        ),
        (
            "for_scope_ends",
            "initial begin for (int later = 0; later < 1; later++) ; result = later; end",
        ),
        (
            "foreach_scope_ends",
            "initial begin foreach (q[later]) ; result = later; end",
        ),
    ] {
        rejects_body(case, body);
    }
}

/// §6.5 applies to argument expressions even when another simulator accepts
/// that form. An explicit relaxation is required for that source.
#[test]
fn calls_casts_and_assertions_require_prior_declarations() {
    for (case, body) in [
        ("function_argument", "initial result = identity(later);"),
        (
            "named_argument",
            "initial result = identity(.value(later));",
        ),
        ("task_argument", "initial take(later);"),
        ("method_argument", "initial q.push_back(later);"),
        ("system_task_argument", "initial $display(\"%0d\", later);"),
        (
            "system_function_argument",
            "initial result = $clog2(later);",
        ),
        ("bits_argument", "initial result = $bits(later);"),
        (
            "sformatf_argument",
            "initial $display(\"%s\", $sformatf(\"%0d\", later));",
        ),
        ("type_cast", "initial result = int'(later);"),
        ("size_cast", "initial result = 8'(later);"),
        ("named_cast", "initial result = word_t'(later);"),
        ("assert_condition", "initial assert (later) result = 1;"),
        ("assume_condition", "initial assume (later) result = 1;"),
        ("cover_condition", "initial cover (later) result = 1;"),
        ("assert_action", "initial assert (1) result = later;"),
        ("assert_else", "initial assert (0) ; else result = later;"),
        (
            "concurrent_assert",
            "assert property (@(posedge clk) later);",
        ),
        (
            "concurrent_disable",
            "assert property (@(posedge clk) disable iff (later) 1);",
        ),
    ] {
        rejects_body(case, body);
    }
}

#[test]
fn timing_controls_require_prior_declarations() {
    for (case, body) in [
        ("posedge", "initial @(posedge later) result = 1;"),
        ("negedge", "initial @(negedge later) result = 1;"),
        ("bare_event", "initial @later result = 1;"),
        ("event_expression", "initial @(later) result = 1;"),
        ("event_list", "initial @(later or clk) result = 1;"),
        ("event_index", "initial @(later[0]) result = 1;"),
        ("event_iff", "initial @(posedge clk iff later) result = 1;"),
        ("delay", "initial #later result = 1;"),
        ("nba_delay", "initial result <= #later 1;"),
    ] {
        rejects_body(case, body);
    }
    rejects(
        "event_trigger",
        "module t; initial ->later; event later; endmodule",
        "later",
    );
}

#[test]
fn parameters_dimensions_and_declarator_order_are_checked() {
    for (case, body) in [
        ("localparam", "localparam int WIDTH = $bits(later);"),
        ("parameter", "parameter int WIDTH = $bits(later);"),
        ("packed_dimension", "logic [$bits(later)-1:0] observed;"),
        ("unpacked_dimension", "logic observed[$bits(later)];"),
        (
            "typedef_dimension",
            "typedef logic [$bits(later)-1:0] observed_t;",
        ),
        (
            "local_dimension",
            "initial begin logic [$bits(later)-1:0] observed; end",
        ),
        (
            "locator_filter_capture",
            "initial q = q.find(item) with (item > later);",
        ),
    ] {
        rejects_body(case, body);
    }
    rejects(
        "same_declaration",
        "module t; int observed = later, later = 1; endmodule",
        "later",
    );
    rejects(
        "foreach_array_not_iterator",
        "module t; initial foreach (later[later]) ; int later[1]; endmodule",
        "later",
    );
    rejects(
        "header_port_dimension",
        "module t(input logic [$bits(later)-1:0] observed, input logic later); endmodule",
        "later",
    );
    rejects(
        "header_parameter",
        "module t #(parameter int WIDTH = $bits(later))(input logic later); endmodule",
        "later",
    );
    rejects(
        "default_argument_containing_scope",
        "module t; task f(input int later = later); endtask int later = 3; endmodule",
        "later",
    );
    rejects(
        "named_iterator_does_not_bind_item",
        "module t; int values[1] = '{1}; initial begin int found[$]; found = values.find(x) with (x > item); end int item = 0; endmodule",
        "item",
    );
    rejects(
        "local_scope_bypasses_object_member",
        "module t; class RandItem; rand int later; endclass RandItem obj; initial begin int ok; obj = new; ok = obj.randomize() with { later == local::later; }; end int later = 3; endmodule",
        "later",
    );
}

#[test]
fn instance_ports_and_gate_terminals_require_prior_declarations() {
    for (case, connection) in [
        ("ordered_port", "leaf u(later);"),
        ("named_port", "leaf u(.value(later));"),
        ("gate_terminal", "buf b(result, later);"),
    ] {
        rejects(
            case,
            &format!(
                "module leaf(input value); endmodule\n\
                 module t; wire result; {connection} logic later = 1; endmodule"
            ),
            "later",
        );
    }
    rejects(
        "implicit_named_port",
        "module leaf(input later); endmodule module t; leaf u(.later); logic later = 1; endmodule",
        "later",
    );
}

#[test]
fn instantiated_children_and_each_real_top_are_checked() {
    for (case, source) in [
        (
            "instantiated_child",
            r#"module child(output wire observed);
  assign observed = later;
  logic later = 1;
endmodule
module t;
  wire observed;
  child u(observed);
  initial #1 $finish;
endmodule"#,
        ),
        (
            "second_real_top",
            r#"module a;
  initial $finish;
endmodule
module z;
  wire observed;
  assign observed = later;
  logic later = 1;
endmodule"#,
        ),
    ] {
        rejects(case, source, "later");
    }
}

#[test]
fn data_receivers_are_uses_even_when_the_expression_is_dotted() {
    for (case, source) in [
        (
            "queue_receiver",
            "module t; initial later.push_back(7); int later[$]; endmodule",
        ),
        (
            "queue_size_receiver",
            "module t; int observed; initial observed = later.size(); int later[$]; endmodule",
        ),
        (
            "struct_member",
            "module t; typedef struct packed { int value; } pair_t; int observed; initial observed = later.value; pair_t later; endmodule",
        ),
        (
            "virtual_interface_member",
            "interface bus_if; int value; endinterface module t; int observed; initial observed = later.value; virtual bus_if later; endmodule",
        ),
        (
            "class_method_outer_module_read",
            "module t; class Reader; function int get(); return later; endfunction endclass int later = 7; endmodule",
        ),
        (
            "class_constraint_outer_module_read",
            "module t; class Reader; rand int value; constraint limit { value < later; } endclass int later = 7; endmodule",
        ),
        (
            "class_handle_port_constraint_capture",
            "class C; rand int value; endclass module t(input C c); task check; void'(c.randomize() with { value < later; }); endtask int later = 7; initial $finish; endmodule",
        ),
    ] {
        rejects(case, source, "later");
    }
}

#[test]
fn retained_constraint_and_checker_bodies_check_outer_module_data() {
    for (case, source) in [
        (
            "out_of_class_constraint",
            r#"module t;
  class C;
    rand int value;
    extern constraint k;
  endclass
  constraint C::k { value < later; }
  int later = 7;
  initial $finish;
endmodule"#,
        ),
        (
            "checker_body",
            r#"module t;
  checker monitor;
    a: assert property(later);
  endchecker
  int later = 7;
  initial $finish;
endmodule"#,
        ),
    ] {
        rejects(case, source, "later");
    }
}

#[test]
fn retained_constraint_and_checker_names_keep_their_own_scope() {
    passes(
        r#"module t;
  class C;
    rand int value;
    int later = 7;
    extern constraint k;
  endclass
  constraint C::k { value < later; }
  checker monitor(input logic later);
    a: assert property(later);
  endchecker
  int later = 99;
  initial begin $display("PASS retained declaration scopes"); $finish; end
endmodule"#,
        "PASS retained declaration scopes",
    );
}

#[test]
fn class_handle_port_constraints_use_the_objects_class_scope() {
    passes(
        r#"class C;
  rand int later;
endclass
module t(input C c);
  task check;
    void'(c.randomize() with { later == 5; });
  endtask
  int later = 99;
  initial begin $display("PASS class handle port scope"); $finish; end
endmodule"#,
        "PASS class handle port scope",
    );
}

#[test]
fn locals_formals_and_foreach_iterators_shadow_later_module_data() {
    passes(
        r#"module t;
  int total = 0;
  int values[3] = '{2, 3, 4};
  task automatic copy(input int later, output int value);
    value = later + 1;
  endtask
  function automatic int calc(input int later); return later + 2; endfunction
  initial begin : blk
    int later;
    int value;
    later = 5;
    copy(later, value);
    for (int later = 0; later < 3; later++) total += later;
    foreach (values[later]) total += values[later];
    if (value != 6 || calc(3) != 5 || total != 12) $display("FAIL lexical scopes");
    else $display("PASS lexical scopes");
    $finish;
  end
  int later = 99;
endmodule"#,
        "PASS lexical scopes",
    );
}

#[test]
fn generate_genvar_and_local_data_shadow_later_module_data() {
    passes(
        r#"module t;
  wire [3:0] observed;
  wire [3:0] local_observed;
  for (genvar later = 0; later < 1; later++) begin : g
    assign observed = later + 5;
  end
  if (1) begin : h
    int later = 4;
    assign local_observed = later;
  end
  int later = 99;
  initial begin
    #1;
    if (observed !== 5 || local_observed !== 4) $display("FAIL generate scopes");
    else $display("PASS generate scopes");
    $finish;
  end
endmodule"#,
        "PASS generate scopes",
    );
}

#[test]
fn class_members_are_visible_throughout_their_class() {
    passes(
        r#"module t;
  class Reader;
    function int get(); return later; endfunction
    int later = 7;
  endclass
  int later = 99;
  initial begin
    Reader reader;
    reader = new;
    if (reader.get() != 7) $display("FAIL class scope");
    else $display("PASS class scope");
    $finish;
  end
endmodule"#,
        "PASS class scope",
    );
}

#[test]
fn enum_and_randomized_object_members_keep_their_own_scope() {
    passes(CLASS_SCOPES, "PASS object constraint scope");
}

#[test]
fn default_arguments_use_the_declared_containing_scope() {
    passes(
        r#"module t;
  int result;
  int later = 3;
  task automatic store(input int later = later); result = later; endtask
  initial begin
    store();
    if (result != 3) $display("FAIL default argument scope");
    else $display("PASS default argument scope");
    $finish;
  end
endmodule"#,
        "PASS default argument scope",
    );
}

#[test]
fn inherited_and_enclosing_class_members_shadow_later_module_data() {
    passes(
        r#"module t;
  class Base;
    protected int later = 5;
  endclass
  class Derived extends Base;
    function int get(); return later; endfunction
  endclass
  class Outer;
    static int later = 7;
    class Inner;
      function int get(); return later; endfunction
    endclass
  endclass
  int later = 99;
  initial begin
    Derived derived;
    Outer::Inner nested;
    derived = new;
    nested = new;
    if (derived.get() != 5 || nested.get() != 7) $display("FAIL inherited scopes");
    else $display("PASS inherited scopes");
    $finish;
  end
endmodule"#,
        "PASS inherited scopes",
    );
}

#[test]
fn foreach_and_locator_iterators_shadow_only_within_their_body() {
    passes(
        r#"module t;
  int values[3] = '{2, 3, 4};
  int found[$];
  int total = 0;
  initial begin
    foreach (values[later]) total += values[later];
    found = values.find(later) with (later > 2);
    if (total != 9 || found.size() != 2 || found[0] != 3 || found[1] != 4)
      $display("FAIL iterator scopes");
    else $display("PASS iterator scopes");
    $finish;
  end
  int later = 99;
endmodule"#,
        "PASS iterator scopes",
    );
}

#[test]
fn qualified_and_hierarchical_references_keep_their_own_scope() {
    passes(
        r#"int later = 7;
package pk;
  int later = 11;
endpackage
module t;
  int from_hierarchy, from_unit, from_package;
  initial begin
    from_hierarchy = t.later;
    from_unit = $unit::later;
    from_package = pk::later;
  end
  int later = 19;
  initial begin
    #1;
    if (from_hierarchy != 19 || from_unit != 7 || from_package != 11)
      $display("FAIL qualified scopes");
    else $display("PASS qualified scopes");
    $finish;
  end
endmodule"#,
        "PASS qualified scopes",
    );
}

struct CliCase {
    dir: PathBuf,
}

impl CliCase {
    fn new(source: &str) -> Self {
        static SEQUENCE: AtomicU32 = AtomicU32::new(0);
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "read_before_decl_819_{}_{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create test directory");
        std::fs::write(dir.join("t.sv"), source).expect("write test source");
        Self { dir }
    }

    fn run(&self, args: &[&str], relax_env: Option<&str>, cache: bool) -> (i32, String, String) {
        self.run_files(&["t.sv"], args, relax_env, cache)
    }

    fn run_files(
        &self,
        files: &[&str],
        args: &[&str],
        relax_env: Option<&str>,
        cache: bool,
    ) -> (i32, String, String) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_xezim"));
        command
            .current_dir(&self.dir)
            .args(["-s", "t", "--max-time", "20"])
            .env_remove(RELAX_ENV)
            .env_remove("XEZIM_NO_CACHE");
        if !args.contains(&"--compile") {
            command.arg("--simulate");
        }
        if cache {
            command.args(["--verbose", "--cache-dir", "cache"]);
        } else {
            command.arg("--no-cache");
        }
        if let Some(value) = relax_env {
            command.env(RELAX_ENV, value);
        }
        let output = command.args(args).args(files).output().expect("run xezim");
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }
}

impl Drop for CliCase {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn check_strict(output: &(i32, String, String)) {
    let (code, stdout, stderr) = output;
    assert_eq!(*code, 1, "strict mode ran the source:\n{stdout}{stderr}");
    assert!(
        stderr.contains("'w1'") && stderr.contains("before its declaration"),
        "missing declaration-order error:\n{stdout}{stderr}"
    );
    assert!(
        stderr.contains("--relax-use-before-decl") && stderr.contains(RELAX_ENV),
        "missing relaxation guidance:\n{stderr}"
    );
    assert!(
        !stdout.contains("FAIL"),
        "simulation ran before rejection:\n{stdout}"
    );
}

fn check_relaxed(output: &(i32, String, String)) {
    let (code, stdout, stderr) = output;
    assert_eq!(*code, 0, "relaxed source rejected:\n{stdout}{stderr}");
    assert!(
        stdout.contains("FAIL read before declaration accepted, y1 = 1 y2 = 1"),
        "relaxed simulation changed:\n{stdout}{stderr}"
    );
    for name in ["w1", "w2"] {
        let warning = format!("'{name}' is used before its declaration in module 't'");
        assert_eq!(
            stderr.matches(&warning).count(),
            1,
            "expected one warning for {name}:\n{stderr}"
        );
    }
}

#[test]
fn issue_rejects_before_simulation_and_describes_the_escape_hatch() {
    let case = CliCase::new(ISSUE);
    check_strict(&case.run(&[], None, false));
    check_strict(&case.run(&["--compile", "-o", "rejected.xezim"], None, false));
    assert!(
        !case.dir.join("rejected.xezim").exists(),
        "rejected design produced an artifact"
    );
    for (arg, expected) in [
        ("--help", "--relax-use-before-decl"),
        ("--show-env-avail", RELAX_ENV),
    ] {
        let (code, stdout, stderr) = case.run(&[arg], None, false);
        assert_eq!(code, 0, "{arg} failed:\n{stdout}{stderr}");
        assert!(
            stdout.contains(expected) || stderr.contains(expected),
            "{arg} omitted {expected}:\n{stdout}{stderr}"
        );
    }
}

#[test]
fn flag_and_environment_relaxation_are_child_process_local() {
    let case = CliCase::new(ISSUE);
    check_relaxed(&case.run(&["--relax-use-before-decl"], None, false));
    check_relaxed(&case.run(&[], Some("1"), false));
    check_strict(&case.run(&[], Some("0"), false));
    check_strict(&case.run(&[], None, false));
}

#[test]
fn unit_data_and_net_initializers_keep_their_origin_across_files() {
    let case = CliCase::new(
        r#"module t;
  initial begin
    #1;
    if (follow === 0) $display("PASS unit files");
    else $display("FAIL unit files");
    $finish;
  end
endmodule"#,
    );
    std::fs::write(
        case.dir.join("unit.sv"),
        "int drive = 0;\nwire follow = drive;\n",
    )
    .expect("write compilation-unit declarations");
    for files in [["unit.sv", "t.sv"], ["t.sv", "unit.sv"]] {
        let (code, stdout, stderr) = case.run_files(&files, &[], None, false);
        assert_eq!(
            code, 0,
            "{files:?}: legal unit data rejected:\n{stdout}{stderr}"
        );
        assert!(
            stdout.contains("PASS unit files") && !stdout.contains("FAIL"),
            "{files:?}: unit net lost its initializer:\n{stdout}{stderr}"
        );
        assert!(
            !stderr.contains("used before its declaration"),
            "{files:?}: unit initializer attributed to the module:\n{stderr}"
        );
    }
}

#[test]
fn unit_parameters_and_initializers_keep_their_scope_across_files() {
    let case = CliCase::new(
        r#"module t;
  int observed_width;
  logic [3:0] observed_follow, observed_unit;
  initial begin
    observed_width = WIDTH;
    observed_follow = follow;
    observed_unit = $unit::source;
  end
  logic [7:0] source = 99;
  initial begin
    #1;
    // The early WIDTH read exercises declaration checking. Check the data
    // namespaces without asserting unrelated parameter-width evaluation.
    if (observed_follow === 7 && observed_unit === 7 && source === 99)
      $display("PASS unit parameter files");
    else $display("FAIL unit parameter files");
    $finish;
  end
endmodule"#,
    );
    std::fs::write(
        case.dir.join("unit.sv"),
        "logic [3:0] source = 7;\nlocalparam int WIDTH = $bits(source);\nwire [3:0] follow = source;\n",
    )
    .expect("write compilation-unit declarations");
    for files in [["unit.sv", "t.sv"], ["t.sv", "unit.sv"]] {
        let (code, stdout, stderr) = case.run_files(&files, &[], None, false);
        assert_eq!(
            code, 0,
            "{files:?}: legal unit parameter rejected:\n{stdout}{stderr}"
        );
        assert!(
            stdout.contains("PASS unit parameter files") && !stdout.contains("FAIL"),
            "{files:?}: unit parameter or initializer captured module data:\n{stdout}{stderr}"
        );
        assert!(
            !stderr.contains("used before its declaration"),
            "{files:?}: unit expression attributed to the module:\n{stderr}"
        );
    }
}

#[test]
fn unit_data_in_another_file_does_not_predeclare_shadowing_module_data() {
    let case = CliCase::new(
        r#"module t;
  wire observed;
  assign observed = source;
  int source = 1;
  initial $display("FAIL forward module data accepted");
endmodule"#,
    );
    std::fs::write(
        case.dir.join("unit.sv"),
        "logic [3:0] source = 7;\nlocalparam int WIDTH = $bits(source);\nwire [3:0] follow = source;\n",
    )
    .expect("write compilation-unit declarations");
    for files in [["unit.sv", "t.sv"], ["t.sv", "unit.sv"]] {
        let (code, stdout, stderr) = case.run_files(&files, &[], None, false);
        assert_eq!(
            code, 1,
            "{files:?}: forward module data accepted:\n{stdout}{stderr}"
        );
        assert!(
            stderr.contains("'source'") && stderr.contains("before its declaration in module 't'"),
            "{files:?}: missing declaration-order diagnostic:\n{stdout}{stderr}"
        );
        assert!(
            !stdout.contains("FAIL"),
            "{files:?}: simulation ran before rejection:\n{stdout}"
        );
    }
}

#[test]
fn repeated_child_instances_warn_once_per_definition_and_name() {
    let source = r#"module child(output wire observed);
  assign observed = later;
  logic later = 1;
endmodule
module t;
  wire left_value, right_value;
  child u_left(left_value), u_right(right_value);
  initial begin
    #1;
    $display("T|children %b %b", left_value, right_value);
    $finish;
  end
endmodule"#;
    let case = CliCase::new(source);
    for (args, relax_env) in [
        (&["--relax-use-before-decl"][..], None),
        (&[][..], Some("1")),
    ] {
        let (code, stdout, stderr) = case.run(args, relax_env, false);
        assert_eq!(code, 0, "relaxed child rejected:\n{stdout}{stderr}");
        assert!(
            stdout.contains("T|children 1 1"),
            "relaxed child connections changed:\n{stdout}{stderr}"
        );
        assert_eq!(
            stderr
                .matches("'later' is used before its declaration in module 'child'")
                .count(),
            1,
            "expected one definition warning for two instances:\n{stderr}"
        );
        assert!(
            !stderr.contains("used before its declaration in module 't'"),
            "warning attributed to the caller:\n{stderr}"
        );
    }
}

#[test]
fn relaxed_trace_matches_the_reference_and_requires_opt_in() {
    rejects("combined_strict", RELAXED, "arr");
    let (code, stdout, stderr) =
        CliCase::new(RELAXED).run(&["--relax-use-before-decl"], None, false);
    assert_eq!(code, 0, "relaxed source rejected:\n{stdout}{stderr}");
    let trace: Vec<_> = stdout
        .lines()
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(
        trace,
        [
            "T|local 8",
            "T|call 1 7 1 4",
            "T|assert pass",
            "T|formal 42",
            "T|hier 1 events 1",
        ],
        "reference trace changed:\n{stdout}{stderr}"
    );
}

#[test]
fn warm_cache_replays_warnings_and_cannot_bypass_strict_policy() {
    let case = CliCase::new(ISSUE);
    let cold = case.run(&["--relax-use-before-decl"], None, true);
    check_relaxed(&cold);
    assert!(
        cold.2.contains("CACHE] miss"),
        "cold run did not miss:\n{}",
        cold.2
    );

    let warm = case.run(&["--relax-use-before-decl"], None, true);
    check_relaxed(&warm);
    assert!(
        warm.2.contains("CACHE] hit"),
        "warm run did not hit:\n{}",
        warm.2
    );

    let from_env = case.run(&[], Some("1"), true);
    check_relaxed(&from_env);
    assert!(
        from_env.2.contains("CACHE] hit"),
        "environment policy used a different cache key:\n{}",
        from_env.2
    );

    for relax_env in [None, Some("0")] {
        let strict = case.run(&[], relax_env, true);
        check_strict(&strict);
        assert!(
            !strict.2.contains("CACHE] hit"),
            "strict process reused the relaxed design:\n{}",
            strict.2
        );
    }
}
#[test]
fn lexical_class_bases_shadow_later_module_data() {
    for (case, source) in [
        (
            "imported_base",
            r#"package p1; class Base; int other; endclass endpackage
package p2; class Base; int later; endclass endpackage
module t; import p2::Base; class Derived extends Base;
function int get(); return later; endfunction endclass
int later = 9; initial begin $display("PASS imported base"); $finish; end endmodule"#,
        ),
        (
            "imported_base_wildcard",
            r#"package p1; class Base; int other; endclass endpackage
package p2; class Base; int later; endclass endpackage
module t; import p2::*; class Derived extends Base;
function int get(); return later; endfunction endclass
int later = 9; initial begin $display("PASS wildcard base"); $finish; end endmodule"#,
        ),
        (
            "nested_base",
            r#"module t; class Outer;
class Base; int later; endclass
class Derived extends Base; function int get(); return later; endfunction endclass
endclass int later = 9;
initial begin $display("PASS nested base"); $finish; end endmodule"#,
        ),
        (
            "local_base",
            r#"class Base; int other; endclass
module t; class Base; int later; endclass
class Derived extends Base; function int get(); return later; endfunction endclass
int later = 9; initial begin $display("PASS local base"); $finish; end endmodule"#,
        ),
    ] {
        let expected = match case {
            "imported_base" => "PASS imported base",
            "imported_base_wildcard" => "PASS wildcard base",
            "nested_base" => "PASS nested base",
            "local_base" => "PASS local base",
            _ => unreachable!(),
        };
        passes(source, expected);
    }
}

#[test]
fn unrelated_class_bases_do_not_hide_later_module_data() {
    for (case, source) in [
        (
            "imported_base_capture",
            r#"package p1; class Base; int later; endclass endpackage
package p2; class Base; int other; endclass endpackage
module t; import p2::Base; class Derived extends Base;
function int get(); return later; endfunction endclass
int later = 9; initial $finish; endmodule"#,
        ),
        (
            "local_base_capture",
            r#"class Base; int later; endclass
module t; class Base; int other; endclass
class Derived extends Base; function int get(); return later; endfunction endclass
int later = 9; initial $finish; endmodule"#,
        ),
    ] {
        rejects(case, source, "later");
    }
}

#[test]
fn conditional_pattern_bindings_shadow_later_module_data() {
    for (case, source) in [
        (
            "pattern_ternary",
            r#"module t; function int get(input int value);
return (value matches .later) ? later : 0; endfunction
int later = 9; initial begin $display("PASS ternary binding"); $finish; end endmodule"#,
        ),
        (
            "pattern_paren",
            r#"module t; function int get(input int value);
if ((value matches .later)) return later; return 0; endfunction
int later = 9; initial begin $display("PASS parenthesized binding"); $finish; end endmodule"#,
        ),
        (
            "pattern_and",
            r#"module t; function int get(input int value);
if (value matches .later &&& later == 7) return later; return 0; endfunction
int later = 9; initial begin $display("PASS conditional binding"); $finish; end endmodule"#,
        ),
    ] {
        let expected = match case {
            "pattern_ternary" => "PASS ternary binding",
            "pattern_paren" => "PASS parenthesized binding",
            "pattern_and" => "PASS conditional binding",
            _ => unreachable!(),
        };
        passes(source, expected);
    }
}

#[test]
fn property_and_sequence_locals_shadow_later_module_data() {
    for (case, source) in [
        (
            "property_local",
            r#"module t; bit clk;
property p; int later; @(posedge clk) (1,later=3) |-> later==3; endproperty
ap: assert property(p); int later;
initial begin #1; $display("PASS property local"); $finish; end endmodule"#,
        ),
        (
            "sequence_local",
            r#"module t; bit clk;
sequence s; int later; @(posedge clk) (1,later=3) ##1 later==3; endsequence
ap: assert property(s); int later;
initial begin #1; $display("PASS sequence local"); $finish; end endmodule"#,
        ),
    ] {
        let expected = match case {
            "property_local" => "PASS property local",
            "sequence_local" => "PASS sequence local",
            _ => unreachable!(),
        };
        passes(source, expected);
    }
}

#[test]
fn object_randomize_arguments_name_class_properties() {
    for (case, source) in [
        (
            "object_randomize_argument",
            r#"module t; class Item; rand int later; endclass Item obj = new;
initial begin int ok; ok = obj.randomize(later);
if (ok) $display("PASS randomize argument"); else $display("FAIL randomize argument");
$finish; end int later = 9; endmodule"#,
        ),
        (
            "object_randomize_with_argument",
            r#"module t; class Item; rand int later; endclass Item obj = new;
initial begin int ok; ok = obj.randomize(later) with {later == 5;};
if (ok && obj.later == 5) $display("PASS constrained randomize argument"); else $display("FAIL constrained randomize argument");
$finish; end int later = 9; endmodule"#,
        ),
    ] {
        let expected = match case {
            "object_randomize_argument" => "PASS randomize argument",
            "object_randomize_with_argument" => "PASS constrained randomize argument",
            _ => unreachable!(),
        };
        passes(source, expected);
    }
    rejects(
        "scope_randomize_capture",
        r#"module t; initial begin int ok; ok = std::randomize(later); end
int later = 9; endmodule"#,
        "later",
    );
}
