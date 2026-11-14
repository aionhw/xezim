//! IEEE 1800-2023 §10.3.3: a vector continuous assignment uses fall for
//! nonzero -> all 0, off for -> all z, and rise for every other transition,
//! including all x and partly z. A net declaration assignment carries the
//! same assignment delay (§10.3.1, §10.3.3); it is not a net delay.
//! Scalar controls follow §28.16: to x uses the minimum delay. With two
//! delays, vector and scalar transitions to all z use §28.16's minimum.
//! Built-in logic nets and positive two/three-delay forms are legal under
//! §6.7.1, §10.3 and §28.16.
//! Variables and packed bounds follow §6.8 and §6.9. Equal bounds retain
//! vector shape (reading §6.9 with the equal bounds allowed by §6.9.1),
//! as do width-one part-selects (§11.5.1).
//! Stimulus uses blocking assignments (§10.4.1) and delay controls (§9.4.1);
//! an automatic task (§13.3) reports via $display (§21.2.1), then $finish
//! (§20.2). `timescale (§22.7) sets the units reported by $time (§20.3.1).
//! Samples avoid stimulus and output-update times (§4.7).
//! An AND gate array (§28.3.5, §28.4) in an instance (§23.3) shares its net
//! with a z driver (§28.12); case inequality (§11.4.5) checks that its
//! transition to x still uses §28.16's minimum. An inlined bufif (§28.6)
//! control must retain its old value before that minimum has elapsed.
//! Selected drivers (§11.5.1) keep their own transition values when another
//! driver overlaps them. A user-written z-padded concatenation with a size
//! cast (§11.4.12, §6.24.1) remains a whole-vector assignment. Named
//! conditional and loop generate scopes (§27.5, §27.4) retain declared shape.
//! Guards cover typedef packed-array reads/writes and $dimensions (§6.18,
//! §7.4.1, §20.7), plus a delayed part beside a bufif (§28.6, §28.16).
//! Regression: fails on main even with --no-cache; this is runtime scheduling.

use std::process::Command;

