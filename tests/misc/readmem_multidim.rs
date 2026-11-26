//! `$readmemh` / `$readmemb` into a multidimensional unpacked array, and
//! `$writememh` / `$writememb` out of one, reported `'ram' does not name an
//! unpacked memory` and did nothing. IEEE 1800-2023 §21.4.3 lays the file
//! out in row-major order: the rightmost dimension varies fastest and every
//! dimension runs from its low index to its high one, whichever way it is
//! declared. The memory may also be a partially indexed array (`ram[2]`).
//! §21.5 writes the same layout.
//!
//! Addresses (the start and finish arguments and the file's `@` entries)
//! count elements in that order, from the low index of the leftmost
//! dimension, as Verilator and Icarus do; the LRM
//! text has them name words of the leftmost dimension instead (see
//! `NdMemory` in `src/compiler/simulator/file_io.rs`). Verilator and Icarus
//! give the same values as xezim for every case here except: both reject a
//! partially indexed target, Verilator loads a descending start/finish range
//! upward, and Icarus counts the addresses of an array whose leftmost
//! dimension starts above 0 from 0.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|") || l.contains("[xezim]"))
        .collect()
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("xezim_rmnd_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The issue's reproducer: a byte-wide `ram [4][3:0]` loaded with start and
/// finish addresses that count bytes (Caliptra's `caliptra_sram`).
#[test]
fn readmemh_2d_reproducer() {
    let dir = scratch("repro");
    let hex = dir.join("mem.hex");
    std::fs::write(&hex, "@0\n11 22 33 44\n55 66 77 88\n").unwrap();
    let src = r#"
module t;
  logic [7:0] ram [4][3:0];
  initial begin
    foreach (ram[i,j]) ram[i][j] = 8'h0;
    $readmemh("@HEX@", ram, 0, 7);
    $display("T|ram[0] = %h %h %h %h", ram[0][3], ram[0][2], ram[0][1], ram[0][0]);
    $display("T|ram[1] = %h %h %h %h", ram[1][3], ram[1][2], ram[1][1], ram[1][0]);
    $display("T|ram[2] = %h %h %h %h", ram[2][3], ram[2][2], ram[2][1], ram[2][0]);
  end
endmodule
"#
    .replace("@HEX@", &hex.display().to_string());
    let got = t_lines(&src);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        got,
        [
            "T|ram[0] = 44 33 22 11",
            "T|ram[1] = 88 77 66 55",
            "T|ram[2] = 00 00 00 00",
        ]
    );
}

/// Whole-array loads: 2-D, 3-D and descending dimensions fill in row-major
/// order with each dimension low to high; a short file leaves the rest
/// unchanged; `$readmemb` and a 2-state element (x digits read as 0, §21.4.2)
/// take the same path.
#[test]
fn readmem_multidim_layout() {
    let dir = scratch("layout");
    let hex = dir.join("a.hex");
    let bin = dir.join("b.bin");
    std::fs::write(&hex, "11 22 33 44\n55 66 77 88\n99 aa bb cc\n").unwrap();
    std::fs::write(&bin, "00000001 // one\n00000010\n0000001x\n").unwrap();
    let src = r#"
module t;
  logic [7:0] ram [4][3:0];
  logic [7:0] r3 [0:1][1:0][0:2];
  logic [7:0] dsc [3:0][1:0];
  bit   [7:0] b2 [2][2];
  initial begin
    foreach (ram[i,j]) ram[i][j] = 8'h0;
    $readmemh("@HEX@", ram);
    $display("T|ram %h %h %h %h | %h %h %h %h | %h %h %h %h | %h %h %h %h",
      ram[0][0], ram[0][1], ram[0][2], ram[0][3], ram[1][0], ram[1][1], ram[1][2], ram[1][3],
      ram[2][0], ram[2][1], ram[2][2], ram[2][3], ram[3][0], ram[3][1], ram[3][2], ram[3][3]);
    $readmemh("@HEX@", r3);
    $display("T|r3 %h %h %h %h %h %h | %h %h %h %h %h %h",
      r3[0][0][0], r3[0][0][1], r3[0][0][2], r3[0][1][0], r3[0][1][1], r3[0][1][2],
      r3[1][0][0], r3[1][0][1], r3[1][0][2], r3[1][1][0], r3[1][1][1], r3[1][1][2]);
    $readmemh("@HEX@", dsc);
    $display("T|dsc %h %h %h %h %h %h %h %h",
      dsc[0][0], dsc[0][1], dsc[1][0], dsc[1][1], dsc[2][0], dsc[2][1], dsc[3][0], dsc[3][1]);
    $readmemb("@BIN@", b2);
    $display("T|b2 %h %h %h %h", b2[0][0], b2[0][1], b2[1][0], b2[1][1]);
  end
endmodule
"#
    .replace("@HEX@", &hex.display().to_string())
    .replace("@BIN@", &bin.display().to_string());
    let got = t_lines(&src);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        got,
        [
            "T|ram 11 22 33 44 | 55 66 77 88 | 99 aa bb cc | 00 00 00 00",
            "T|r3 11 22 33 44 55 66 | 77 88 99 aa bb cc",
            "T|dsc 11 22 33 44 55 66 77 88",
            "T|b2 01 02 02 00",
        ]
    );
}

