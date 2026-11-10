//! IEEE 1800-2017 §7.8, §7.4.1, §7.4.6: an associative array whose element
//! is a multi-dimensional, offset or ascending packed vector stores each
//! element under its key, at the element's full width. Writes landed in a
//! scalar signal named after the array instead (no entry was created), so
//! `p[1]` of `logic [3:0][7:0] p [int]` read 8 bits. Expected values are
//! from the reference simulator.

use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_line(msgs: &[String], want: &str) {
    assert!(
        msgs.iter().any(|m| m.trim() == want),
        "missing {want:?} in {msgs:#?}"
    );
}

#[test]
fn multi_dimensional_packed_element_is_stored_whole() {
    let msgs = messages(
        "module t;
  logic [3:0][7:0] p [int];
  initial begin
    p[1] = 32'h11223344;
    #1 $display(\"p=%h bits=%0d n=%0d\", p[1], $bits(p[1]), p.num());
  end
endmodule",
    );
    assert_line(&msgs, "p=11223344 bits=32 n=1");
}

#[test]
fn offset_and_ascending_elements_are_stored() {
    let msgs = messages(
        "module t;
  logic [8:1] r [int];
  logic [0:7] u [int];
  initial begin
    r[1] = 8'h02; u[1] = 8'h80;
    #1 $display(\"r=%h r1=%b r2=%b u=%h u0=%b u7=%b\", r[1], r[1][1], r[1][2], u[1], u[1][0], u[1][7]);
  end
endmodule",
    );
    assert_line(&msgs, "r=02 r1=0 r2=1 u=80 u0=1 u7=0");
}

#[test]
fn select_into_a_packed_array_element() {
    let msgs = messages(
        "module t;
  logic [3:0][7:0] p [int];
  logic [0:3][7:0] q [string];
  bit [1:0][3:0] b2 [int];
  initial begin
    p[1] = 32'h11223344; q[\"a\"] = 32'hAABBCCDD; b2[5] = 8'h5A;
    #1 $display(\"p1=%h p3=%h q0=%h q3=%h b2=%h\", p[1][1], p[1][3], q[\"a\"][0], q[\"a\"][3], b2[5][1]);
  end
endmodule",
    );
    assert_line(&msgs, "p1=33 p3=11 q0=aa q3=dd b2=5");
}

#[test]
fn bit_of_a_packed_array_element() {
    let msgs = messages(
        "module t;
  logic [3:0][7:0] p [int];
  initial begin
    p[1] = 32'h11223344;
    #1 $display(\"nest=%b\", p[1][1][4]);
  end
endmodule",
    );
    assert_line(&msgs, "nest=1");
}

#[test]
fn slice_and_bit_writes_into_elements() {
    let msgs = messages(
        "module t;
  logic [3:0][7:0] p [int];
  logic [0:3][7:0] q [string];
  logic [8:1] r [int];
  logic [0:7] u [int];
  int k;
  initial begin
    p[1] = 32'h11223344; q[\"a\"] = 32'hAABBCCDD; r[1] = 8'h02; u[1] = 8'h80;
    p[1][2] = 8'hFF; q[\"a\"][0] = 8'h01; r[1][8] = 1; u[1][7] = 1;
    k = 1; p[k][0] = 8'h00;
    #1 $display(\"p=%h q=%h r=%h u=%h\", p[1], q[\"a\"], r[1], u[1]);
  end
endmodule",
    );
    assert_line(&msgs, "p=11ff3300 q=01bbccdd r=82 u=81");
}

#[test]
fn slice_write_creates_an_element() {
    // `bit` elements: a fresh 4-state element would start all x (§7.8.6).
    let msgs = messages(
        "module t;
  bit [3:0][7:0] p [int];
  initial begin
    p[1] = 32'h11223344;
    p[2][1] = 8'h77;
    #1 $display(\"p2=%h n=%0d\", p[2], p.num());
    foreach (p[i]) $display(\"k=%0d v=%h\", i, p[i]);
  end
endmodule",
    );
    assert_line(&msgs, "p2=00007700 n=2");
    assert_line(&msgs, "k=1 v=11223344");
    assert_line(&msgs, "k=2 v=00007700");
}
