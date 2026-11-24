//! §19.7 instance options read as `cg.option.<name>`: the covergroup's own
//! setting, else the default (reads used to return x). A setting from a
//! constructor argument is read in the instance's scope, as its coverage
//! is. An option written at
//! run time is stored but, as in the reference simulator, does not change
//! the coverage. Every expected value was cross-checked against the
//! reference simulator.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect()
}

fn assert_line(lines: &[String], want: &str) {
    assert!(
        lines.iter().any(|l| l == want),
        "expected `{want}`, got {lines:?}"
    );
}

#[test]
fn read_instance_options() {
    let l = lines(
        r#"
module top;
  bit [1:0] x, y;
  covergroup cg;
    option.goal = 90;
    cpx : coverpoint x { bins b0 = {0}; bins b1 = {1}; option.weight = 2; }
    cpy : coverpoint y;
  endgroup
  covergroup cg2; cpx : coverpoint x; endgroup
  cg c = new();
  cg2 d = new();
  initial begin
    $display("goal %0d at_least %0d weight %0d abm %0d per_instance %0d", c.option.goal,
             c.option.at_least, c.option.weight, c.option.auto_bin_max, c.option.per_instance);
    $display("d goal %0d", d.option.goal);
    x = 0; y = 0; c.sample(); c.sample();
    y = 1; c.sample();
    $display("before %0.2f", c.get_inst_coverage());
    c.option.at_least = 2;
    $display("after at_least=2: %0.2f", c.get_inst_coverage());
    $display("at_least now %0d", c.option.at_least);
  end
endmodule
"#,
    );
    assert_line(&l, "goal 90 at_least 1 weight 1 abm 64 per_instance 0");
    assert_line(&l, "d goal 100");
    assert_line(&l, "before 50.00");
    assert_line(&l, "after at_least=2: 50.00");
    assert_line(&l, "at_least now 2");
}

/// An option the body sets from a constructor argument reads back as its
/// value (it used to read `x`), and the coverage still uses it. Two
/// commercial simulators print `cov 5000 0` and `opt 1 2`.
#[test]
fn option_from_a_constructor_argument_reads_back_its_value() {
    let l = lines(
        r#"
module tb;
  bit [1:0] v;
  function automatic int p(real r); return $rtoi(r * 100.0 + 0.5); endfunction
  covergroup cg (int al);
    option.per_instance = 1;
    option.at_least = al;
    cp: coverpoint v { bins b0 = {0}; bins b1 = {1}; }
  endgroup
  cg c1 = new(1);
  cg c2 = new(2);
  initial begin
    v = 0; c1.sample(); c2.sample();
    $display("cov %0d %0d", p(c1.get_inst_coverage()), p(c2.get_inst_coverage()));
    $display("opt %0d %0d", c1.option.at_least, c2.option.at_least);
  end
endmodule
"#,
    );
    assert_line(&l, "cov 5000 0");
    assert_line(&l, "opt 1 2");
}

/// Other integer options and an expression of an argument.
#[test]
fn options_from_argument_expressions_read_back() {
    let l = lines(
        r#"
module tb;
  bit [1:0] v;
  covergroup cg (int al, int w, int g);
    option.at_least = al + 1;
    option.weight = w;
    option.goal = g;
    cp: coverpoint v;
  endgroup
  cg c1 = new(1, 5, 90);
  cg c2 = new(2, 7, 80);
  initial begin
    $display("arg %0d %0d %0d %0d", c1.option.at_least, c2.option.at_least,
             c1.option.weight, c2.option.goal);
  end
endmodule
"#,
    );
    assert_line(&l, "arg 2 3 5 80");
}

/// Guard: in a covergroup with constructor arguments, a literal option and
/// one set from a module parameter read back as before, and a value written
/// at run time wins over the argument.
#[test]
fn literal_parameter_and_written_options_beside_arguments_are_unchanged() {
    let l = lines(
        r#"
module tb;
  parameter int W = 3;
  bit [1:0] v;
  covergroup ca (int al);
    option.at_least = al;
    option.goal = 90;
    option.weight = W;
    cp: coverpoint v;
  endgroup
  ca a = new(4);
  initial begin
    $display("lit %0d par %0d", a.option.goal, a.option.weight);
    a.option.at_least = 9;
    $display("set %0d", a.option.at_least);
  end
endmodule
"#,
    );
    assert_line(&l, "lit 90 par 3");
    assert_line(&l, "set 9");
}

/// §19.7: string-valued options retain the constructor argument's type.
#[test]
fn string_option_from_constructor_preserves_its_full_value() {
    let output = lines(
        r#"
module top;
  bit b;
  covergroup cg(string label_text);
    option.name = label_text;
    option.comment = {label_text, " comment"};
    coverpoint b;
  endgroup
  cg c = new("long option name");
  initial $display("NAME=%s COMMENT=%s", c.option.name, c.option.comment);
endmodule
"#,
    );
    assert_line(
        &output,
        "NAME=long option name COMMENT=long option name comment",
    );
}
