//! Built-in implementations of the UVM distribution's DPI-C helpers
//! (uvm_svcmd_dpi.c / uvm_regex.cc / uvm_hdl.c). These let UVM compile and
//! run WITHOUT +define+UVM_NO_DPI, which is what makes command-line
//! processing (+UVM_CONFIG_DB_TRACE, +UVM_OBJECTION_TRACE, …) reach
//! uvm_cmdline_processor — under UVM_NO_DPI the fallback returns no args in
//! every simulator, including the reference. Semantics ported from the C
//! sources; regex behavior goes through libc's POSIX-ERE engine, matching
//! the C implementation exactly.

use xezim::simulate_multi;

const SRC: &str = r#"
module top;
  import "DPI-C" function string uvm_glob_to_re(string glob);
  import "DPI-C" context function int uvm_re_match(string re, string str);
  import "DPI-C" function chandle uvm_dpi_regcomp(string regex);
  import "DPI-C" function int uvm_dpi_regexec(chandle preg, string str);
  import "DPI-C" function void uvm_dpi_regfree(chandle preg);
  import "DPI-C" function string uvm_dpi_get_next_arg_c(int init);
  import "DPI-C" function string uvm_dpi_get_tool_name_c();
  import "DPI-C" context function int uvm_hdl_check_path(string path);
  import "DPI-C" context function int uvm_hdl_deposit(string path, logic [1023:0] value);
  import "DPI-C" context function int uvm_hdl_read(string path, output logic [1023:0] value);

  reg [7:0] probe = 8'h3C;
  initial begin
    string re, a, path;
    chandle h;
    logic [1023:0] rd;
    int seen_plusarg;
    re = uvm_glob_to_re("*.agent.*");
    $display("T|re=%s", re);
    $display("T|m1=%0d m2=%0d", uvm_re_match(re, "env.agent.mon"),
             uvm_re_match(re, "env.driver") != 0);
    h = uvm_dpi_regcomp("^abc.*z$");
    $display("T|x1=%0d x2=%0d", uvm_dpi_regexec(h, "abcdz"),
             uvm_dpi_regexec(h, "abcd") != 0);
    uvm_dpi_regfree(h);
    path = uvm_hdl_check_path("top.probe") ? "top.probe" : "probe";
    $display("T|chk=%0d", uvm_hdl_check_path(path));
    void'(uvm_hdl_deposit(path, 1024'ha5));
    void'(uvm_hdl_read(path, rd));
    $display("T|probe=%h rd=%h", probe, rd[7:0]);
    seen_plusarg = 0;
    a = uvm_dpi_get_next_arg_c(1);
    while (a != "") begin
      if (a == "+UVM_CONFIG_DB_TRACE") seen_plusarg = 1;
      a = uvm_dpi_get_next_arg_c(0);
    end
    $display("T|arg=%0d tool=%s", seen_plusarg, uvm_dpi_get_tool_name_c());
  end
endmodule
"#;