/// Addressing: the file's `@` entries, a start address, start and finish
/// addresses in both directions, and an array whose leftmost dimension does
/// not start at 0 (addresses count from its low index).
#[test]
fn readmem_multidim_addresses() {
    let dir = scratch("addr");
    let hex = dir.join("a.hex");
    let at = dir.join("at.hex");
    let at3 = dir.join("at3.hex");
    std::fs::write(&hex, "11 22 33 44\n55 66 77 88\n99 aa bb cc\n").unwrap();
    std::fs::write(&at, "@2\nd0 d1 d2 d3\n@0 e0 e1\n").unwrap();
    std::fs::write(&at3, "@3 a3 b3\n@5 c5\n").unwrap();
    let src = r#"
module t;
  logic [7:0] ram [4][3:0];
  logic [7:0] m [2:3][4:5];
  task clr; foreach (ram[i,j]) ram[i][j] = 8'h0; endtask
  task show(string tag);
    $display("T|%s %h %h %h %h | %h %h %h %h | %h %h %h %h | %h %h %h %h", tag,
      ram[0][0], ram[0][1], ram[0][2], ram[0][3], ram[1][0], ram[1][1], ram[1][2], ram[1][3],
      ram[2][0], ram[2][1], ram[2][2], ram[2][3], ram[3][0], ram[3][1], ram[3][2], ram[3][3]);
  endtask
  initial begin
    clr(); $readmemh("@AT@", ram); show("file@");
    clr(); $readmemh("@HEX@", ram, 2); show("start2");
    clr(); $readmemh("@HEX@", ram, 6, 9); show("6to9");
    clr(); $readmemh("@HEX@", ram, 3, 2); show("3to2");
    foreach (m[i,j]) m[i][j] = 8'h0;
    $readmemh("@AT3@", m);
    $display("T|m %h %h %h %h", m[2][4], m[2][5], m[3][4], m[3][5]);
  end
endmodule
"#
    .replace("@HEX@", &hex.display().to_string())
    .replace("@AT@", &at.display().to_string())
    .replace("@AT3@", &at3.display().to_string());
    let got = t_lines(&src);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        got,
        [
            "T|file@ e0 e1 d0 d1 | d2 d3 00 00 | 00 00 00 00 | 00 00 00 00",
            "T|start2 00 00 11 22 | 33 44 55 66 | 77 88 99 aa | bb cc 00 00",
            "T|6to9 00 00 00 00 | 00 00 11 22 | 33 44 00 00 | 00 00 00 00",
            "T|3to2 00 00 22 11 | 00 00 00 00 | 00 00 00 00 | 00 00 00 00",
            "T|m 00 a3 b3 c5",
        ]
    );
}

