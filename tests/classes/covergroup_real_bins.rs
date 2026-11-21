//! IEEE 1800-2023 real coverpoint bins and integral coverage guards.

use xezim::simulate;

fn check_coverage(source: &str, expected: &[&str]) {
    let sim = simulate(source, 100_000).expect("simulate failed");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.trim())
        .filter(|line| line.starts_with("coverage "))
        .collect();
    assert_eq!(sim.error_count, 0, "{lines:?}");
    assert_eq!(lines, expected);
}

/// §19.5.1 and §19.11.1: a real range includes both endpoints.
#[test]
fn real_range() {
    check_coverage(
        r#"
`timescale 1ns/1ps
module top;
  real x;
  covergroup cg;
    option.per_instance = 1;
    cp: coverpoint x {
      bins in_range = {[1.0:3.0]};
      bins other = {[5.0:6.0]};
    }
  endgroup
  cg c;
  initial begin
    c = new;
    x = 2.0;
    #1;
    c.sample();
    $display("coverage %0.1f", c.get_inst_coverage());
    x = 6.0;
    #1;
    c.sample();
    $display("coverage %0.1f", c.get_inst_coverage());
  end
endmodule
"#,
        &["coverage 50.0", "coverage 100.0"],
    );
}

/// §19.5.1 and §19.5.7: a real singleton matches the same numeric value.
#[test]
fn real_singleton() {
    check_coverage(
        r#"
`timescale 1ns/1ps
module top;
  real x;
  covergroup cg;
    option.per_instance = 1;
    cp: coverpoint x { bins exact = {2.0}; }
  endgroup
  cg c;
  initial begin
    c = new;
    x = 2.25;
    #1;
    c.sample();
    $display("coverage %0.1f", c.get_inst_coverage());
    x = 2.0;
    #1;
    c.sample();
    $display("coverage %0.1f", c.get_inst_coverage());
  end
endmodule
"#,
        &["coverage 0.0", "coverage 100.0"],
    );
}

/// §6.20.7 and §19.5.1: $ leaves one end of a real range unbounded.
#[test]
fn real_unbounded_ranges() {
    check_coverage(
        r#"
`timescale 1ns/1ps
module top;
  real x;
  covergroup cg;
    option.per_instance = 1;
    cp: coverpoint x {
      bins low = {[$:1.5]};
      bins high = {[5.5:$]};
    }
  endgroup
  cg c;
  initial begin
    c = new;
    x = 1.5;
    #1;
    c.sample();
    $display("coverage %0.1f", c.get_inst_coverage());
    x = 8.0;
    #1;
    c.sample();
    $display("coverage %0.1f", c.get_inst_coverage());
  end
endmodule
"#,
        &["coverage 50.0", "coverage 100.0"],
    );
}

/// §19.5.5 and §19.11.1: a fully ignored bin is excluded from the denominator.
#[test]
fn real_ignore_bins() {
    check_coverage(
        r#"
`timescale 1ns/1ps
module top;
  real x;
  covergroup cg;
    option.per_instance = 1;
    cp: coverpoint x {
      bins excluded = {[1.0:2.0]};
      bins live = {[3.0:4.0]};
      ignore_bins ignored = {[1.0:2.0]};
    }
  endgroup
  cg c;
  initial begin
    int covered, total;
    real percent;
    c = new;
    x = 1.5;
    #1;
    c.sample();
    percent = c.cp.get_inst_coverage(covered, total);
    $display("coverage %0.1f %0d/%0d", percent, covered, total);
    x = 3.5;
    #1;
    c.sample();
    percent = c.cp.get_inst_coverage(covered, total);
    $display("coverage %0.1f %0d/%0d", percent, covered, total);
  end
endmodule
"#,
        &["coverage 0.0 0/1", "coverage 100.0 1/1"],
    );
}

