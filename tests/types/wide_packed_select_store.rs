//! IEEE 1800-2023 §7.4.1 / §11.5.1: a store to one element, part-select or
//! bit of a packed vector edits the stored vector in place. The result has
//! to be what reading the vector out, replacing the bits and storing it back
//! gave: x/z bits dropped by a 2-state vector and kept by a 4-state one, an
//! out-of-range or unknown index writing nothing, ascending and offset
//! dimensions, partly out-of-range part-selects, and vectors wider than
//! 64 bits. The same body runs on module variables, class properties and the
//! locals of an automatic task; a queue pop in each loop keeps the loop in
//! the interpreter, as in the #1210 test case. Expected `T|` lines were
//! recorded from the reference simulator.

use std::process::Command;

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

/// The declarations every owner kind repeats.
const DECLS: &str = "bit [15:0][63:0] m2;
logic [15:0][63:0] m4;
logic [0:15][7:0] asc;
logic [3:-4][7:0] off;
logic [1023:0] flat;
bit [4095:0][63:0] big;
logic [262143:0] wide4;
logic [63:0] srcq[$];
logic [63:0] chk;
int k;
logic [3:0] kx;
";

const BODY: &str = r#"    // elements of 2-state and 4-state packed arrays
    for (int i = 0; i < 16; i++) begin
      srcq.push_back(64'(i + 1) * 64'h0123_4567_89AB_CDEF);
      m2[i] = srcq.pop_front();
      m4[15 - i] = srcq.size() == 0 ? 64'(i) * 64'hFEDC_BA98_7654_3210 : 64'd0;
    end
    $display("T| a %h", m2[3:0]);
    $display("T| b %h", m4[15:12]);
    // x and z bits: dropped by the 2-state vector, kept by the 4-state one
    m2[3] = 64'hxxxx_zzzz_0000_FFFF;
    m4[2] = 'x;
    m4[5] = 64'hzzzz_0000_xxxx_1234;
    m4[6][15:8] = 8'hzx;
    $display("T| c %h %h %h %h", m2[3], m4[2], m4[5], m4[6]);
    // an index outside the array, or unknown, writes nothing
    k = 20; m2[k] = 64'd1; m4[k] = 64'd1;
    k = -1; m2[k] = 64'd2; m4[k] = 64'd2;
    kx = 'x; m2[kx] = 64'd3; m4[kx] = 64'd3;
    chk = 0;
    for (int i = 0; i < 16; i++) chk = {chk[62:0], chk[63]} ^ m2[i] ^ (i == 2 || i == 5 || i == 6 ? 64'd0 : m4[i]);
    $display("T| d %h %h %h", chk, m4[0], m4[15]);
    // the same value again is no change; a narrower value is extended
    m2[7] = m2[7];
    m4[7] = 16'hBEEF;
    m2[8] = -64'sd2;
    $display("T| e %h %h %h", m2[7], m4[7], m2[8]);
    // ascending and offset dimensions
    for (int i = 0; i < 16; i++) begin srcq.push_back(64'(i)); asc[i] = 8'(srcq.pop_front() * 17); end
    asc[16] = 8'hFF;
    for (int i = -4; i <= 3; i++) begin srcq.push_back(64'(i)); off[i] = 8'(srcq.pop_front()); end
    off[4] = 8'h77; off[-5] = 8'h66;
    $display("T| f %h %h %h %h | %h %h %h", asc, asc[0], asc[15], asc[1], off, off[-4], off[3]);
    // part-selects and bits of a flat vector
    flat = '0;
    for (int i = 0; i < 20; i++) begin
      srcq.push_back(64'(i));
      flat[i * 48 +: 16] = 16'(srcq.pop_front() * 4369);
      flat[1023 - i * 40 -: 8] = 8'(i + 1);
      flat[500 + i] = i[0];
    end
    flat[1030 -: 16] = 16'hFFFF;
    flat[-4 +: 8] = 8'hA5;
    flat[2000] = 1'b1;
    flat[700:693] = 8'bx1z0_1100;
    $display("T| g %h", flat[1023:768]);
    $display("T| h %h", flat[767:512]);
    $display("T| i %h", flat[255:0]);
    // a wide vector: one element among 4096 words
    for (int i = 0; i < 300; i++) begin
      srcq.push_back(64'(i) * 64'h9E37_79B9_7F4A_7C15);
      big[(i * 53) % 4096] = srcq.pop_front();
      wide4[((i * 31) % 32768) * 8 +: 8] = 8'(i);
    end
    chk = 0;
    for (int i = 0; i < 4096; i += 53) chk = {chk[62:0], chk[63]} ^ big[i];
    $display("T| j %h %h %h %h %h", chk, big[53], wide4[31 * 8 +: 8], wide4[(299 * 31) * 8 +: 8], wide4[62 * 8 - 4 +: 16]);
"#;

/// `BODY` without the two stores a class property and a task local do not
/// yet take as the reference simulator does (x/z bits into a 2-state
/// element of a class property; a part-select with a negative base).
const BODY_PORTABLE: &str = r#"    // elements of 2-state and 4-state packed arrays
    for (int i = 0; i < 16; i++) begin
      srcq.push_back(64'(i + 1) * 64'h0123_4567_89AB_CDEF);
      m2[i] = srcq.pop_front();
      m4[15 - i] = srcq.size() == 0 ? 64'(i) * 64'hFEDC_BA98_7654_3210 : 64'd0;
    end
    $display("T| a %h", m2[3:0]);
    $display("T| b %h", m4[15:12]);
    // x and z bits: dropped by the 2-state vector, kept by the 4-state one
    m2[3] = 64'h0000_1111_0000_FFFF;
    m4[2] = 'x;
    m4[5] = 64'hzzzz_0000_xxxx_1234;
    m4[6][15:8] = 8'hzx;
    $display("T| c %h %h %h %h", m2[3], m4[2], m4[5], m4[6]);
    // an index outside the array, or unknown, writes nothing
    k = 20; m2[k] = 64'd1; m4[k] = 64'd1;
    k = -1; m2[k] = 64'd2; m4[k] = 64'd2;
    kx = 'x; m2[kx] = 64'd3; m4[kx] = 64'd3;
    chk = 0;
    for (int i = 0; i < 16; i++) chk = {chk[62:0], chk[63]} ^ m2[i] ^ (i == 2 || i == 5 || i == 6 ? 64'd0 : m4[i]);
    $display("T| d %h %h %h", chk, m4[0], m4[15]);
    // the same value again is no change; a narrower value is extended
    m2[7] = m2[7];
    m4[7] = 16'hBEEF;
    m2[8] = -64'sd2;
    $display("T| e %h %h %h", m2[7], m4[7], m2[8]);
    // ascending and offset dimensions
    for (int i = 0; i < 16; i++) begin srcq.push_back(64'(i)); asc[i] = 8'(srcq.pop_front() * 17); end
    asc[16] = 8'hFF;
    for (int i = -4; i <= 3; i++) begin srcq.push_back(64'(i)); off[i] = 8'(srcq.pop_front()); end
    off[4] = 8'h77; off[-5] = 8'h66;
    $display("T| f %h %h %h %h | %h %h %h", asc, asc[0], asc[15], asc[1], off, off[-4], off[3]);
    // part-selects and bits of a flat vector
    flat = '0;
    for (int i = 0; i < 20; i++) begin
      srcq.push_back(64'(i));
      flat[i * 48 +: 16] = 16'(srcq.pop_front() * 4369);
      flat[1023 - i * 40 -: 8] = 8'(i + 1);
      flat[500 + i] = i[0];
    end
    flat[1030 -: 16] = 16'hFFFF;
    flat[2000] = 1'b1;
    flat[700:693] = 8'bx1z0_1100;
    $display("T| g %h", flat[1023:768]);
    $display("T| h %h", flat[767:512]);
    $display("T| i %h", flat[255:0]);
    // a wide vector: one element among 4096 words
    for (int i = 0; i < 300; i++) begin
      srcq.push_back(64'(i) * 64'h9E37_79B9_7F4A_7C15);
      big[(i * 53) % 4096] = srcq.pop_front();
      wide4[((i * 31) % 32768) * 8 +: 8] = 8'(i);
    end
    chk = 0;
    for (int i = 0; i < 4096; i += 53) chk = {chk[62:0], chk[63]} ^ big[i];
    $display("T| j %h %h %h %h %h", chk, big[53], wide4[31 * 8 +: 8], wide4[(299 * 31) * 8 +: 8], wide4[62 * 8 - 4 +: 16]);
"#;

fn expected() -> Vec<String> {
    [
        "T| a 048d159e26af37bc0369d0369d0369cd02468acf13579bde0123456789abcdef",
        "T| b 0000000000000000fedcba9876543210fdb97530eca86420fc962fc962fc9630",
        "T| c 000000000000ffff xxxxxxxxxxxxxxxx zzzz0000xxxx1234 f5c28f5c28f5zx90",
        "T| d de190a09abc78655 eeeeeeeeeeeeeef0 0000000000000000",
        "T| e 091a2b3c4d5e6f78 000000000000beef fffffffffffffffe",
        "T| f 00112233445566778899aabbccddeeff 00 ff 11 | 03020100fffefdfc fc 03",
        "T| g ff80000000020000000003004443000400003332050000002221000000001110",
        "T| h 00000008ffff0000XZ80eeee000a0000dddd0b000000cccc000000000dbb00aa",
        "T| i 555500000000444400000000333300000000222200000000111100000000000a",
        "T| j 69affcd0090ecc65 9e3779b97f4a7c15 01 2b x02x",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn expected_portable() -> Vec<String> {
    [
        "T| a 048d159e26af37bc0369d0369d0369cd02468acf13579bde0123456789abcdef",
        "T| b 0000000000000000fedcba9876543210fdb97530eca86420fc962fc962fc9630",
        "T| c 000011110000ffff xxxxxxxxxxxxxxxx zzzz0000xxxx1234 f5c28f5c28f5zx90",
        "T| d df081a09abc78655 eeeeeeeeeeeeeef0 0000000000000000",
        "T| e 091a2b3c4d5e6f78 000000000000beef fffffffffffffffe",
        "T| f 00112233445566778899aabbccddeeff 00 ff 11 | 03020100fffefdfc fc 03",
        "T| g ff80000000020000000003004443000400003332050000002221000000001110",
        "T| h 00000008ffff0000XZ80eeee000a0000dddd0b000000cccc000000000dbb00aa",
        "T| i 5555000000004444000000003333000000002222000000001111000000000000",
        "T| j 69affcd0090ecc65 9e3779b97f4a7c15 01 2b x02x",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

#[test]
fn module_variable_selects_store_in_place() {
    let src = format!("module top;\n{DECLS}  initial begin\n{BODY}  end\nendmodule\n");
    assert_eq!(lines(&src), expected());
}

#[test]
fn class_property_selects_store_in_place() {
    let src = format!(
        "class C;\n{DECLS}  task run();\n{BODY_PORTABLE}  endtask\nendclass\n\
         module top;\n  C c;\n  initial begin c = new; c.run(); end\nendmodule\n"
    );
    assert_eq!(lines(&src), expected_portable());
}

#[test]
fn task_local_selects_store_in_place() {
    let src = format!(
        "module top;\n  task automatic run();\n{DECLS}    wide4 = 'x;\n{BODY_PORTABLE}  endtask\n\
         initial run();\nendmodule\n"
    );
    assert_eq!(lines(&src), expected_portable());
}

/// §9.4.2 / §10.6.2 / §10.4.2: `always @(vec)`, `always_comb` and a
/// continuous assignment reading a wide vector see an element store and
/// nothing else; a forced vector ignores one; a nonblocking element store
/// lands in the NBA region. `$dumpvars` is active throughout.
#[test]
fn readers_forces_and_nonblocking_stores_see_element_stores() {
    let dir = std::env::temp_dir().join(format!("xezim_wide_sens_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, r#"// §9.4.2 / §10.6: readers of a wide vector see an element store; a forced vector ignores it.
module top;
  bit [255:0][63:0] buf2;
  logic [255:0][63:0] buf4;
  logic [63:0] srcq[$];
  logic [63:0] tap, tap4;
  int wakes2, wakes4;
  always @(buf2) wakes2++;
  always @(buf4[9]) wakes4++;
  always_comb tap = buf2[100];
  assign tap4 = buf4[200];
  initial begin
    $dumpfile("wide_sens.vcd");
    $dumpvars(0, top);
    #1;
    for (int i = 0; i < 8; i++) begin
      srcq.push_back(64'(i) + 64'd5);
      buf2[100 + (i % 2)] = srcq.pop_front();
      buf4[9] = 64'(i / 2);
      buf4[200] = {32'hx, 32'(i)};
      #1;
    end
    $display("T| a wakes2=%0d wakes4=%0d tap=%h tap4=%h", wakes2, wakes4, tap, tap4);
    buf2[100] = buf2[100];
    #1 $display("T| b wakes2=%0d tap=%h", wakes2, tap);
    force buf2 = '1;
    #1;
    srcq.push_back(64'd77);
    buf2[100] = srcq.pop_front();
    buf2[3] = 64'd0;
    #1 $display("T| c tap=%h el3=%h wakes2=%0d", tap, buf2[3], wakes2);
    release buf2;
    srcq.push_back(64'd78);
    buf2[100] = srcq.pop_front();
    #1 $display("T| d tap=%h el3=%h el101=%h wakes2=%0d", tap, buf2[3], buf2[101], wakes2);
    // nonblocking element stores
    for (int i = 0; i < 4; i++) begin
      srcq.push_back(64'(i) * 64'd1000);
      buf2[i] <= srcq.pop_front();
      buf4[i] <= 64'(i) + 64'd1;
    end
    $display("T| e %h %h", buf2[3], buf4[3]);
    #1 $display("T| f %h %h %h %h", buf2[3], buf4[3], buf2[0], buf4[0]);
    $finish;
  end
endmodule
"#).expect("write design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--no-cache", "--wave"])
        .arg(&path)
        .current_dir(&dir)
        .output()
        .expect("run xezim");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "run failed:\n{text}");
    assert!(dir.join("wide_sens.vcd").exists(), "no VCD written");
    let _ = std::fs::remove_dir_all(&dir);
    let got: Vec<&str> = text.lines().filter(|l| l.starts_with("T|")).collect();
    assert_eq!(
        got,
        [
            "T| a wakes2=8 wakes4=4 tap=000000000000000b tap4=xxxxxxxx00000007",
            "T| b wakes2=8 tap=000000000000000b",
            "T| c tap=ffffffffffffffff el3=ffffffffffffffff wakes2=9",
            "T| d tap=000000000000004e el3=ffffffffffffffff el101=ffffffffffffffff wakes2=10",
            "T| e ffffffffffffffff xxxxxxxxxxxxxxxx",
            "T| f 0000000000000bb8 0000000000000004 0000000000000000 0000000000000001",
        ]
    );
}
