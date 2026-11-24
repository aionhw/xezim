//! §6.11.2, §7.2.1: unrolled constant packed member stores compile for
//! 2-state, 4-state, and mixed containers while converting x/z values to 0 for
//! 2-state members and preserving x for 4-state members.

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
fn unrolled_two_state_and_mixed_packed_member_stores() {
    let src = r#"
module packed_loop_check;
  localparam int entry = 0;

  typedef struct packed { logic lock; logic [5:0] value; } ctrl4_t;
  typedef struct packed { ctrl4_t csr; } ent4_t;
  typedef struct packed { ent4_t [7:0] entries; } bank4_t;

  typedef struct packed { bit lock; bit [5:0] value; } ctrl2_t;
  typedef struct packed { ctrl2_t csr; } ent2_t;
  typedef struct packed { ent2_t [7:0] entries; } bank2_t;

  typedef struct packed { bit lock2; logic [5:0] value4; } mixed_t;
  typedef struct packed { mixed_t [3:0] m; } mbank_t;

  bank4_t wr4, wr4_nba;
  bank2_t wr2, wr2_nba;
  mbank_t mb, mb_nba;
  logic [47:0] words;
  logic [7:0] bits, rd_locks;
  logic clk = 0;

  always_comb begin
    for (int entry = 0; entry < 8; entry++) begin
      wr4.entries[entry].csr.lock = bits[entry];
      wr4.entries[entry].csr.value = words[entry*6 +: 6];
      wr2.entries[entry].csr.lock = bits[entry];
      wr2.entries[entry].csr.value = words[entry*6 +: 6];
      rd_locks[entry] = wr4.entries[entry].csr.lock;
    end
    for (int i = 0; i < 4; i++) begin
      mb.m[i].lock2 = bits[i];
      mb.m[i].value4 = words[i*6 +: 6];
    end
  end

  always_ff @(posedge clk) begin
    for (int entry = 0; entry < 8; entry++) begin
      wr4_nba.entries[entry].csr.lock <= bits[entry];
      wr4_nba.entries[entry].csr.value <= words[entry*6 +: 6];
      wr2_nba.entries[entry].csr.lock <= bits[entry];
      wr2_nba.entries[entry].csr.value <= words[entry*6 +: 6];
    end
    for (int i = 0; i < 4; i++) begin
      mb_nba.m[i].lock2 <= bits[i];
      mb_nba.m[i].value4 <= words[i*6 +: 6];
    end
  end

  initial begin
    words = 48'h9876543210ab; bits = 8'ha3;
    #1; clk = 1; #1;
    $display("T|bank=%h/%h rd=%h nba=%h/%h", wr4, wr2, rd_locks, wr4_nba, wr2_nba);
    $display("T|mixed=%h/%h", mb, mb_nba);

    words = 48'hxxx6_54x2_1xab;
    bits = 8'b10xx_101x;
    #1; clk = 0; #1; clk = 1; #1;
    $display("T|xz4=%h xz2=%h", wr4, wr2);
    $display("T|xz_nba=%h/%h", wr4_nba, wr2_nba);
    $display("T|xz_mix=%h/%b,%b", mb, mb_nba.m[0].lock2, mb_nba.m[1].value4);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "T|bank=cc1ec94188616b/cc1ec94188616b rd=a3 nba=cc1ec94188616b/cc1ec94188616b",
            "T|mixed=188616b/188616b",
            "T|xz4=XXxXcX4XX8XXXb xz2=8000c94808612b",
            "T|xz_nba=XXxXcX4XX8XXXb/8000c94808612b",
            "T|xz_mix=XX8XX2b/0,xxxx10",
        ]
    );
}