/// §19.5.6: sampling an illegal value issues a run-time error.
#[test]
fn real_illegal_bins() {
    let sim = simulate(
        r#"
`timescale 1ns/1ps
module top;
  real x;
  covergroup cg;
    cp: coverpoint x {
      bins live = {[3.0:4.0]};
      illegal_bins forbidden = {[1.0:2.0]};
    }
  endgroup
  cg c;
  initial begin
    c = new;
    x = 1.5;
    #1;
    c.sample();
  end
endmodule
"#,
        100_000,
    )
    .expect("simulate failed");
    assert_eq!(sim.error_count, 1, "sampling an illegal real bin");
}

/// §19.5 and §19.11.1: a real point with only a default bin has 0/0 counted bins.
#[test]
fn real_default_only() {
    check_coverage(
        r#"
`timescale 1ns/1ps
module top;
  real x;
  covergroup cg;
    option.per_instance = 1;
    cp: coverpoint x { bins other = default; }
  endgroup
  cg c;
  initial begin
    int covered, total;
    real percent;
    c = new;
    x = 2.0;
    #1;
    c.sample();
    percent = c.cp.get_inst_coverage(covered, total);
    $display("coverage %0.4f %0d/%0d", percent, covered, total);
  end
endmodule
"#,
        &["coverage 0.0000 0/0"],
    );
}

/// §19.4 and §19.5.1: an embedded covergroup can sample a real class property.
#[test]
fn real_class_property() {
    check_coverage(
        r#"
`timescale 1ns/1ps
module top;
  class C;
    real x;
    covergroup cg;
      option.per_instance = 1;
      cp: coverpoint x {
        bins low = {[1.0:3.0]};
        bins high = {[5.0:6.0]};
      }
    endgroup
    function new();
      cg = new;
    endfunction
  endclass
  initial begin
    C c;
    c = new;
    c.x = 2.5;
    #1;
    c.cg.sample();
    $display("coverage %0.1f", c.cg.get_inst_coverage());
  end
endmodule
"#,
        &["coverage 50.0"],
    );
}

/// §19.5.1 and §19.8.1: the real sample formal supplies the value of the point.
#[test]
fn real_sample_formal() {
    check_coverage(
        r#"
`timescale 1ns/1ps
module top;
  covergroup cg with function sample(input real x);
    option.per_instance = 1;
    cp: coverpoint x {
      bins low = {[1.0:3.0]};
      bins high = {[5.0:6.0]};
    }
  endgroup
  cg c;
  initial begin
    c = new;
    #1;
    c.sample(2.5);
    $display("coverage %0.1f", c.get_inst_coverage());
  end
endmodule
"#,
        &["coverage 50.0"],
    );
}

