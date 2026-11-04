//! IEEE 1800-2023 §6.24.3 / §11.4.14: a bit-stream cast to an unpacked
//! array, dynamic array or queue unpacks the operand's bits leftmost element
//! first (a descending range starts at its left bound, a 2-D array goes row
//! by row), and a cast FROM an unpacked collection packs its elements the
//! same way; `%p` lists a descending array in declared order (§21.2.1.7).
//! Expected values come from the reference simulator.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use xezim::simulate;

fn t_lines(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

#[test]
fn cast_into_unpacked_audit_repro() {
    const SRC: &str = r#"
module rbc;
  typedef byte barr_t[4];
  typedef bit [7:0] q8_t[$];
  typedef logic [3:0] n4_t[2];
  barr_t ba;
  int x32;
  q8_t qq;
  n4_t n4;
  initial begin
    ba = barr_t'(32'h01020304); $display("T|r1|%p", ba);
    x32 = int'(ba); $display("T|r2|%h", x32);
    qq = q8_t'(24'habcdef); $display("T|r3|%p", qq);
    n4 = n4_t'(8'h5a); $display("T|r4|%p", n4);
  end
endmodule"#;
    let sim = simulate(SRC, 1000).expect("sim");
    assert_eq!(
        t_lines(&sim),
        [
            "T|r1|'{1, 2, 3, 4}",
            "T|r2|01020304",
            "T|r3|'{'hab, 'hcd, 'hef}",
            "T|r4|'{'h5, 'ha}",
        ],
    );
}

#[test]
fn cast_into_unpacked_targets() {
    const SRC: &str = r#"
module c1;
  typedef byte barr_t[4];
  typedef bit [7:0] q8_t[$];
  typedef logic [3:0] n4_t[2];
  typedef bit [7:0] d8_t[];
  typedef shortint s16_t[2];
  typedef bit [15:0] q16_t[$];
  typedef bit [3:0] a2_t[2][2];
  typedef bit [7:0] asc_t[0:2];
  typedef bit [7:0] desc_t[2:0];
  barr_t ba; int x32; q8_t qq; n4_t n4; d8_t dd; s16_t ss; q16_t q16; a2_t a2; asc_t as3; desc_t ds3;
  bit [23:0] b24; bit [31:0] b32;
  initial begin
    ba = barr_t'(32'h01020304); $display("T|r1|%p", ba);
    x32 = int'(ba); $display("T|r2|%h", x32);
    qq = q8_t'(24'habcdef); $display("T|r3|%p %0d", qq, qq.size());
    n4 = n4_t'(8'h5a); $display("T|r4|%p", n4);
    dd = d8_t'(16'hbeef); $display("T|r5|%p %0d", dd, dd.size());
    ss = s16_t'(32'hffff0001); $display("T|r6|%p", ss);
    q16 = q16_t'(32'h12345678); $display("T|r7|%p", q16);
    a2 = a2_t'(16'h1234); $display("T|r8|%p", a2);
    as3 = asc_t'(24'h010203); ds3 = desc_t'(24'h010203); $display("T|r9|%p %p %0d %0d", as3, ds3, as3[0], ds3[0]);
    b24 = 24'(qq); $display("T|r10|%h", b24);
    b32 = 32'(ss); $display("T|r11|%h", b32);
    begin
      automatic barr_t lb = barr_t'(32'h0a0b0c0d);
      automatic q8_t lq = q8_t'(16'h1122);
      $display("T|r12|%p %p", lb, lq);
    end
    qq = q8_t'(ba); $display("T|r13|%p", qq);
    ba = barr_t'(qq[0:3]); $display("T|r14|%p", ba);
  end
endmodule"#;
    let sim = simulate(SRC, 1000).expect("sim");
    assert_eq!(
        t_lines(&sim),
        [
            "T|r1|'{1, 2, 3, 4}",
            "T|r2|01020304",
            "T|r3|'{'hab, 'hcd, 'hef} 3",
            "T|r4|'{'h5, 'ha}",
            "T|r5|'{'hbe, 'hef} 2",
            "T|r6|'{-1, 1}",
            "T|r7|'{'h1234, 'h5678}",
            "T|r8|'{'{'h1, 'h2}, '{'h3, 'h4}}",
            "T|r9|'{'h1, 'h2, 'h3} '{'h1, 'h2, 'h3} 1 3",
            "T|r10|abcdef",
            "T|r11|ffff0001",
            "T|r12|'{10, 11, 12, 13} '{'h11, 'h22}",
            "T|r13|'{'h1, 'h2, 'h3, 'h4}",
            "T|r14|'{1, 2, 3, 4}",
        ],
    );
}