/// A partially indexed array names the subarray it selects (§21.4.3); its
/// addresses are the subarray's own. Hierarchical targets resolve as for a
/// one-dimensional memory, from a task too, and `$writememh` / `$writememb`
/// of a whole array, a range and a subarray read back with `$readmem*`.
#[test]
fn readmem_writemem_multidim_subarrays_and_round_trip() {
    let dir = scratch("sub");
    let hex = dir.join("a.hex");
    let wh = dir.join("w.hex");
    let wr = dir.join("wr.hex");
    let wb = dir.join("w.bin");
    std::fs::write(&hex, "11 22 33 44\n55 66 77 88\n99 aa bb cc\n").unwrap();
    let src = r#"
module mem_m;
  bit [7:0] ram [4][3:0];
endmodule
module t;
  mem_m u ();
  logic [7:0] r3 [0:1][1:0][0:2];
  logic [7:0] cp [4][3:0];
  logic [7:0] cp2 [4][3:0];
  task tload;
    $readmemh("@HEX@", u.ram, 4, 7);
  endtask
  initial begin
    $readmemh("@HEX@", u.ram[2]);
    $display("T|row2 %h %h %h %h | %h", u.ram[2][0], u.ram[2][1], u.ram[2][2], u.ram[2][3], u.ram[3][0]);
    foreach (r3[i,j,k]) r3[i][j][k] = 8'h0;
    $readmemh("@HEX@", r3[1]);
    $readmemh("@HEX@", r3[0][1], 1);
    $display("T|r3 %h %h %h %h %h %h | %h %h %h %h %h %h",
      r3[0][0][0], r3[0][0][1], r3[0][0][2], r3[0][1][0], r3[0][1][1], r3[0][1][2],
      r3[1][0][0], r3[1][0][1], r3[1][0][2], r3[1][1][0], r3[1][1][1], r3[1][1][2]);
    tload();
    $display("T|task %h %h %h %h", u.ram[1][0], u.ram[1][1], u.ram[1][2], u.ram[1][3]);
    $writememh("@WH@", u.ram);
    $writememb("@WB@", u.ram[1]);
    $writememh("@WR@", u.ram, 9, 6);
    foreach (cp[i,j]) cp[i][j] = 8'hee;
    $readmemh("@WH@", cp);
    $display("T|cp %h %h %h %h | %h %h %h %h | %h %h %h %h | %h %h %h %h",
      cp[0][0], cp[0][1], cp[0][2], cp[0][3], cp[1][0], cp[1][1], cp[1][2], cp[1][3],
      cp[2][0], cp[2][1], cp[2][2], cp[2][3], cp[3][0], cp[3][1], cp[3][2], cp[3][3]);
    foreach (cp2[i,j]) cp2[i][j] = 8'hee;
    $readmemb("@WB@", cp2[3]);
    $readmemh("@WR@", cp2, 9, 6);
    $display("T|cp2 %h %h %h %h | %h %h %h %h | %h %h %h %h",
      cp2[1][0], cp2[1][1], cp2[1][2], cp2[1][3], cp2[2][0], cp2[2][1], cp2[2][2], cp2[2][3],
      cp2[3][0], cp2[3][1], cp2[3][2], cp2[3][3]);
  end
endmodule
"#
    .replace("@HEX@", &hex.display().to_string())
    .replace("@WH@", &wh.display().to_string())
    .replace("@WR@", &wr.display().to_string())
    .replace("@WB@", &wb.display().to_string());
    let got = t_lines(&src);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        got,
        [
            "T|row2 11 22 33 44 | 00",
            "T|r3 00 00 00 00 11 22 | 11 22 33 44 55 66",
            "T|task 11 22 33 44",
            "T|cp 00 00 00 00 | 11 22 33 44 | 11 22 33 44 | 00 00 00 00",
            "T|cp2 ee ee 33 44 | 11 22 ee ee | 11 22 33 44",
        ]
    );
}