/// §6.12, §19.5.1 and §19.5.7: realtime and shortreal both use numeric bins.
#[test]
fn realtime_and_shortreal() {
    check_coverage(
        r#"
`timescale 1ns/1ps
module top;
  realtime rt;
  shortreal sr;
  covergroup cg;
    option.per_instance = 1;
    p_rt: coverpoint rt { bins exact = {2.5}; }
    p_sr: coverpoint sr { bins exact = {0.5}; }
  endgroup
  cg c;
  initial begin
    c = new;
    rt = 2.5;
    sr = 0.5;
    #1;
    c.sample();
    $display("coverage realtime %0.1f", c.p_rt.get_inst_coverage());
    $display("coverage shortreal %0.1f", c.p_sr.get_inst_coverage());
  end
endmodule
"#,
        &["coverage realtime 100.0", "coverage shortreal 100.0"],
    );
}

/// §19.5–§19.7: integral bounds, wildcard, transition, automatic, default,
/// and cross bins retain their own matching and counting rules.
#[test]
fn integral_bin_forms() {
    check_coverage(
        r#"
`timescale 1ns/1ps
module top;
  int x;
  bit [2:0] bits;
  bit a, b;
  covergroup cg;
    option.per_instance = 1;
    bounds: coverpoint x {
      bins low = {[1.0:3.0]};
      bins high = {[5.0:6.0]};
    }
    mask: coverpoint bits { wildcard bins hi = {3'b1??}; }
    steps: coverpoint bits { bins rise = (1 => 5); }
    automatic_bins: coverpoint bits;
    capped: coverpoint bits { option.auto_bin_max = 4; }
    fallback: coverpoint bits {
      bins first = {5};
      bins second = {6};
      bins other = default;
    }
    pa: coverpoint a;
    pb: coverpoint b;
    pair: cross pa, pb;
  endgroup
  cg c;
  task automatic report();
    $display("coverage bounds %0.1f", c.bounds.get_inst_coverage());
    $display("coverage wildcard %0.1f", c.mask.get_inst_coverage());
    $display("coverage transition %0.1f", c.steps.get_inst_coverage());
    $display("coverage auto %0.1f", c.automatic_bins.get_inst_coverage());
    $display("coverage capped %0.1f", c.capped.get_inst_coverage());
    $display("coverage default %0.1f", c.fallback.get_inst_coverage());
    $display("coverage cross %0.1f", c.pair.get_inst_coverage());
  endtask
  initial begin
    c = new;
    x = 2;
    bits = 1;
    a = 0;
    b = 1;
    #1;
    c.sample();
    report();
    bits = 5;
    a = 1;
    #1;
    c.sample();
    report();
  end
endmodule
"#,
        &[
            "coverage bounds 50.0",
            "coverage wildcard 0.0",
            "coverage transition 0.0",
            "coverage auto 12.5",
            "coverage capped 25.0",
            "coverage default 0.0",
            "coverage cross 25.0",
            "coverage bounds 50.0",
            "coverage wildcard 100.0",
            "coverage transition 100.0",
            "coverage auto 25.0",
            "coverage capped 50.0",
            "coverage default 50.0",
            "coverage cross 50.0",
        ],
    );
}

/// §6.18, §19.5.3, §19.8.1 and §23.9: the sample formal uses the local integral typedef.
#[test]
fn integral_sample_typedef_uses_its_own_scope() {
    check_coverage(
        r#"
`timescale 1ns/1ps
module real_scope;
  typedef real T;
  T x;
endmodule

module integral_scope;
  typedef int T;
  covergroup cg with function sample(T v);
    option.per_instance = 1;
    cp: coverpoint v;
  endgroup
  cg c;
  initial begin
    int covered, total;
    real percent;
    c = new;
    #1;
    c.sample(2);
    percent = c.cp.get_inst_coverage(covered, total);
    $display("coverage %0d/%0d", covered, total);
  end
endmodule

module top;
  real_scope r();
  integral_scope i();
endmodule
"#,
        &["coverage 1/64"],
    );
}

/// §6.12.1, §8.25, §19.5.3 and §19.8.1: an input of type T specializes to int.
#[test]
fn integral_sample_type_parameter_with_real_actual() {
    check_coverage(
        r#"
`timescale 1ns/1ps
module top;
  class C #(type T = int);
    covergroup cg with function sample(T v);
      option.per_instance = 1;
      cp: coverpoint v;
    endgroup
    function new();
      cg = new;
    endfunction
  endclass
  initial begin
    C #(int) c;
    int covered, total;
    real percent;
    c = new;
    #1;
    c.cg.sample(2.5);
    percent = c.cg.cp.get_inst_coverage(covered, total);
    $display("coverage %0d/%0d", covered, total);
  end
endmodule
"#,
        &["coverage 1/64"],
    );
}

/// §19.5.1 and §19.5.5: overlapping ignored ranges leave a fractional
/// interval live. Signed zeroes select the same singleton.
#[test]
fn real_fractional_exclusions_and_signed_zero() {
    check_coverage(
        r#"
module top;
  covergroup cg with function sample(real v);
    cp: coverpoint v {
      bins zero = {0.0};
      bins partial = {[-2.75:-1.25]};
      ignore_bins ignored = {[-2.5:-2.0], [-2.25:-1.25]};
    }
  endgroup
  cg c;
  initial begin
    int covered, total;
    real pct;
    c = new;
    c.sample(-0.0);
    c.sample(-2.25);
    pct = c.cp.get_inst_coverage(covered, total);
    $display("coverage %0d/%0d", covered, total);
    c.sample(-2.625);
    pct = c.cp.get_inst_coverage(covered, total);
    $display("coverage %0d/%0d", covered, total);
  end
endmodule
"#,
        &["coverage 1/2", "coverage 2/2"],
    );
}
