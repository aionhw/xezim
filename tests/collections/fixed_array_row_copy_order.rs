//! IEEE 1800-2023 §6.22.2, §7.4.4 and §7.6: fixed unpacked arrays
//! copy by left-to-right position, including selected rows and planes.
//! Equivalent aggregates compare corresponding elements (§11.2.2, §11.4.5).

use xezim::simulate;

fn check_output(source: &str, expected: &[&str]) {
    let sim = simulate(source, 10_000).expect("simulate");
    let got: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .collect();
    assert_eq!(got, expected);
}

/// §7.4.4, §7.6: the selected row starts at index 1.
#[test]
fn descending_row_copies_to_ascending_array() {
    check_output(
        r#"
`timescale 1ns/1ps
module top;
  int src[2][1:0];
  int dst[4:5];
  initial begin
    src[1][1] = 41;
    src[1][0] = 42;
    #1 dst = src[1];
    #1 $display("%0d %0d", dst[4], dst[5]);
    $finish;
  end
endmodule
"#,
        &["41 42"],
    );
}

/// §7.6: both sides use declared order, including negative bounds.
#[test]
fn copy_from_a_row_follows_declared_bounds() {
    check_output(
        r#"
`timescale 1ns/1ps
module top;
  int asc_src[2][0:1], desc_src[2][1:0];
  int asc_dst[4:5], desc_dst[5:4];
  int neg_src[2][-2:-4], neg_dst[-8:-6];
  initial begin
    asc_src[1][0] = 41;
    asc_src[1][1] = 42;
    desc_src[1][1] = 41;
    desc_src[1][0] = 42;
    neg_src[1][-2] = 41;
    neg_src[1][-3] = 43;
    neg_src[1][-4] = 45;
    #1;
    asc_dst = asc_src[1];
    desc_dst = asc_src[1];
    $display("asc-to-asc %0d %0d", asc_dst[4], asc_dst[5]);
    $display("asc-to-desc %0d %0d", desc_dst[5], desc_dst[4]);
    desc_dst = desc_src[1];
    $display("desc-to-desc %0d %0d", desc_dst[5], desc_dst[4]);
    neg_dst = neg_src[1];
    $display("negative %0d %0d %0d", neg_dst[-8], neg_dst[-7], neg_dst[-6]);
    $finish;
  end
endmodule
"#,
        &[
            "asc-to-asc 41 42",
            "asc-to-desc 41 42",
            "desc-to-desc 41 42",
            "negative 41 43 45",
        ],
    );
}

/// §7.4.4, §7.6: the first source element goes to the row's left bound.
#[test]
fn copy_into_a_row_follows_declared_bounds() {
    check_output(
        r#"
`timescale 1ns/1ps
module top;
  int one[4:5];
  int dst[2][1:0];
  initial begin
    one[4] = 41;
    one[5] = 42;
    #1 dst[1] = one;
    #1 $display("destination %0d %0d", dst[1][1], dst[1][0]);
    $finish;
  end
endmodule
"#,
        &["destination 41 42"],
    );
}

/// §7.6: the fixed rows have equal lengths and different bounds.
#[test]
fn opposite_direction_rows_copy_by_position() {
    check_output(
        r#"
`timescale 1ns/1ps
module top;
  int src[2][1:0], dst[2][4:5];
  initial begin
    src[1][1] = 41;
    src[1][0] = 42;
    #1 dst[0] = src[1];
    #1 $display("row %0d %0d", dst[0][4], dst[0][5]);
    $finish;
  end
endmodule
"#,
        &["row 41 42"],
    );
}

