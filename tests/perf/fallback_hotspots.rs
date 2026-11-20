//! §6.21 and §13.5: a whole output actual is overwritten on return, and
//! literal default inputs take the same width and sign as explicit inputs.

#[test]
fn output_before_read_and_literal_default_compile() {
    let dir = std::env::temp_dir().join(format!("xezim_output_default_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.sv");
    std::fs::write(
        &path,
        r#"
module top;
  typedef struct packed { bit [3:0] a, b; } pair_t;
  function automatic int coord(input pair_t p, input bit [3:0] lane = '1);
    return p.a + p.b + lane;
  endfunction
  function automatic void set_value(output int value);
    value = 7;
  endfunction
  bit clk;
  int sum;
  pair_t pair;
  always @(posedge clk) begin
    int keep;
    set_value(keep);
    sum = sum + keep + coord(pair);
  end
  initial begin
    pair = 8'h12;
    repeat (3) begin #1 clk = 1; #1 clk = 0; end
    $display("sum=%0d", sum);
    $finish;
  end
endmodule
"#,
    )
    .unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--no-cache", "--profile", "-s", "top"])
        .arg(path)
        .env("XEZIM_FALLBACK_SITES", "1")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(dir);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    assert!(text.lines().any(|l| l == "sum=75"), "{text}");
    assert!(!text.contains("reason=static_block_local"), "{text}");
    assert!(!text.contains("reason=Expr_Call_ports"), "{text}");
}

/// §13.5: a value also passed as an input, or through inout, can still
/// carry across activations.
#[test]
fn input_uses_still_observe_the_previous_activation() {
    let sim = xezim::simulate(
        r#"
module top;
  bit clk;
  int left, right;
  function automatic void add(input int prev, output int dst);
    dst = prev + 1;
  endfunction
  function automatic void bump(inout int dst);
    dst += 2;
  endfunction
  always @(posedge clk) begin
    int a;
    add(a, a);
    left = a;
  end
  always @(posedge clk) begin
    int b;
    bump(b);
    right = b;
  end
  initial begin
    repeat (3) begin #1 clk = 1; #1 clk = 0; end
    $display("sum=%0d,%0d", left, right);
    $finish;
  end
endmodule
"#,
        100,
    )
    .unwrap();
    assert!(
        sim.output.iter().any(|e| e.message.trim_end() == "sum=3,6"),
        "{:?}",
        sim.output
    );
}

/// §7.4 and §13.5.3: copy-out maps unpacked elements in declaration
/// order. Every activation starts with the output type's default, also
/// when the function returns before assigning every element.
#[test]
fn fixed_array_output_compiles_and_copies_in_declaration_order() {
    let dir = std::env::temp_dir().join(format!("xezim_array_out_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.sv");
    std::fs::write(
        &path,
        r#"
module top;
  bit clk;
  int calls, sum;
  function automatic void fill(input int step, output int row[2:4]);
    row[2] = -step;
    if (step == 2) return;
    row[3] = 8;
    row[4] = 11;
  endfunction
  always @(posedge clk) begin
    int vals[9:7];
    fill(calls, vals);
    sum += 3 * vals[9] + 2 * vals[8] + vals[7];
    calls++;
  end
  initial begin
    repeat (3) begin #1 clk = 1; #1 clk = 0; end
    $display("sum=%0d", sum);
    $finish;
  end
endmodule
"#,
    )
    .unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--no-cache", "--profile", "-s", "top"])
        .arg(path)
        .env("XEZIM_FALLBACK_SITES", "1")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(dir);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    assert!(text.lines().any(|l| l == "sum=45"), "{text}");
    assert!(!text.contains("reason=fb_local_array"), "{text}");
    assert!(!text.contains("reason=static_block_local"), "{text}");
}