/// A one-dimensional memory in scope keeps its own path when another
/// instance holds a multidimensional array of the same name, and a target
/// that is an element, not an array, is still reported.
#[test]
fn readmem_multidim_does_not_capture_other_names() {
    let dir = scratch("names");
    let hex = dir.join("a.hex");
    std::fs::write(&hex, "11 22 33 44\n").unwrap();
    let src = r#"
module other;
  bit [7:0] mem [2][2];
endmodule
module t;
  other o ();
  logic [7:0] mem [0:3];
  logic [7:0] two [2][2];
  initial begin
    $readmemh("@HEX@", mem);
    $display("T|mem %h %h %h %h o.mem %h", mem[0], mem[1], mem[2], mem[3], o.mem[0][0]);
    $readmemh("@HEX@", two[1][0]);
  end
endmodule
"#
    .replace("@HEX@", &hex.display().to_string());
    let got = t_lines(&src);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(got[0], "T|mem 11 22 33 44 o.mem 00");
    assert!(
        got[1].contains("$readmemh") && got[1].contains("does not name an unpacked memory"),
        "{got:?}"
    );
}

/// The array a name reaches first is the memory, whatever its kind: in a
/// class method, the class's one-dimensional `mem` property, not the
/// module's two-dimensional `mem`. An element that is a packed struct is a
/// vector word (§21.4.1), loaded and written like any other; an unpacked
/// struct is not. A range of a multidimensional array between negative
/// addresses is written data only (§21.5.3) and read back to the same
/// addresses.
#[test]
fn readmem_multidim_name_kind_packed_struct_and_negative_addresses() {
    let dir = scratch("kinds");
    let f = |n: &str| dir.join(n).display().to_string();
    std::fs::write(dir.join("a.hex"), "11 22 33 44\n").unwrap();
    let src = r#"
module t;
  typedef struct packed { logic [3:0] hi; logic [3:0] lo; } pair_t;
  typedef struct { logic [3:0] hi; logic [3:0] lo; } upair_t;
  logic [7:0] mem [2][2];
  pair_t ps [2][2];
  pair_t pb [2][2];
  upair_t us [2][2];
  logic [7:0] ng [-2:-1][2];
  logic [7:0] nb [-2:-1][2];
  class C;
    logic [7:0] mem [0:3];
    function void ld(string fn);
      $readmemh(fn, mem);
    endfunction
  endclass
  initial begin
    C c = new;
    foreach (mem[i, j]) mem[i][j] = 8'h00;
    c.ld("@A@");
    $display("T|class %h %h %h %h module %h", c.mem[0], c.mem[1], c.mem[2], c.mem[3], mem[0][0]);
    $readmemh("@A@", ps);
    $display("T|packed %h %h %h %h hi=%h lo=%h", ps[0][0], ps[0][1], ps[1][0], ps[1][1], ps[1][1].hi, ps[1][1].lo);
    $writememh("@PW@", ps);
    $readmemh("@PW@", pb);
    $display("T|packed back %h %h %h %h", pb[0][0], pb[0][1], pb[1][0], pb[1][1]);
    $readmemh("@A@", us);
    ng = '{'{8'h01, 8'h02}, '{8'h03, 8'h04}};
    foreach (nb[i, j]) nb[i][j] = 8'h00;
    $writememh("@NW@", ng, -1, 0);
    $readmemh("@NW@", nb, -1, 0);
    $display("T|negative %h %h %h %h", nb[-2][0], nb[-2][1], nb[-1][0], nb[-1][1]);
  end
endmodule
"#
    .replace("@A@", &f("a.hex"))
    .replace("@PW@", &f("pw.hex"))
    .replace("@NW@", &f("nw.hex"));
    let got = t_lines(&src);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(got.len(), 5, "{got:?}");
    assert_eq!(got[0], "T|class 11 22 33 44 module 00");
    assert_eq!(got[1], "T|packed 11 22 33 44 hi=4 lo=4");
    assert_eq!(got[2], "T|packed back 11 22 33 44");
    assert!(
        got[3].contains("$readmemh") && got[3].contains("does not name an unpacked memory"),
        "{got:?}"
    );
    assert_eq!(got[4], "T|negative 00 02 03 00");
}