/// §6.22.2, §7.6: each copied row follows the inner dimension's direction.
#[test]
fn whole_array_copy_follows_inner_direction() {
    check_output(
        r#"
`timescale 1ns/1ps
module top;
  int src[2][1:0], dst[4:5][0:1];
  initial begin
    src[0][1] = 41;
    src[0][0] = 42;
    src[1][1] = 43;
    src[1][0] = 44;
    #1 dst = src;
    #1 $display("whole %0d %0d %0d %0d", dst[4][0], dst[4][1], dst[5][0], dst[5][1]);
    $finish;
  end
endmodule
"#,
        &["whole 41 42 43 44"],
    );
}

/// §7.4.4, §7.6: equal numeric bounds still allow opposite directions.
#[test]
fn selected_planes_copy_by_position() {
    check_output(
        r#"
`timescale 1ns/1ps
module top;
  int src[2][0:1][0:1], dst[2][0:1][1:0];
  initial begin
    src[1][0][0] = 41;
    src[1][0][1] = 42;
    src[1][1][0] = 43;
    src[1][1][1] = 44;
    #1 dst[0] = src[1];
    #1 $display("plane %0d %0d %0d %0d",
                dst[0][0][1], dst[0][0][0], dst[0][1][1], dst[0][1][0]);
    $finish;
  end
endmodule
"#,
        &["plane 41 42 43 44"],
    );
}

/// §11.2.2, §11.4.5: compare operands populated independently in declared order.
#[test]
fn independently_populated_row_compares_by_position() {
    check_output(
        r#"
`timescale 1ns/1ps
module top;
  int src[2][1:0], built[4:5];
  initial begin
    src[1][1] = 41;
    src[1][0] = 42;
    built[4] = 41;
    built[5] = 42;
    #1 $display("independent %b %b %b %b",
                built == src[1], built != src[1], built === src[1], built !== src[1]);
    $finish;
  end
endmodule
"#,
        &["independent 1 0 1 0"],
    );
}

/// §6.21, §7.6, §7.7: rows of fixed locals and formals follow the same order.
#[test]
fn rows_of_locals_and_formals_copy_by_position() {
    check_output(
        r#"
`timescale 1ns/1ps
module top;
  int src[2][1:0], dst[4:5];

  function automatic void copy_formal(input int formal_src[2][1:0]);
    dst = formal_src[1];
    $display("formal-row %0d %0d", dst[4], dst[5]);
  endfunction

  function automatic void copy_automatic();
    int auto_src[2][1:0];
    auto_src[1][1] = 41;
    auto_src[1][0] = 42;
    dst = auto_src[1];
    $display("automatic-row %0d %0d", dst[4], dst[5]);
  endfunction

  initial begin : copy_block
    int block_src[2][1:0];
    block_src[1][1] = 41;
    block_src[1][0] = 42;
    src[1][1] = 41;
    src[1][0] = 42;
    #1;
    dst = block_src[1];
    $display("block-row %0d %0d", dst[4], dst[5]);
    copy_automatic();
    copy_formal(src);
    $finish;
  end
endmodule
"#,
        &["block-row 41 42", "automatic-row 41 42", "formal-row 41 42"],
    );
}

/// Guard (§7.6): copying through a 1-D temporary preserves the row.
#[test]
fn descending_row_round_trips_through_one_dimension() {
    check_output(
        r#"
`timescale 1ns/1ps
module top;
  int src[2][1:0], dst[2][1:0];
  int tmp[1:0];
  initial begin
    src[1][1] = 41;
    src[1][0] = 42;
    #1;
    tmp = src[1];
    dst[0] = tmp;
    #1 $display("round-trip %0d %0d", dst[0][1], dst[0][0]);
    $finish;
  end
endmodule
"#,
        &["round-trip 41 42"],
    );
}

/// Guard (§6.21, §7.6): the temporary preserves order in both directions.
#[test]
fn module_and_block_rows_round_trip_through_one_dimension() {
    check_output(
        r#"
`timescale 1ns/1ps
module top;
  int src[2][1:0], dst[2][1:0];
  int tmp[2];
  initial begin : relay
    int local_rows[2][1:0];
    src[1][1] = 41;
    src[1][0] = 42;
    local_rows[1][1] = 51;
    local_rows[1][0] = 52;
    #1;
    tmp = src[1];
    local_rows[0] = tmp;
    tmp = local_rows[1];
    dst[0] = tmp;
    #1;
    $display("to-local %0d %0d", local_rows[0][1], local_rows[0][0]);
    $display("from-local %0d %0d", dst[0][1], dst[0][0]);
    $finish;
  end
endmodule
"#,
        &["to-local 41 42", "from-local 51 52"],
    );
}