#[test]
fn uvm_dpi_builtins_regex_hdl_and_argv() {
    let sim = simulate_multi(
        &[SRC.to_string()],
        100,
        Some("top"),
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &["+UVM_CONFIG_DB_TRACE".to_string()],
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
    .expect("simulate failed");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    let has = |s: &str| msgs.iter().any(|m| m == s);
    assert!(
        has("T|re=/^.*\\.agent\\..*$/"),
        "glob_to_re translation; output: {:?}",
        msgs
    );
    assert!(
        has("T|m1=0 m2=1"),
        "uvm_re_match search semantics; output: {:?}",
        msgs
    );
    assert!(
        has("T|x1=0 x2=1"),
        "regcomp/regexec handles; output: {:?}",
        msgs
    );
    assert!(has("T|chk=1"), "uvm_hdl_check_path; output: {:?}", msgs);
    assert!(
        has("T|probe=a5 rd=a5"),
        "uvm_hdl deposit/read roundtrip; output: {:?}",
        msgs
    );
    assert!(
        has("T|arg=1 tool=xezim"),
        "argv walk must surface plusargs and the tool name; output: {:?}",
        msgs
    );
}

/// The compile/execute/free regex API of later UVM releases, and UVM
/// 1.1d's argument-less argv walk (it restarts after reporting the end).
const LATER_API_SRC: &str = r#"
module top;
  import "DPI-C" function chandle uvm_re_comp(string re, bit deglob);
  import "DPI-C" function int uvm_re_exec(chandle rexp, string str);
  import "DPI-C" function void uvm_re_free(chandle rexp);
  import "DPI-C" function string uvm_re_buffer();
  import "DPI-C" function bit uvm_re_compexecfree(string re, string str, bit deglob,
                                                   output int exec_ret);
  import "DPI-C" function string uvm_re_deglobbed(string glob, bit with_brackets);
  import "DPI-C" function string uvm_dpi_get_next_arg_c();
  initial begin
    chandle h;
    int r;
    bit ok;
    h = uvm_re_comp("env.*.mon", 1);
    $display("T|comp=%0d m=%0d n=%0d", h != null, uvm_re_exec(h, "env.a.mon"),
             uvm_re_exec(h, "env.a.drv") != 0);
    uvm_re_free(h);
    h = uvm_re_comp("/a(/", 0);
    $display("T|bad=%0d buf=%0d", h == null, uvm_re_buffer() != "");
    ok = uvm_re_compexecfree("/^x+$/", "xxx", 0, r);
    $display("T|cef ok=%0d r=%0d", ok, r);
    $display("T|deglob '%s' '%s'", uvm_re_deglobbed("a*", 0), uvm_re_deglobbed("a*", 1));
    $display("T|argv %s %s '%s' %s", uvm_dpi_get_next_arg_c(), uvm_dpi_get_next_arg_c(),
             uvm_dpi_get_next_arg_c(), uvm_dpi_get_next_arg_c());
  end
endmodule
"#;

#[test]
fn uvm_dpi_builtins_later_regex_api_and_legacy_argv() {
    let sim = simulate_multi(
        &[LATER_API_SRC.to_string()],
        100,
        Some("top"),
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &["+X".to_string()],
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
    .expect("simulate failed");
    let got: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect();
    assert_eq!(
        got,
        [
            "T|comp=1 m=0 n=1",
            "T|bad=1 buf=1",
            "T|cef ok=1 r=0",
            "T|deglob '^a.*$' '/^a.*$/'",
            "T|argv xezim +X '' xezim",
        ]
    );
}

/// The regex, signal-size and polling C functions of UVM 1800.2-2020.3
/// (`uvm_regex.cc`, `uvm_hdl_polling.c`), imported directly. Every line was
/// cross-checked against the reference simulator running those C sources
/// (compiled as its `-sv_lib`): the deglob forms, `uvm_re_buffer` after a
/// deglob, a bracket strip and a failed compile, `uvm_re_compexecfree`
/// reporting REG_NOMATCH when the pattern does not compile, and a probe
/// whose value changes toggle the notifier bit and reach the exported
/// `uvm_polling_value_change_notify` once per change until disabled.
const API_2020_SRC: &str = r#"
package pollpkg;
  bit notifier;
endpackage
module dut;
  logic [7:0] cnt = 0;
  logic clk = 0;
  always #5 clk = ~clk;
  always @(posedge clk) cnt <= cnt + 1;
endmodule
module top;
  import "DPI-C" function string uvm_re_deglobbed(string glob, bit with_brackets);
  import "DPI-C" function string uvm_re_buffer();
  import "DPI-C" function void uvm_re_free(chandle rexp);
  import "DPI-C" function chandle uvm_re_comp(string re, bit deglob);
  import "DPI-C" function int uvm_re_exec(chandle rexp, string str);
  import "DPI-C" function chandle uvm_re_compexec(string re, string str, bit deglob, output int exec_ret);
  import "DPI-C" function bit uvm_re_compexecfree(string re, string str, bit deglob, output int exec_ret);
  import "DPI-C" context function chandle uvm_polling_create(input string name, input int sv_key);
  import "DPI-C" context function void uvm_polling_set_enable_callback(chandle hnd, int enable);
  import "DPI-C" context function int uvm_polling_get_callback_enable(chandle hnd);
  import "DPI-C" context function int uvm_polling_setup_notifier(string fullname);
  import "DPI-C" context function void uvm_polling_process_changelist();
  import "DPI-C" context function int uvm_hdl_signal_size(string path);
  export "DPI-C" function uvm_polling_value_change_notify;
  export "DPI-C" function m__uvm_report_dpi;
  function void m__uvm_report_dpi(int severity, string id, string message, int verbosity,
                                  string filename, int line);
    $display("REPORT sev=%0d id=%s %s", severity, id, message);
  endfunction
  int notes[$];
  function void uvm_polling_value_change_notify(int sv_key);
    notes.push_back(sv_key);
  endfunction
  dut u();
  initial begin
    chandle h;
    int r;
    bit ok;
    $display("T|deglob '%s' '%s' '%s' '%s' '%s'", uvm_re_deglobbed("", 0), uvm_re_deglobbed("", 1),
             uvm_re_deglobbed("a*b?[c]", 0), uvm_re_deglobbed("a*b?[c]", 1), uvm_re_deglobbed("/x+/", 0));
    $display("T|buffer '%s'", uvm_re_buffer());
    h = uvm_re_comp("top.*.u", 1);
    $display("T|comp glob %0d %0d", uvm_re_exec(h, "top.x.u"), uvm_re_exec(h, "top.x.v") != 0);
    uvm_re_free(h);
    h = uvm_re_comp("/^a[0-9]+$/", 0);
    $display("T|comp re %0d %0d buffer '%s'", uvm_re_exec(h, "a12"), uvm_re_exec(h, "a1b") != 0, uvm_re_buffer());
    uvm_re_free(h);
    h = uvm_re_compexec("b.d", "abcde", 0, r);
    $display("T|compexec null=%0d r=%0d", h == null, r);
    uvm_re_free(h);
    r = 5;
    ok = uvm_re_compexecfree("/a(/", "a", 0, r);
    $display("T|cef bad ok=%0d r=%0d buffer_set=%0d", ok, r, uvm_re_buffer() != "");
    ok = uvm_re_compexecfree("x*", "yyy", 1, r);
    $display("T|cef glob ok=%0d nomatch=%0d", ok, r != 0);
    $display("T|size %0d %0d %0d", uvm_hdl_signal_size("top.u.cnt"), uvm_hdl_signal_size("top.u.cnt[3:0]"),
             uvm_hdl_signal_size("top.u.clk"));
    $display("T|notifier %0d", uvm_polling_setup_notifier("pollpkg.notifier"));
    h = uvm_polling_create("top.u.cnt", 7);
    $display("T|create null=%0d enabled=%0d", h == null, uvm_polling_get_callback_enable(h));
    uvm_polling_set_enable_callback(h, 1);
    $display("T|enabled=%0d", uvm_polling_get_callback_enable(h));
    repeat (3) begin
      @(pollpkg::notifier);
      uvm_polling_process_changelist();
      $display("T|t=%0t notes=%p cnt=%0d", $time, notes, u.cnt);
    end
    uvm_polling_set_enable_callback(h, 0);
    #30;
    $display("T|after disable notes=%0d", notes.size());
    $finish;
  end
endmodule
"#;

#[test]
fn uvm_dpi_builtins_2020_regex_and_polling() {
    let sim = simulate_multi(
        &[API_2020_SRC.to_string()],
        1000,
        Some("top"),
        &[],
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
    .expect("simulate failed");
    let got: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|") || m.starts_with("REPORT"))
        .collect();
    assert_eq!(
        got,
        [
            "T|deglob '^$' '/^$/' '^a.*b.\\[c\\]$' '/^a.*b.\\[c\\]$/' 'x+'",
            "T|buffer 'x+'",
            "T|comp glob 0 1",
            "T|comp re 0 1 buffer '^a[0-9]+$'",
            "T|compexec null=0 r=0",
            "T|cef bad ok=0 r=1 buffer_set=1",
            "T|cef glob ok=1 nomatch=1",
            "T|size 8 4 1",
            "T|notifier 1",
            "T|create null=0 enabled=0",
            "T|enabled=1",
            "T|t=5 notes='{7} cnt=1",
            "T|t=15 notes='{7, 7} cnt=2",
            "T|t=25 notes='{7, 7, 7} cnt=3",
            "T|after disable notes=3",
        ]
    );
}

/// An empty pattern matches every string, as glibc's regcomp has it, on
/// every host (BSD libc rejects it with REG_EMPTY). UVM 1.2 builds one from
/// an empty scope (`uvm_config_db::set(null, "", ...)`), and `//` strips to
/// one; each compile path must accept it without touching `uvm_re_buffer`.
const EMPTY_PATTERN_SRC: &str = r#"
module top;
  import "DPI-C" function void uvm_dump_re_cache();
  import "DPI-C" function string uvm_glob_to_re(string glob);
  import "DPI-C" context function int uvm_re_match(string re, string str);
  import "DPI-C" function chandle uvm_dpi_regcomp(string regex);
  import "DPI-C" function int uvm_dpi_regexec(chandle preg, string str);
  import "DPI-C" function chandle uvm_re_comp(string re, bit deglob);
  import "DPI-C" function int uvm_re_exec(chandle rexp, string str);
  import "DPI-C" function string uvm_re_buffer();
  export "DPI-C" function m__uvm_report_dpi;
  function void m__uvm_report_dpi(int severity, string id, string message, int verbosity,
                                  string filename, int line);
    $display("REPORT sev=%0d id=%s %s", severity, id, message);
  endfunction
  initial begin
    string re;
    chandle h;
    re = uvm_glob_to_re("");
    $display("T|match '%s' %0d %0d %0d", re, uvm_re_match(re, ""),
             uvm_re_match(re, "uvm_test_top.env"), uvm_re_match("//", "x"));
    h = uvm_dpi_regcomp("");
    $display("T|regcomp null=%0d m=%0d", h == null, uvm_dpi_regexec(h, "abc"));
    h = uvm_re_comp("//", 0);
    $display("T|comp null=%0d m=%0d", h == null, uvm_re_exec(h, "abc"));
    $display("T|buffer '%s'", uvm_re_buffer());
  end
endmodule
"#;

#[test]
fn uvm_dpi_builtins_empty_pattern_matches_everything() {
    let sim = simulate_multi(
        &[EMPTY_PATTERN_SRC.to_string()],
        100,
        Some("top"),
        &[],
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
    .expect("simulate failed");
    let got: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|") || m.starts_with("REPORT"))
        .collect();
    assert_eq!(
        got,
        [
            "T|match '' 0 0 0",
            "T|regcomp null=0 m=0",
            "T|comp null=0 m=0",
            "T|buffer ''",
        ]
    );
}
