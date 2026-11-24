//! IEEE 1800-2023 §9.2.2.2.1, §7.2.1, §7.4: an `always_comb` that writes a
//! member of an element of a packed array of packed structs
//! (`r[0].data.id[i] = '0; r[0].data.id[i] |= ...;`) writes `r`, and that
//! longest static prefix is not part of its implicit sensitivity. The write
//! collector recorded no write for a member below an index, so each such
//! block wrote nothing as far as the scheduler knew and stayed sensitive to
//! all of `r`. Two of them building disjoint elements of one array woke each
//! other after every write and exhausted the settle limit in every time
//! slot (#1566). A read of another element's member (`r[1].tag`) still wakes the
//! block that reads it: the read is outside every prefix the block writes.
//! The first test's values agree with Verilator 5.046. The second follows
//! §9.2.2.2.1; Verilator 5.046 leaves the whole variable out of the block's
//! sensitivity and prints `00` twice.

use std::process::Command;

fn run(name: &str, src: &str) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_cimw_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.sv"));
    std::fs::write(&path, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "test",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    text
}

#[test]
fn blocks_building_disjoint_elements_settle() {
    let text = run(
        "disjoint",
        r#"module test;
  typedef struct packed { logic [7:0] mask; logic [7:0][2:0] id; } data_t;
  typedef struct packed { logic [7:0] tag; data_t data; } req_t;
  req_t [1:0] r;
  logic [7:0][1:0][2:0] a, b;
  logic [1:0] ga, gb;
  assign r[0].tag = 8'h11;
  assign r[0].data.mask = 8'h0f;
  always_comb begin
    for (integer i = 0; i < 8; i++) begin
      r[0].data.id[i] = '0;
      for (integer j = 0; j < 2; j++)
        r[0].data.id[i] |= a[i][j] & {3{ga[j]}};
    end
  end
  assign r[1].tag = 8'h22;
  assign r[1].data.mask = 8'hf0;
  always_comb begin
    r[1].data.id = '0;
    r[1].data.id[0] |= b[0][0] & {3{gb[0]}};
    r[1].data.id[1] |= b[1][1] & {3{gb[1]}};
    for (integer i = 2; i < 8; i++)
      r[1].data.id[i] |= b[i][0] & {3{gb[0]}} | b[i][1] & {3{gb[1]}};
  end
  initial begin
    #1;
    for (int i = 0; i < 8; i++) begin
      a[i][0] = 3'(i); a[i][1] = 3'(7 - i);
      b[i][0] = 3'(i + 1); b[i][1] = 3'(i + 2);
    end
    ga = 2'b01; gb = 2'b10;
    #1 $display("T|a r=%h", r);
    ga = 2'b10; gb = 2'b01;
    #1 $display("T|b r=%h", r);
    $finish;
  end
endmodule
"#,
    );
    assert!(!text.contains("settle limit"), "{text}");
    assert!(text.contains("T|a r=22f023eb18110ffac688"), "{text}");
    assert!(text.contains("T|b r=22f01f58c1110f053977"), "{text}");
}

#[test]
fn read_of_another_element_member_still_wakes_the_block() {
    let text = run(
        "other",
        r#"module test;
  typedef struct packed { logic [3:0] tag; logic [1:0][2:0] id; } e_t;
  e_t [1:0] r;
  logic [3:0] t;
  assign r[1].tag = t;
  assign r[1].id = '0;
  assign r[0].tag = 4'h5;
  always_comb begin
    r[0].id = '0;
    r[0].id[1] |= r[1].tag[2:0];
  end
  initial begin
    t = 4'h3;
    #1 $display("T|a id=%h", r[0].id);
    t = 4'h6;
    #1 $display("T|b id=%h", r[0].id);
    $finish;
  end
endmodule
"#,
    );
    assert!(!text.contains("settle limit"), "{text}");
    assert!(text.contains("T|a id=18"), "{text}");
    assert!(text.contains("T|b id=30"), "{text}");
}
