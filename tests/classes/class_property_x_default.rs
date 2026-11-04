//! Declared element types determine aggregate class-property defaults.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

fn check(source: &str, expected: &[&str]) {
    if !super::compiled_method_test_env::policies(&[("0", "1000"), ("1", "0"), ("1", "1000")]) {
        return;
    }
    let sim = xezim::simulate(source, 100).expect("simulation failed");
    let output: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .collect();
    for line in expected {
        assert!(output.contains(line), "missing {line:?}: {output:?}");
    }
}

#[test]
fn fixed_arrays_and_named_integral_types() {
    check(
        r#"
typedef logic [3:0] word_t;
typedef enum logic [1:0] {FIRST, SECOND} state_t;
typedef struct packed {logic [3:0] data; logic flag;} packed_t;
typedef struct packed {bit [3:0] data; bit flag;} binary_t;
typedef union packed {logic [3:0] left; logic [3:0] right;} overlay_t;
class sample;
  word_t scalar, words[2], cube[2][2][2];
  state_t state, states[2];
  packed_t packet, packets[2]; binary_t binary, binaries[2];
  overlay_t overlay;
  integer counters[2]; time stamps[2]; bit [3:0] flags[2]; int ids[2];
endclass
module tb;
  sample item;
  initial begin
    item = new;
    $display("VALUES|%b %b %b %b %b %b %b %b %b %b",
      item.scalar, item.words[1], item.cube[1][0][1], item.state,
      item.states[0], item.packet, item.packets[1], item.binary,
      item.binaries[1], item.overlay);
    $display("CHECK|%0d %0d %0d %0d", item.counters[0] === 'x,
      item.stamps[1] === 'x, item.flags[1] === 0, item.ids[0] === 0);
    $display("ARRAY|%p %p", item.words, item.states);
  end
endmodule
"#,
        &[
            "VALUES|xxxx xxxx xxxx xx xx xxxxx xxxxx 00000 00000 xxxx",
            "CHECK|1 1 1 1",
            "ARRAY|'{'hX, 'hX} '{x, x}",
        ],
    );
}

#[test]
fn unpacked_structs_and_nested_member_arrays() {
    check(
        r#"
typedef struct {logic [3:0] data; bit [3:0] flags; integer count; int id;} leaf_t;
typedef struct {leaf_t inner; logic [3:0] words[2]; bit [3:0] masks[2];} record_t;
class sample;
  leaf_t leaf, leaves[2]; record_t record;
  function void report();
    $display("METHOD|%b %b %b %b", leaf.data, leaves[1].data,
      record.words[1], record.masks[0]);
  endfunction
endclass
module tb;
  sample item;
  initial begin
    item = new;
    item.report();
    $display("STRUCT|%b %b %0d %0d %b", item.record.inner.data,
      item.record.inner.flags, item.leaf.count === 'x,
      item.leaf.id === 0, item.leaves[0].data);
    $display("SHAPE|%p", item.leaves);
  end
endmodule
"#,
        &[
            "METHOD|xxxx xxxx xxxx 0000",
            "STRUCT|xxxx 0000 1 1 xxxx",
            "SHAPE|'{'{data:'hX, flags:'h0, count:x, id:0}, '{data:'hX, flags:'h0, count:x, id:0}}",
        ],
    );
}

#[test]
fn dynamic_array_new_resize_and_method_paths() {
    check(
        r#"
typedef struct {logic [3:0] data; bit [3:0] flags;} record_t;
class sample;
  logic [3:0] words[]; integer counts[]; bit [3:0] flags[];
  record_t records[];
  function void allocate();
    words = new[2]; counts = new[2]; flags = new[2]; records = new[2];
  endfunction
endclass
module tb;
  sample item;
  initial begin
    item = new;
    item.allocate();
    item.words[0] = 4'h3;
    item.words = new[4](item.words);
    item.records[0].flags = 1;
    $display("DYNAMIC|%b %b %b %b %0d %b", item.words[0],
      item.words[1], item.words[2], item.words[3],
      item.counts[0] === 'x, item.flags[1]);
    $display("RECORDS|%p", item.records);
    item.words = new[1];
    $display("REPLACE|%0d %b", item.words.size(), item.words[0]);
  end
endmodule
"#,
        &[
            "DYNAMIC|0011 xxxx xxxx xxxx 1 0000",
            "RECORDS|'{'{data:'hX, flags:'h1}, '{data:'hX, flags:'h0}}",
            "REPLACE|1 xxxx",
        ],
    );
}

#[test]
fn inheritance_package_scope_and_shallow_copy() {
    check(
        r#"
package types_pkg;
  class parent; logic [3:0] inherited[2]; endclass
  class sample extends parent; logic [3:0] words[2]; endclass
endpackage
module tb;
  types_pkg::sample first, second;
  initial begin
    first = new;
    first.words[0] = 4'h3;
    second = new first;
    $display("COPY|%b %b %b", second.words[0], second.words[1], second.inherited[0]);
    second.words[0] = 4'h5;
    $display("INDEPENDENT|%b %b", first.words[0], second.words[0]);
  end
endmodule
"#,
        &["COPY|0011 xxxx xxxx", "INDEPENDENT|0011 0101"],
    );
}

#[test]
fn parameterized_element_type_and_width() {
    check(
        r#"
typedef logic [7:0] octet_t;
class sample #(type T = octet_t, int N = 2);
  T scalar, words[N], dynamic_words[];
  logic [N*2-1:0] sized[2];
endclass
module tb;
  sample unknowns;
  sample #(int, 3) numbers;
  sample #(integer, 3) wide_unknowns;
  initial begin
    unknowns = new; numbers = new; wide_unknowns = new;
    unknowns.dynamic_words = new[1]; numbers.dynamic_words = new[1];
    $display("TYPES|%b %b %b %0d %0d %0d", unknowns.scalar,
      unknowns.words[1], unknowns.dynamic_words[0], numbers.scalar,
      numbers.words[2], numbers.dynamic_words[0]);
    $display("WIDTH|%b %b", unknowns.sized[1], numbers.sized[0]);
    $display("EQUAL_WIDTH|%0d %0d", wide_unknowns.scalar === 'x, wide_unknowns.words[1] === 'x);
  end
endmodule
"#,
        &[
            "TYPES|xxxxxxxx xxxxxxxx xxxxxxxx 0 0 0",
            "WIDTH|xxxx xxxxxx",
            "EQUAL_WIDTH|1 1",
        ],
    );
}

#[test]
fn explicit_initializers_and_member_defaults() {
    check(
        r#"
typedef struct {logic [3:0] data = 4'h5; logic [3:0] other; bit [3:0] flags;} record_t;
class sample;
  record_t record, records[2], initialized = '{data:4'h1, other:4'h2, flags:4'h3};
  logic [3:0] words[2] = '{4'h7, 4'h8};
  logic [3:0] dynamic_words[] = new[2];
endclass
module tb;
  sample item;
  initial begin
    item = new;
    $display("DEFAULTS|%b %b %b %b", item.record.data, item.record.other,
      item.records[1].data, item.records[1].other);
    $display("INITIALIZED|%p %p %b", item.initialized, item.words, item.dynamic_words[1]);
  end
endmodule
"#,
        &[
            "DEFAULTS|0101 xxxx 0101 xxxx",
            "INITIALIZED|'{data:'h1, other:'h2, flags:'h3} '{'h7, 'h8} xxxx",
        ],
    );
}

#[test]
fn randomization_preserves_nonrandom_defaults() {
    check(
        r#"
class sample;
  rand logic [3:0] random_words[2]; logic [3:0] words[2];
  rand logic [3:0] value; logic [3:0] held;
endclass
module tb;
  sample item;
  initial begin
    item = new;
    $display("BEFORE|%0d %0d %0d", item.random_words[0] === 'x,
      item.words[0] === 'x, item.value === 'x);
    if (!item.randomize()) $fatal(1, "randomization failed");
    $display("AFTER|%0d %0d %0d %0d", $isunknown(item.random_words[0]),
      $isunknown(item.value), item.words[1] === 'x, item.held === 'x);
  end
endmodule
"#,
        &["BEFORE|1 1 1", "AFTER|0 0 1 1"],
    );
}

#[test]
fn static_fixed_array_element_width_and_defaults() {
    check(
        r#"
class sample;
  static logic [3:0] words[2]; static integer counts[2]; static bit [3:0] flags[2];
endclass
module tb;
  initial begin
    $display("STATIC|%b %0d %b", sample::words[1], sample::counts[0] === 'x, sample::flags[1]);
    $display("STATIC_SHAPE|%p", sample::words);
  end
endmodule
"#,
        &["STATIC|xxxx 1 0000", "STATIC_SHAPE|'{'hX, 'hX}"],
    );
}

#[test]
fn associative_missing_struct_member_defaults_do_not_insert() {
    check(
        r#"
typedef struct {logic [3:0] data; bit [3:0] flags; integer count;} record_t;
class sample; record_t records[int]; logic [3:0] words[int]; endclass
module tb;
  sample item;
  initial begin
    item = new;
    $display("MISSING|%b %b %0d %b %0d", item.records[7].data,
      item.records[7].flags, item.records[7].count === 'x,
      item.words[7], item.records.num());
  end
endmodule
"#,
        &["MISSING|xxxx 0000 1 xxxx 0"],
    );
}

#[test]
fn constant_named_property_starts_unknown_before_constructor_assignment() {
    check(
        r#"
typedef logic [3:0] word_t;
class sample;
  const word_t value;
  function new();
    $display("CONST_BEFORE|%b", value);
    value = 4'h7;
  endfunction
endclass
module tb;
  sample item;
  initial begin item = new; $display("CONST_AFTER|%b", item.value); end
endmodule
"#,
        &["CONST_BEFORE|xxxx", "CONST_AFTER|0111"],
    );
}

#[test]
fn struct_type_parameter_defaults_are_instance_specific() {
    check(
        r#"
typedef struct {logic [3:0] data; bit [3:0] flags;} record_t;
class sample #(type T = int);
  T value, records[2], dynamic_records[];
endclass
module tb;
  sample #(int) numbers;
  sample #(record_t) item;
  initial begin
    numbers = new; item = new;
    item.dynamic_records = new[2];
    $display("BOUND_STRUCT|%b %b %b %b %b %0d", item.value.data,
      item.value.flags, item.records[1].data, item.records[1].flags,
      item.dynamic_records[1].data, numbers.value);
    $display("BOUND_SHAPE|%p %p", item.records, item.dynamic_records);
  end
endmodule
"#,
        &[
            "BOUND_STRUCT|xxxx 0000 xxxx 0000 xxxx 0",
            "BOUND_SHAPE|'{'{data:'hX, flags:'h0}, '{data:'hX, flags:'h0}} '{'{data:'hX, flags:'h0}, '{data:'hX, flags:'h0}}",
        ],
    );
}
