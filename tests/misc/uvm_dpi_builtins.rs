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
        None,
    )
    .expect("simulate failed");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    let has = |s: &str| msgs.iter().any(|m| m == s);
    assert!(
        has("T|re=/^.*\\.agent\\..*$/"),
        "glob_to_re translation; output: {:?}",
        msgs
    );
    assert!(has("T|m1=0 m2=1"), "uvm_re_match search semantics; output: {:?}", msgs);
    assert!(has("T|x1=0 x2=1"), "regcomp/regexec handles; output: {:?}", msgs);
    assert!(has("T|chk=1"), "uvm_hdl_check_path; output: {:?}", msgs);
    assert!(has("T|probe=a5 rd=a5"), "uvm_hdl deposit/read roundtrip; output: {:?}", msgs);
    assert!(
        has("T|arg=1 tool=xezim"),
        "argv walk must surface plusargs and the tool name; output: {:?}",
        msgs
    );
}