#[test]
fn vector_continuous_assignments_use_rise_for_x_and_mixed_z() {
    let dir = std::env::temp_dir().join(format!("vector_continuous_delay_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("design.sv");
    std::fs::write(
        &input,
        r#"
`timescale 1ns/1ps
module top;
  logic [3:0] x;
  logic a;
  typedef logic [0:0] one_t;
  one_t [3:0] packed_elements;
  logic neighbor_a, neighbor_b;
  wire [7:0] neighbor, child_neighbor;
  assign #(6, 2) neighbor[3:0] = {4{neighbor_a}};
  bufif1 #(4, 6, 2) neighbor_gate(neighbor[4], neighbor_b, 1'b1);
  neighbor_driver neighbor_child(neighbor_a, neighbor_b, child_neighbor);
  initial begin
    packed_elements = 4'b0110;
    if (packed_elements[1] !== 1'b1 || packed_elements[0] !== 1'b0)
      $display("FAIL typedef element read");
    if ($dimensions(packed_elements) != 2) $display("FAIL typedef dimensions");
    packed_elements[3] = 1'b1;
    if (packed_elements !== 4'b1110) $display("FAIL typedef element write");
    neighbor_a = 0; neighbor_b = 0;
    #9;
    if (neighbor[4:0] !== 5'b00000 || child_neighbor[4:0] !== 5'b00000)
      $display("FAIL neighbor initial");
    #1 neighbor_a = 1; neighbor_b = 1;
    #1;
    if (neighbor[4] !== 1'b0 || child_neighbor[4] !== 1'b0)
      $display("FAIL neighbor early bufif");
    #6;
    if (neighbor[4:0] !== 5'b11111 || child_neighbor[4:0] !== 5'b11111)
      $display("FAIL neighbor settled");
  end
  wire [3:0] v2, v3, v4;
  wire [3:0] #(4, 2) nd2 = x;
  wire [3:0] #(4, 6, 2) nd3 = x;
  wire s2, s3, s4;
  wire [3:0] gate_value;
  delayed_gate_driver g(x, gate_value);
  logic [3:0] selected;
  logic scalar;
  wire [7:0] selected_net, scalar_net, literal_net;
  assign #(4, 2) selected_net[3:0] = selected;
  assign selected_net = 8'bzzzzzzzz;
  assign #(4, 6, 2) scalar_net[5] = scalar;
  assign scalar_net = 8'bzzzzzzzz;
  assign #(4, 2) literal_net = {4'bzzzz, 4'(selected)};
  assign #(4, 2) v2 = x;
  assign #(4, 6, 2) v3 = x;
  assign #(4, 2, 6) v4 = x;
  assign #(4, 2) s2 = a;
  assign #(4, 6, 2) s3 = a;
  assign #(4, 2, 6) s4 = a;
  logic shape_bit;
  logic [3:0] shape_value;
  wire [0:0] one2, one3, child_one;
  singleton_driver child(shape_bit, child_one);
  wire [0:0] #(4, 2) net_delay;
  assign net_delay = shape_bit;
  wire [0:0] #(4, 2) one_decl = shape_bit;
  wire [7:0] one_part, known_part;
  if (1) begin : singleton_if
    wire [0:0] v;
    assign #(4, 2) v = shape_bit;
    wire [0:0] #(4, 6, 2) d = shape_bit;
  end
  for (genvar i = 0; i < 2; i++) begin : singleton_for
    wire [0:0] v;
    assign #(4, 6, 2) v = shape_bit;
    wire [0:0] #(4, 2) d = shape_bit;
  end
  wire [7:0] gate_slice;
  selected_gate_driver selected_gate(shape_bit, gate_slice);
  assign #(4, 2) one2 = shape_bit;
  assign #(4, 6, 2) one3 = shape_bit;
  assign #(4, 2) one_part[5:5] = shape_bit;
  assign one_part = 'z;
  assign #(4, 2) known_part[3:0] = shape_value;
  assign known_part = 'z;
  task automatic shape_sample;
    $display("S %0d %b %b %b %b %b %b %b %b %b %b %b", $time,
      one2, one3, one_decl, one_part[5:5], known_part[3:0],
      singleton_if.v, singleton_if.d,
      singleton_for[0].v, singleton_for[0].d,
      singleton_for[1].v, singleton_for[1].d);
  endtask
  task automatic sample;
    $display("R %0d %b %b %b %b %b %b%b%b", $time, v2, v3, v4, nd2, nd3, s2, s3, s4);
  endtask
  initial begin
    x = 4'h3; a = 1'b1;
    shape_bit = 1'b1; shape_value = 4'h3;
    selected = 4'h3; scalar = 1'b1;
    #10 x = 4'bxxxx; a = 1'bx;
    selected = 4'h0; scalar = 1'bx;
    #1 sample; // t11: before minimum delay 2
    #2 sample; // t13: scalar x arrived, vector x still pending until 14
    if (gate_value !== 4'bxxxx) $display("FAIL gate delay %b", gate_value);
    if (selected_net[3:0] !== 4'h0) $display("FAIL selected fall %b", selected_net);
    if (scalar_net[5] !== 1'bx) $display("FAIL selected scalar x %b", scalar_net);
    if (literal_net[3:0] !== 4'h3) $display("FAIL literal vector rise %b", literal_net);
    #2 sample; // t15
    #5 x = 4'h3; a = 1'b1;
    selected = 4'bxxxx; scalar = 1'b0;
    #3 sample; // t23
    if (selected_net[3:0] !== 4'h0) $display("FAIL selected x before %b", selected_net);
    #2 sample; // t25
    if (selected_net[3:0] !== 4'bxxxx) $display("FAIL selected x after %b", selected_net);
    if (scalar_net[5] !== 1'bx) $display("FAIL selected scalar fall before %b", scalar_net);
    #2 if (scalar_net[5] !== 1'b0) $display("FAIL selected scalar fall after %b", scalar_net);
    #3 x = 4'b00z1; a = 1'bz;
    selected = 4'b00z0;
    #1 sample; // t31
    #2 sample; // t33: partly z still needs the vector rising delay 4
    if (selected_net[3:0] !== 4'bxxxx) $display("FAIL selected mixed before %b", selected_net);
    #2 sample; // t35
    if (selected_net[3:0] !== 4'b00z0) $display("FAIL selected mixed after %b", selected_net);
    #2 sample; // t37: scalar s4's explicit off delay 6 has elapsed
    #3 x = 4'bzzzz; a = 1'b1;
    #1 sample; // t41
    #2 sample; // t43: two-delay minimum and explicit off=2, not off=6
    #2 sample; // t45
    #2 sample; // t47
    #3 x = 4'h3; // t50: settle a known nonzero value before the fall
    #10 x = 4'b0000; a = 1'b0;
    scalar = 1'b1;
    #1 sample; // t61
    #2 sample; // t63: fall=2, not fall=6
    if (scalar_net[5] !== 1'b0) $display("FAIL selected scalar rise before %b", scalar_net);
    #2 sample; // t65
    if (scalar_net[5] !== 1'b1) $display("FAIL selected scalar rise after %b", scalar_net);
    #2 sample; // t67
    #3 x = 4'h5; a = 1'b1;
    #3 sample; // t73
    #2 sample; // t75: all other known nonzero values also use rise
    #5 shape_bit = 1'bx; shape_value = 4'h5; // t80
    #1 if (gate_slice[5] !== 1'b1) $display("FAIL selected gate before minimum %b", gate_slice);
    #2 shape_sample; // t83: singleton vectors and known part still wait for rise
    if (child_one !== 1'b1) $display("FAIL child singleton before %b", child_one);
    if (net_delay !== 1'bx) $display("FAIL one-bit net delay %b", net_delay);
    #2 shape_sample; // t85
    if (child_one !== 1'bx) $display("FAIL child singleton after %b", child_one);
    #5 shape_bit = 1'b1; // t90: settle known nonzero before the vector fall
    #10 shape_bit = 1'b0; shape_value = 4'h0; // t100
    #3 shape_sample; // t103: two-delay vectors fall=2; three-delay vectors fall=6
    #2 shape_sample; // t105
    #2 shape_sample; // t107
    #3 shape_bit = 1'bz; shape_value = 4'bzzzz; // t110
    #1 shape_sample; // t111: before turn-off
    #2 shape_sample; // t113: all vectors use off=2
    $finish;
  end
endmodule
module neighbor_driver(input wire a, b, output wire [7:0] w);
  assign #(6, 2) w[3:0] = {4{a}};
  bufif1 #(4, 6, 2) g(w[4], b, 1'b1);
endmodule
module selected_gate_driver(input wire a, output wire [7:0] w);
  bufif1 #(4, 2) g(w[5], a, 1'b1);
  assign w = 'z;
endmodule
module singleton_driver(input wire a, output wire [0:0] v);
  assign #(4, 2) v = a;
endmodule
module delayed_gate_driver(input wire [3:0] a, output wire [3:0] w);
  and #(4, 2) gates[3:0](w, a, 4'b1111);
  assign w = 4'bzzzz;
endmodule
"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg(&input)
        .arg("--no-cache")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.lines().any(|line| line.starts_with("FAIL ")),
        "{stdout}"
    );
    let got: Vec<_> = stdout
        .lines()
        .filter(|line| line.starts_with("R "))
        .collect();
    assert_eq!(
        got,
        [
            "R 11 0011 0011 0011 0011 0011 111",
            "R 13 0011 0011 0011 0011 0011 xxx",
            "R 15 xxxx xxxx xxxx xxxx xxxx xxx",
            "R 23 xxxx xxxx xxxx xxxx xxxx xxx",
            "R 25 0011 0011 0011 0011 0011 111",
            "R 31 0011 0011 0011 0011 0011 111",
            "R 33 0011 0011 0011 0011 0011 zz1",
            "R 35 00z1 00z1 00z1 00z1 00z1 zz1",
            "R 37 00z1 00z1 00z1 00z1 00z1 zzz",
            "R 41 00z1 00z1 00z1 00z1 00z1 zzz",
            "R 43 zzzz zzzz 00z1 zzzz zzzz zzz",
            "R 45 zzzz zzzz 00z1 zzzz zzzz 111",
            "R 47 zzzz zzzz zzzz zzzz zzzz 111",
            "R 61 0011 0011 0011 0011 0011 111",
            "R 63 0000 0011 0000 0000 0011 010",
            "R 65 0000 0011 0000 0000 0011 010",
            "R 67 0000 0000 0000 0000 0000 000",
            "R 73 0000 0000 0000 0000 0000 000",
            "R 75 0101 0101 0101 0101 0101 111",
        ],
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let shapes: Vec<_> = stdout
        .lines()
        .filter(|line| line.starts_with("S "))
        .collect();
    assert_eq!(
        shapes,
        [
            "S 83 1 1 1 1 0011 1 1 1 1 1 1",
            "S 85 x x x x 0101 x x x x x x",
            "S 103 0 1 0 0 0000 0 1 1 0 1 0",
            "S 105 0 1 0 0 0000 0 1 1 0 1 0",
            "S 107 0 0 0 0 0000 0 0 0 0 0 0",
            "S 111 0 0 0 0 0000 0 0 0 0 0 0",
            "S 113 z z z z zzzz z z z z z z",
        ],
        "{stdout}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
