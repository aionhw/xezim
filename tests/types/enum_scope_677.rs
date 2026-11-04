//! IEEE 1800-2023 §23.9 / §6.19.3: lexical declarations determine enum checks.

#[test]
fn integer_formals_and_locals_shadow_outer_enum_members() {
    let sim = xezim::simulate(
        r#"
module top;
  typedef enum {low, high} range_t;
  range_t high_value;
  int result;
  function automatic int limit(int high);
    high = 1024;
    return high;
  endfunction
  task automatic set_limit(int high);
    high = 1025;
    result = high;
  endtask
  initial begin
    $display("%0d", limit(0));
    set_limit(0);
    $display("%0d", result);
    begin int high; high = 1026; $display("%0d", high); end
  end
endmodule
"#,
        100,
    )
    .expect("integer declarations are not enums");
    let msgs: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert_eq!(msgs, ["1024", "1025", "1026"]);
}
