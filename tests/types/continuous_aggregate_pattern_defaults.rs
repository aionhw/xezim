//! IEEE 1800-2023 §10.9.1 and §10.9.2 apply a default whole when the
//! member or element matches its self-determined type (§6.22.1).

fn check(src: &str, expected: &[&str]) {
    let sim = xezim::simulate(src, 10_000).expect("simulate assignment patterns");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("VALUE="))
        .collect();
    assert_eq!(lines, expected);
}

#[test]
fn matching_defaults_take_the_whole_member_or_element() {
    check(
        r#"
`timescale 1ns/1ps
module top;
  typedef struct packed {logic [3:0] hi, lo;} P;
  typedef P Q;
  typedef Q R;
  typedef struct {P pk;} S;
  typedef struct {S inner;} Wrapper;
  typedef logic [1:0][3:0] V;
  typedef struct {V elem;} VHolder;
  typedef struct {logic [3:0] elem[1:0];} ArrayHolder;
  typedef struct {P elem[1:0];} Pair;

  R source;
  V vec;
  S inner_value;
  logic [3:0] elems[1:0];
  S plain, casted, typed_pattern;
  Wrapper nested, whole_inner;
  VHolder packed_array;
  ArrayHolder fixed_array;
  Pair repeated;

  assign plain = '{default:source};
  assign casted = '{default:P'(8'h34)};
  assign typed_pattern = '{default:P'{4'h5, 4'h6}};
  assign nested = '{default:source};
  assign whole_inner = '{default:inner_value};
  assign packed_array = '{default:vec};
  assign fixed_array = '{default:elems};
  assign repeated = '{default:source};

  initial begin
    #1;
    source = 8'h12;
    vec = 8'h87;
    inner_value = '{pk:8'h9a};
    elems[1] = 4'hb;
    elems[0] = 4'hc;
    #1;
    $display("VALUE=%h %h %h %h %h %h %h%h %h%h",
             plain.pk, casted.pk, typed_pattern.pk, nested.inner.pk,
             whole_inner.inner.pk, packed_array.elem, fixed_array.elem[1],
             fixed_array.elem[0], repeated.elem[1], repeated.elem[0]);
    source = 8'hd3;
    #1;
    $display("VALUE=%h %h %h%h", plain.pk, nested.inner.pk,
             repeated.elem[1], repeated.elem[0]);
    $finish;
  end
endmodule
"#,
        &["VALUE=12 34 56 12 9a 87 bc 1212", "VALUE=d3 d3 d3d3"],
    );
}

#[test]
fn matching_package_type_in_net_and_port_patterns() {
    // §10.3.1 and §23.3.3.2 supply the continuous assignment contexts.
    check(
        r#"
`timescale 1ns/1ps
package data_types;
  typedef struct packed {logic [3:0] hi, lo;} P;
  typedef P Q;
endpackage

package holders;
  typedef struct {data_types::P pk;} S;
endpackage

module sink(input var holders::S s);
endmodule

module top;
  data_types::Q source;
  nettype holders::S N;
  N net_value = '{default:source};
  sink u(.s('{default:data_types::P'(8'h34)}));

  initial begin
    #1; source = 8'h12;
    #1; $display("VALUE=%h %h", net_value.pk, u.s.pk);
    $finish;
  end
endmodule
"#,
        &["VALUE=12 34"],
    );
}

#[test]
fn matching_package_spellings_in_continuous_patterns() {
    // §6.22.1(h), §26.3: all spellings name the package's declaration.
    for member in ["P", "pk::P"] {
        for (import, p, s) in [
            ("", "pk::P", "pk::S"),
            ("import pk::*;", "P", "S"),
            ("import pk::P; import pk::S;", "P", "S"),
        ] {
            let source = format!(
                r#"
`timescale 1ns/1ps
package pk;
  typedef struct packed {{logic [3:0] hi, lo;}} P;
  typedef struct {{{member} pk;}} S;
endpackage
module sink(input var pk::S s);
endmodule
module dut;
  {import}
  {p} p;
  {s} a;
  assign a = '{{default:p}};
  nettype {s} N;
  N n = '{{default:p}};
  sink u(.s('{{default:p}}));
  initial begin
    #1; p = 8'h12;
    #1; $display("VALUE=%h %h %h", a.pk, n.pk, u.s.pk); $finish;
  end
endmodule
"#
            );
            check(&source, &["VALUE=12 12 12"]);
            check(
                &(source
                    + r#"
module top;
  nettype pk::S N;
  dut d();
endmodule
"#),
                &["VALUE=12 12 12"],
            );
        }
    }
}

#[test]
fn defaults_preserve_descent_and_explicit_keys() {
    check(
        r#"
`timescale 1ns/1ps
typedef struct packed {logic [3:0] hi, lo;} Hidden;
typedef struct {Hidden pk;} UnitHolder;
typedef struct packed {logic [3:0] hi, lo;} LaterHidden;

module top;
  // §6.22.1(d): this declaration has a different identity from $unit::Hidden.
  typedef struct packed {logic [3:0] hi, lo;} Hidden;
  Hidden local_value;
  UnitHolder unit_holder;
  typedef struct {LaterHidden pk;} EarlyHolder;
  typedef struct packed {logic [3:0] hi, lo;} LaterHidden;
  LaterHidden later_value;
  EarlyHolder early_holder;
  typedef logic [3:0] NIB;
  typedef struct packed {NIB hi, lo;} P;
  typedef struct packed {NIB hi, lo;} Q;
  typedef struct {P pk;} S;
  typedef logic [1:0][3:0] V;
  typedef struct {V elem;} VHolder;

  P source;
  Q other;
  logic [0:1][3:0] reversed_vec;
  bit [1:0][3:0] two_state_vec;
  S scalar, same_shape, named, keyed, nibble_key, procedural;
  VHolder by_range, by_state;

  assign unit_holder = '{default:local_value};
  assign early_holder = '{default:later_value};
  assign scalar = '{default:1};
  assign same_shape = '{default:other};
  assign named = '{pk:8'h34, default:source};
  assign keyed = '{P:8'h56, default:source};
  assign nibble_key = '{NIB:9, default:1};
  assign by_range = '{default:reversed_vec};
  assign by_state = '{default:two_state_vec};

  initial begin
    #1;
    source = 8'h12;
    other = 8'h12;
    local_value = 8'h12;
    later_value = 8'h12;
    reversed_vec = 8'h12;
    two_state_vec = 8'h12;
    #1; procedural = '{default:source};
    #1;
    $display("VALUE=%h %h %h %h %h %h %h %h",
             scalar.pk, same_shape.pk, named.pk, keyed.pk, nibble_key.pk,
             by_range.elem, by_state.elem, procedural.pk);
    $display("VALUE=unit hidden %h %h", unit_holder.pk, early_holder.pk);
    $finish;
  end
endmodule
"#,
        &["VALUE=11 22 34 56 99 22 22 12", "VALUE=unit hidden 22 22"],
    );
}

/// §10.3.3: a delayed continuous assignment schedules the whole new value,
/// and a change before delivery replaces the pending one. The matching
/// default still fills the packed member whole (§10.9.2), with one leaf and
/// with a sibling leaf.
#[test]
fn matching_defaults_under_an_inertial_delay() {
    check(
        r#"
`timescale 1ns/1ps
module top;
  typedef struct packed {logic [3:0] hi, lo;} P;
  typedef struct {logic [3:0] tag; P pk;} S;
  typedef struct {P pk;} S1;
  P p;
  logic [3:0] t;
  S sd;
  S1 d1;
  assign #2 sd = '{tag:t, default:p};
  assign #2 d1 = '{default:p};
  initial begin
    p = 8'h12;
    t = 4'h7;
    #3.5 $display("VALUE=%h %h %h", sd.tag, sd.pk, d1.pk);
    p = 8'hab;
    #1 t = 4'h9;
    #1.5 $display("VALUE=%h %h %h", sd.tag, sd.pk, d1.pk);
    #1 $display("VALUE=%h %h %h", sd.tag, sd.pk, d1.pk);
    $finish;
  end
endmodule
"#,
        &["VALUE=7 12 12", "VALUE=7 12 ab", "VALUE=9 ab ab"],
    );
}
