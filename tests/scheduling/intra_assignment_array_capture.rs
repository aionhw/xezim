//! IEEE 1800-2023 §9.4.5, §7.6: an intra-assignment delay or event control
//! captures the right-hand side when the statement runs. An unpacked array
//! assignment copies the elements in declared order.

fn check_output(source: &str, expected: &[&str]) {
    let sim = xezim::simulate(source, 100).expect("simulate failed");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(lines, expected);
}

#[test]
fn blocking_delay_captures_a_subarray() {
    check_output(
        r#"
module top;
  int src[2][2] = '{'{31, 32}, '{41, 42}};
  int dst[2];
  initial begin
    dst = #4 src[1];
    $display("T|dst=%0d %0d at %0d", dst[0], dst[1], $time);
  end
  initial #1 src[1] = '{61, 62};
endmodule
"#,
        &["T|dst=41 42 at 4"],
    );
}

#[test]
fn nonblocking_delay_captures_a_subarray() {
    check_output(
        r#"
module top;
  int src[2][2] = '{'{31, 32}, '{41, 42}};
  int q[2] = '{11, 12};
  initial begin
    q <= #4 src[1];
    #1 $display("T|early q=%0d %0d at %0d", q[0], q[1], $time);
    src[1] = '{61, 62};
    #4 $display("T|late q=%0d %0d", q[0], q[1]);
  end
endmodule
"#,
        &["T|early q=11 12 at 1", "T|late q=41 42"],
    );
}

#[test]
fn blocking_event_captures_a_subarray() {
    check_output(
        r#"
module top;
  int src[2][2] = '{'{31, 32}, '{41, 42}};
  int r[2];
  event go;
  initial begin
    r = @(go) src[1];
    $display("T|r=%0d %0d at %0d", r[0], r[1], $time);
  end
  initial begin
    #1 src[1] = '{61, 62};
    -> go;
  end
endmodule
"#,
        &["T|r=41 42 at 1"],
    );
}

#[test]
fn nonblocking_event_captures_a_subarray() {
    check_output(
        r#"
module top;
  int src[2][2] = '{'{31, 32}, '{41, 42}};
  int dst[2] = '{11, 12};
  event go;
  initial begin
    dst <= @(go) src[1];
    #1 $display("T|before event dst=%0d %0d at %0d", dst[0], dst[1], $time);
    src[1] = '{61, 62};
    -> go;
    #1 $display("T|after event dst=%0d %0d", dst[0], dst[1]);
  end
endmodule
"#,
        &["T|before event dst=11 12 at 1", "T|after event dst=41 42"],
    );
}

#[test]
fn blocking_delay_captures_a_whole_two_dimensional_array() {
    check_output(
        r#"
module top;
  int src[2][2] = '{'{31, 32}, '{41, 42}};
  int dst[2][2];
  initial begin
    dst = #4 src;
    $display("T|dst[0]=%0d %0d dst[1]=%0d %0d", dst[0][0], dst[0][1], dst[1][0], dst[1][1]);
  end
  initial #1 src[1] = '{61, 62};
endmodule
"#,
        &["T|dst[0]=31 32 dst[1]=41 42"],
    );
}

#[test]
fn blocking_delay_captures_a_subarray_of_a_three_dimensional_array() {
    check_output(
        r#"
module top;
  int s3[2][2][2];
  int dst[2];
  initial begin
    s3[1][0] = '{51, 52};
    dst = #4 s3[1][0];
    $display("T|dst=%0d %0d", dst[0], dst[1]);
  end
  initial #1 s3[1][0] = '{71, 72};
endmodule
"#,
        &["T|dst=51 52"],
    );
}

#[test]
fn undelayed_assignment_copies_a_subarray() {
    check_output(
        r#"
module top;
  int src[2][2] = '{'{31, 32}, '{41, 42}};
  int dst[2];
  initial begin
    dst = src[1];
    $display("T|dst=%0d %0d at %0d", dst[0], dst[1], $time);
  end
  initial #1 src[1] = '{61, 62};
endmodule
"#,
        &["T|dst=41 42 at 0"],
    );
}

#[test]
fn blocking_delay_captures_a_one_dimensional_array() {
    check_output(
        r#"
module top;
  int one[2] = '{1, 2};
  int w[2];
  initial begin
    w = #4 one;
    $display("T|w=%0d %0d", w[0], w[1]);
  end
  initial #1 one = '{8, 9};
endmodule
"#,
        &["T|w=1 2"],
    );
}

#[test]
fn blocking_delay_captures_a_one_dimensional_self_assignment() {
    check_output(
        r#"
module top;
  int wc[2] = '{1, 2};
  initial begin
    wc = #1 wc;
    $display("T|wc=%0d %0d", wc[0], wc[1]);
  end
  initial #0 wc = '{8, 9};
endmodule
"#,
        &["T|wc=1 2"],
    );
}

#[test]
fn nonblocking_delay_captures_a_subarray_into_another_subarray() {
    check_output(
        r#"
module top;
  int src[2][2] = '{'{31, 32}, '{41, 42}};
  int dst2[2][2];
  initial begin
    dst2[0] <= #4 src[1];
    #1 src[1] = '{61, 62};
    #5 $display("T|dst2[0]=%0d %0d", dst2[0][0], dst2[0][1]);
  end
endmodule
"#,
        &["T|dst2[0]=41 42"],
    );
}