/// A multidimensional class property is a memory too, through a handle
/// and as a bare name in a method. Its elements live in the object's
/// store rather than in signal cells, so a load must reach them and a
/// write must read them back, not write zeros. A 2-state (`bit`) property
/// drops the X/Z digits of a loaded word (§6.11.1).
#[test]
fn readmem_writemem_multidim_class_property() {
    let dir = scratch("classprop");
    let f = |n: &str| dir.join(n).display().to_string();
    std::fs::write(dir.join("a.hex"), "11 22 33 44\n").unwrap();
    std::fs::write(dir.join("b.hex"), "55 66 77 88\n").unwrap();
    std::fs::write(dir.join("c.hex"), "xx 1z 33 44\n").unwrap();
    let src = r#"
class C;
  logic [7:0] mem [2][2];
  bit [7:0] bm [2][2];
  function void ld(string fn);
    $readmemh(fn, mem);
  endfunction
  function void wr(string fn);
    $writememh(fn, mem);
  endfunction
endclass
module t;
  logic [7:0] back [2][2];
  logic [7:0] back2 [2][2];
  logic [7:0] back3 [2][2];
  initial begin
    C c = new;
    $readmemh("@A@", c.mem);
    $display("T|load %h %h %h %h", c.mem[0][0], c.mem[0][1], c.mem[1][0], c.mem[1][1]);
    c.mem[1][1] = 8'h5a;
    $writememh("@W@", c.mem);
    $readmemh("@W@", back);
    $display("T|back %h %h %h %h", back[0][0], back[0][1], back[1][0], back[1][1]);
    c.ld("@B@");
    $display("T|method %h %h %h %h", c.mem[0][0], c.mem[0][1], c.mem[1][0], c.mem[1][1]);
    c.wr("@W2@");
    $readmemh("@W2@", back2);
    $display("T|method back %h %h %h %h", back2[0][0], back2[0][1], back2[1][0], back2[1][1]);
    $readmemh("@C@", c.bm);
    $display("T|bit %h %h %h %h", c.bm[0][0], c.bm[0][1], c.bm[1][0], c.bm[1][1]);
    $writememh("@W3@", c.bm);
    $readmemh("@W3@", back3);
    $display("T|bit back %h %h %h %h", back3[0][0], back3[0][1], back3[1][0], back3[1][1]);
  end
endmodule
"#
    .replace("@A@", &f("a.hex"))
    .replace("@B@", &f("b.hex"))
    .replace("@C@", &f("c.hex"))
    .replace("@W3@", &f("w3.hex"))
    .replace("@W@", &f("w.hex"))
    .replace("@W2@", &f("w2.hex"));
    let got = t_lines(&src);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        got,
        [
            "T|load 11 22 33 44",
            "T|back 11 22 33 5a",
            "T|method 55 66 77 88",
            "T|method back 55 66 77 88",
            "T|bit 00 10 33 44",
            "T|bit back 00 10 33 44",
        ]
    );
}

#[test]
fn sparse_memory_can_restart_after_the_range_endpoint() {
    let dir = scratch("restart");
    let hex = dir.join("mem.hex");
    std::fs::write(&hex, "@3 aa ee @0 bb\n").unwrap();
    let src = r#"
module t;
  bit [7:0] m[2][2], a[int], fixed_mem[4];
  initial begin
    $readmemh("@HEX@", m, 0, 3);
    $readmemh("@HEX@", a, 0, 3);
    $readmemh("@HEX@", fixed_mem, 0, 3);
    $display("T|%h %h %h %h %h %h", m[1][1], m[0][0], a[3], a[0], fixed_mem[3], fixed_mem[0]);
  end
endmodule
"#
    .replace("@HEX@", &hex.display().to_string());
    let got = t_lines(&src);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(got, ["T|aa bb aa bb aa bb"]);
}
