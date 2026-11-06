//! §10.10: inside a method, an unpacked array concatenation assigned to a
//! dynamic array or queue property reached through a handle (`cfg.r`,
//! `this.cfg.q`, `s[i].used`) gives the property exactly the elements on the
//! right, including when the right side reads the property itself.
#[test]
fn concat_write_through_handle_member_in_method() {
    let result = xezim::simulate(
        r#"
class cfg_c;
  bit [4:0] r[];
  int q[$];
endclass
class node_c;
  bit [4:0] used[];
endclass
class top_c;
  cfg_c cfg;
  node_c s[3];
  bit [4:0] base[3] = '{2, 3, 4};
  function void run();
    cfg = new;
    cfg.r = {14, 18, 5};
    $display("A %p", cfg.r);
    this.cfg.q = {1, 2};
    cfg.q = {cfg.q, 3};
    $display("B %p", cfg.q);
    foreach (s[i]) s[i] = new;
    foreach (s[i])
      foreach (base[j])
        if (i != j) s[i].used = {s[i].used, base[j]};
    $display("C %p %p %p", s[0].used, s[1].used, s[2].used);
  endfunction
endclass
module t;
  top_c x;
  initial begin
    x = new;
    x.run();
  end
endmodule
"#,
        100_000,
    )
    .unwrap();
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(
        lines,
        [
            "A '{'he, 'h12, 'h5}",
            "B '{1, 2, 3}",
            "C '{'h3, 'h4} '{'h2, 'h4} '{'h2, 'h3}",
        ]
    );
}

/// The concatenation write converts each item to the member's element type,
/// runs operand calls once, and splices nested braces into the member.
#[test]
fn concat_write_through_handle_member_element_type_calls_and_nesting() {
    let result = xezim::simulate(
        r#"
class cfg_c;
  bit [4:0] r[];
  int q[$];
endclass
class top_c;
  cfg_c cfg;
  int calls;
  function int next_value();
    calls++;
    return calls * 10;
  endfunction
  function void run();
    cfg = new;
    cfg.r = {63, 32'hx};
    $display("A %p", cfg.r);
    cfg.q = {next_value()};
    $display("B %0d %p", calls, cfg.q);
    cfg.q = {1, {2, 3}};
    $display("C %p", cfg.q);
  endfunction
endclass
module t;
  top_c x;
  initial begin
    x = new;
    x.run();
  end
endmodule
"#,
        100_000,
    )
    .unwrap();
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(lines, ["A '{'h1f, 'h0}", "B 1 '{10}", "C '{1, 2, 3}"]);
}