#[test]
fn cast_from_unpacked_operands() {
    const SRC: &str = r#"
module c3;
  typedef byte barr_t[4];
  barr_t ba; bit [7:0] qq[$]; shortint ss[2]; int x32; bit [23:0] b24; bit [31:0] b32; bit [3:0] a2[2][2]; bit [15:0] b16;
  bit [7:0] ds3[2:0];
  initial begin
    ba[0] = 1; ba[1] = 2; ba[2] = 3; ba[3] = 4;
    qq.push_back(8'hab); qq.push_back(8'hcd); qq.push_back(8'hef);
    ss[0] = -1; ss[1] = 1;
    a2[0][0] = 1; a2[0][1] = 2; a2[1][0] = 3; a2[1][1] = 4;
    ds3[2] = 1; ds3[1] = 2; ds3[0] = 3;
    x32 = int'(ba); $display("T|v1|%h", x32);
    b24 = 24'(qq); $display("T|v2|%h", b24);
    b32 = 32'(ss); $display("T|v3|%h", b32);
    b16 = 16'(a2); $display("T|v4|%h", b16);
    b24 = 24'(ds3); $display("T|v5|%h", b24);
    x32 = {>>{ba}}; $display("T|v6|%h", x32);
  end
endmodule"#;
    let sim = simulate(SRC, 1000).expect("sim");
    assert_eq!(
        t_lines(&sim),
        [
            "T|v1|01020304",
            "T|v2|abcdef",
            "T|v3|ffff0001",
            "T|v4|1234",
            "T|v5|010203",
            "T|v6|01020304",
        ],
    );
}

#[test]
fn stream_into_unpacked_order() {
    const SRC: &str = r#"
module c2;
  typedef byte barr_t[4];
  barr_t ba; bit [7:0] qq[$]; logic [3:0] n4[2]; bit [3:0] a2[2][2]; bit [7:0] ds3[2:0]; bit [7:0] dd[];
  initial begin
    {>>{ba}} = 32'h01020304; $display("T|s1|%p", ba);
    qq = {>>{24'habcdef}}; $display("T|s2|%p", qq);
    {>>{n4}} = 8'h5a; $display("T|s3|%p", n4);
    {>>{a2}} = 16'h1234; $display("T|s4|%p", a2);
    {>>{ds3}} = 24'h010203; $display("T|s5|%p %0d", ds3, ds3[0]);
    dd = new[2]; {>>{dd}} = 16'hbeef; $display("T|s6|%p", dd);
  end
endmodule"#;
    let sim = simulate(SRC, 1000).expect("sim");
    assert_eq!(
        t_lines(&sim),
        [
            "T|s1|'{1, 2, 3, 4}",
            "T|s2|'{'hab, 'hcd, 'hef}",
            "T|s3|'{'h5, 'ha}",
            "T|s4|'{'{'h1, 'h2}, '{'h3, 'h4}}",
            "T|s5|'{'h1, 'h2, 'h3} 3",
            "T|s6|'{'hbe, 'hef}",
        ],
    );
}

#[test]
fn print_descending_array_in_declared_order() {
    const SRC: &str = r#"
module c4;
  bit [7:0] ds3[2:0]; int a14[1:4]; int d41[4:1];
  initial begin
    ds3[2] = 1; ds3[1] = 2; ds3[0] = 3;
    foreach (a14[i]) a14[i] = i*10; foreach (d41[i]) d41[i] = i*10;
    $display("T|p1|%p %p %p", ds3, a14, d41);
  end
endmodule"#;
    let sim = simulate(SRC, 1000).expect("sim");
    assert_eq!(
        t_lines(&sim),
        ["T|p1|'{'h1, 'h2, 'h3} '{10, 20, 30, 40} '{40, 30, 20, 10}",],
    );
}
