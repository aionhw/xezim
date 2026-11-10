//! IEEE 1800-2017 §7.8, §7.4.6, §11.5.1: a select past the unpacked
//! dimensions of an associative array selects into the element. `a[7][3]`
//! of `logic [31:0] a [int]` reads bit 3 of `a[7]`. The read path took
//! the extra select as another key and read the absent element `a[7][3]`,
//! all x at the element's width. Writes, part-selects, fixed arrays,
//! queues and dynamic arrays were right. Expected values are from the
//! reference simulator.

use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_line(msgs: &[String], want: &str) {
    assert!(
        msgs.iter().any(|m| m == want),
        "missing {want:?} in {msgs:?}"
    );
}

#[test]
fn bit_select_of_an_int_keyed_element() {
    let msgs = messages(
        "module t;
  logic [31:0] a [int];
  initial begin a[7] = 32'hA; #1 $display(\"b3=%b b0=%b\", a[7][3], a[7][0]); end
endmodule",
    );
    assert_line(&msgs, "b3=1 b0=0");
}

#[test]
fn bit_select_for_each_key_type() {
    let msgs = messages(
        "module t;
  typedef int key_t;
  logic [31:0] a [key_t];
  logic [31:0] c [string];
  logic [31:0] w [*];
  initial begin
    a[7] = 32'hA; c[\"x\"] = 32'hA; w[7] = 32'hA;
    #1 $display(\"a=%b c=%b w=%b\", a[7][3], c[\"x\"][3], w[7][3]);
  end
endmodule",
    );
    assert_line(&msgs, "a=1 c=1 w=1");
}

#[test]
fn variable_key_and_variable_bit() {
    let msgs = messages(
        "module t;
  logic [31:0] a [int];
  int k;
  initial begin a[7] = 32'hA; k = 7; #1 $display(\"key=%b bit=%b\", a[k][3], a[7][k-4]); end
endmodule",
    );
    assert_line(&msgs, "key=1 bit=1");
}

#[test]
fn bit_select_in_a_generate_block() {
    let msgs = messages(
        "module t;
  typedef int key_t;
  if (1) begin : g
    logic [31:0] a [key_t];
    initial begin a[7] = 32'hA; #1 $display(\"gen=%b\", a[7][3]); end
  end
endmodule",
    );
    assert_line(&msgs, "gen=1");
}

#[test]
fn bit_select_of_a_packed_struct_element() {
    let msgs = messages(
        "module t;
  typedef struct packed { logic [7:0] hi; logic [7:0] lo; } s_t;
  s_t m [int];
  initial begin m[1] = 16'hABCD; #1 $display(\"b0=%b lo=%h\", m[1][0], m[1].lo); end
endmodule",
    );
    assert_line(&msgs, "b0=1 lo=cd");
}

#[test]
fn bit_select_past_a_two_dimensional_associative_array() {
    let msgs = messages(
        "module t;
  bit [15:0] ab [int][string];
  initial begin ab[1][\"k\"] = 16'h00F0; #1 $display(\"ab=%b\", ab[1][\"k\"][4]); end
endmodule",
    );
    assert_line(&msgs, "ab=1");
}

#[test]
fn bit_select_of_an_absent_element_is_one_bit() {
    let msgs = messages(
        "module t;
  bit [7:0] bm [int];
  initial begin #1 $display(\"miss=%b\", bm[9][2]); end
endmodule",
    );
    assert_line(&msgs, "miss=0");
}

// Guards: forms that were already right and must stay so.

#[test]
fn part_selects_and_whole_element_reads_stay_right() {
    let msgs = messages(
        "module t;
  logic [31:0] a [int];
  initial begin a[7] = 32'hA; #1 $display(\"ps=%h ips=%h w=%h\", a[7][7:0], a[7][3+:4], a[7]); end
endmodule",
    );
    assert_line(&msgs, "ps=0a ips=1 w=0000000a");
}

#[test]
fn multidimensional_associative_and_queue_rows_stay_keys() {
    let msgs = messages(
        "module t;
  int m [int][int];
  int aq [int][$];
  initial begin
    m[1][2] = 5; aq[3].push_back(11); aq[3].push_back(22);
    #1 $display(\"m12=%0d m13=%0d aq31=%0d\", m[1][2], m[1][3], aq[3][1]);
  end
endmodule",
    );
    assert_line(&msgs, "m12=5 m13=0 aq31=22");
}

#[test]
fn bit_write_then_bit_read() {
    let msgs = messages(
        "module t;
  logic [31:0] a [int];
  initial begin a[7] = 32'hA; a[7][0] = 1'b1; #1 $display(\"a=%h b0=%b\", a[7], a[7][0]); end
endmodule",
    );
    assert_line(&msgs, "a=0000000b b0=1");
}