/// Guard (§7.6, §11.2.2, §11.4.5): all four operators agree on the copy.
#[test]
fn a_copied_row_compares_equal_to_its_source() {
    check_output(
        r#"
`timescale 1ns/1ps
module top;
  int src[2][1:0], dst[1:0];
  initial begin
    src[1][1] = 41;
    src[1][0] = 42;
    #1 dst = src[1];
    #1 $display("copied %b %b %b %b",
                dst == src[1], dst != src[1], dst === src[1], dst !== src[1]);
    $finish;
  end
endmodule
"#,
        &["copied 1 0 1 0"],
    );
}

/// Guard (§7.6, §11.2.2): whole rank-3 and rank-4 copies and a selected plane.
#[test]
fn higher_rank_copies_with_equal_numeric_bounds_compare_equal() {
    check_output(
        r#"
`timescale 1ns/1ps
module top;
  int s3[0:1][0:2][0:1], d3[1:0][0:2][0:1];
  int s4[0:1][0:2][0:1][0:1], d4[1:0][0:2][0:1][0:1];
  int planes[2][0:2][1:0];
  initial begin
    s3[0][0][0] = 11;
    s3[1][2][1] = 77;
    s4[0][0][0][0] = 11;
    s4[1][2][1][1] = 77;
    #1;
    d3 = s3;
    d4 = s4;
    planes[1] = s3[0];
    #1;
    $display("rank3 %b", d3 == s3);
    $display("rank4 %b", d4 == s4);
    $display("rank3-plane %b", planes[1] == s3[0]);
    $finish;
  end
endmodule
"#,
        &["rank3 1", "rank4 1", "rank3-plane 1"],
    );
}

/// Guard (§7.7, §11.2.2, §13.5.1): function copy-in and task copy-in and copy-out.
#[test]
fn copies_relayed_through_formals_compare_equal() {
    check_output(
        r#"
`timescale 1ns/1ps
module top;
  int src[2][1:0], from_function[2][0:1], from_task[2][0:1];

  function automatic void relay_function(input int value[2][1:0]);
    from_function = value;
  endfunction

  task automatic relay_task(input int value[2][1:0], output int result[2][0:1]);
    result = value;
  endtask

  initial begin
    src[0][1] = 31;
    src[0][0] = 32;
    src[1][1] = 41;
    src[1][0] = 42;
    #1;
    relay_function(src);
    relay_task(src, from_task);
    #1;
    $display("function %b", from_function == src);
    $display("task %b", from_task == src);
    $finish;
  end
endmodule
"#,
        &["function 1", "task 1"],
    );
}

/// Guard (§6.21, §7.6, §11.2.2): both block and automatic function locals.
#[test]
fn copies_relayed_through_locals_compare_equal() {
    check_output(
        r#"
`timescale 1ns/1ps
module top;
  int src[2][1:0], from_block[2][0:1], from_automatic[2][0:1];

  function automatic void relay();
    int local_copy[2][1:0];
    local_copy = src;
    from_automatic = local_copy;
  endfunction

  initial begin
    src[0][1] = 31;
    src[0][0] = 32;
    src[1][1] = 41;
    src[1][0] = 42;
    #1;
    begin : block_relay
      int block_copy[2][1:0];
      block_copy = src;
      from_block = block_copy;
    end
    relay();
    #1;
    $display("block %b", from_block == src);
    $display("automatic %b", from_automatic == src);
    $finish;
  end
endmodule
"#,
        &["block 1", "automatic 1"],
    );
}
