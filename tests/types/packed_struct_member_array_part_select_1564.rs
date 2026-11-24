//! §7.2.1, §7.4.1, §11.5.1: bit-selects and constant/indexed part-selects
//! on elements of packed-array members inside packed or unpacked structs.

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn packed_struct_member_array_part_selects() {
    let src = r#"
module test;
  typedef struct packed {
    logic [3:0] tag;
    logic [0:1][1:4] asc_f;
    logic [1:0][3:0] f;
  } e_t;
  e_t [1:0] r, rc;
  e_t ur [2];
  logic [3:0] m1;
  logic [1:0] k = 0;

  always_comb m1 = r[0].f[0];
  assign rc[0].f[0][3:1] = 3'b011;

  initial begin
    r = '0;
    r[0].f[0] = 4'h5;
    r[0].f[0][3:1] = 3'b011;     #1 $display("T|range %h %h", r[0].f[0], m1);
    r = '0; r[0].f[0] = 4'h5;
    r[0].f[0][1] = 1'b1;         #1 $display("T|bit %h %h", r[0].f[0], m1);
    r = '0; r[0].f[0] = 4'h5;
    r[0].f[0][k+:2] = 2'b10;     #1 $display("T|indexed %h %h %b", r[0].f[0], m1, r[0].f[0][k+:2]);
    r = '0; r[0].f[0] = 4'h5;
    r[0].f[1][3:1] = 3'b011;     #1 $display("T|f1 %h", r[0].f[1]);
    r = '0;
    r[0].tag[3:1] = 3'b011;      #1 $display("T|tag %h", r[0].tag);

    r = '0; r[0].asc_f[0] = 4'b1010;
    r[0].asc_f[0][2:4] = 3'b011;
    $display("T|asc %b %b", r[0].asc_f[0], r[0].asc_f[0][1:3]);

    ur[0] = '0; ur[0].f[0] = 4'h5;
    ur[0].f[0][3:1] = 3'b011;
    $display("T|unpacked %h %b", ur[0].f[0], ur[0].f[0][2:0]);

    r = '0; r[0].f[0] = 4'h5;
    r[0].f[0][3:1] <= 3'b011;
    #1;
    $display("T|nba %h cont=%b", r[0].f[0], rc[0].f[0]);

    begin
      e_t [1:0] lr;
      lr = '0; lr[0].f[0] = 4'h5;
      lr[0].f[0][3:1] = 3'b011;
      $display("T|local %h", lr[0].f[0]);
    end
    $finish;
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "T|range 7 7",
            "T|bit 7 7",
            "T|indexed 6 6 10",
            "T|f1 6",
            "T|tag 6",
            "T|asc 1011 101",
            "T|unpacked 7 111",
            "T|nba 7 cont=011x",
            "T|local 7",
        ]
    );
}
