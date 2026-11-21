//! `$writememh` / `$writememb` lines (IEEE 1800-2023 §21.5.3): an unpacked
//! or dynamic array is written one word per line with no `@` address, also
//! for a range that does not start at the memory's lowest address; an
//! associative array writes `@<address> <word>` for every word. xezim used
//! to write an address on every line of every memory, so tools comparing a
//! dump with a data-only golden file rejected it. A dynamic array writes the
//! elements its size counts. Each file reads back with `$readmem*` (§21.4)
//! given the same addresses. Expected values from the reference simulator,
//! which writes the same data lines (after its own header comments).

use std::path::{Path, PathBuf};
use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xezim_wmdata_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Each `@name@` of the source becomes the path of `name` in `dir`.
fn with_paths(src: &str, dir: &Path, names: &[&str]) -> String {
    names.iter().fold(src.to_string(), |s, n| {
        s.replace(&format!("@{n}@"), &dir.join(n).display().to_string())
    })
}

fn files(dir: &Path, names: &[&str]) -> Vec<String> {
    names
        .iter()
        .map(|n| std::fs::read_to_string(dir.join(n)).unwrap_or_else(|e| panic!("{n}: {e}")))
        .collect()
}

/// The issue's reproducer: a range of a 2^20-word memory is the four words
/// alone, which a load with the same start address puts back.
#[test]
fn writememh_range_of_large_memory_is_data_only() {
    let dir = scratch("range");
    let names = ["dump.hex"];
    let src = with_paths(
        r#"
module t;
  reg [31:0] memory [0:(2**20)-1];
  reg [31:0] back [0:(2**20)-1];
  initial begin
    memory[20'h10000] = 32'h00000000;
    memory[20'h10001] = 32'h7fff0000;
    memory[20'h10002] = 32'h00007fff;
    memory[20'h10003] = 32'h7fff7fff;
    $writememh("@dump.hex@", memory, 20'h10000, 20'h10003);
    $readmemh("@dump.hex@", back, 20'h10000);
    $display("T|%h %h %h %h %h", back[20'hffff], back[20'h10000], back[20'h10001],
             back[20'h10003], back[20'h10004]);
  end
endmodule
"#,
        &dir,
        &names,
    );
    let got = t_lines(&src);
    let text = files(&dir, &names);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(text, ["00000000\n7fff0000\n00007fff\n7fff7fff\n"]);
    assert_eq!(got, ["T|xxxxxxxx 00000000 7fff0000 7fff7fff xxxxxxxx"]);
}

/// Whole memories whose lowest index is 0 or 4 or that are declared
/// descending (lowest address first), an ascending range, a descending
/// range, a start address alone, and `$writememb`: data only, and each reads
/// back to the words written.
#[test]
fn writemem_unpacked_layouts_read_back() {
    let dir = scratch("unpacked");
    let names = [
        "whole.hex",
        "high.hex",
        "desc.hex",
        "range.hex",
        "down.hex",
        "from.hex",
        "m.bin",
    ];
    let src = with_paths(
        r#"
module t;
  logic [7:0] m [0:3];
  logic [7:0] h [4:7];
  logic [7:0] d [3:0];
  logic [7:0] r [0:3];
  logic [7:0] rh [4:7];
  logic [7:0] rd [3:0];
  initial begin
    m = '{8'haa, 8'hbb, 8'hcc, 8'hdd};
    h = '{8'h44, 8'h55, 8'h66, 8'h77};
    d = '{8'h33, 8'h22, 8'h11, 8'h00};
    $writememh("@whole.hex@", m);
    $writememh("@high.hex@", h);
    $writememh("@desc.hex@", d);
    $writememh("@range.hex@", m, 1, 2);
    $writememh("@down.hex@", m, 3, 1);
    $writememh("@from.hex@", h, 6);
    $writememb("@m.bin@", m, 2);
    r = '{default: 8'h00};
    $readmemh("@whole.hex@", r);
    $display("T|whole %h %h %h %h", r[0], r[1], r[2], r[3]);
    rh = '{default: 8'h00};
    $readmemh("@high.hex@", rh);
    $display("T|high %h %h %h %h", rh[4], rh[5], rh[6], rh[7]);
    rd = '{default: 8'h00};
    $readmemh("@desc.hex@", rd);
    $display("T|desc %h %h %h %h", rd[0], rd[1], rd[2], rd[3]);
    r = '{default: 8'h00};
    $readmemh("@range.hex@", r, 1, 2);
    $display("T|range %h %h %h %h", r[0], r[1], r[2], r[3]);
    r = '{default: 8'h00};
    $readmemh("@down.hex@", r, 3, 1);
    $display("T|down %h %h %h %h", r[0], r[1], r[2], r[3]);
    rh = '{default: 8'h00};
    $readmemh("@from.hex@", rh, 6);
    $display("T|from %h %h %h %h", rh[4], rh[5], rh[6], rh[7]);
    r = '{default: 8'h00};
    $readmemb("@m.bin@", r, 2);
    $display("T|bin %h %h %h %h", r[0], r[1], r[2], r[3]);
  end
endmodule
"#,
        &dir,
        &names,
    );
    let got = t_lines(&src);
    let text = files(&dir, &names);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        text,
        [
            "aa\nbb\ncc\ndd\n",
            "44\n55\n66\n77\n",
            "00\n11\n22\n33\n",
            "bb\ncc\n",
            "dd\ncc\nbb\n",
            "66\n77\n",
            "11001100\n11011101\n",
        ]
    );
    assert_eq!(
        got,
        [
            "T|whole aa bb cc dd",
            "T|high 44 55 66 77",
            "T|desc 00 11 22 33",
            "T|range 00 bb cc 00",
            "T|down 00 bb cc dd",
            "T|from 00 00 66 77",
            "T|bin 00 00 cc dd",
        ]
    );
}

/// A dynamic array writes its elements (not the storage set aside for it),
/// an empty one writes an empty file, and a queue writes its elements, all
/// data only. A multidimensional array and a subarray of one are data only
/// too (§21.4.3 order). An associative array writes an address on every
/// word, also when its keys run from 0 without a gap.
#[test]
fn writemem_dynamic_multidim_and_associative() {
    let dir = scratch("dynmd");
    let names = [
        "dyn.hex",
        "dyn12.hex",
        "none.hex",
        "q.hex",
        "ram.hex",
        "row.hex",
        "aa.hex",
        "z.hex",
    ];
    let src = with_paths(
        r#"
module t;
  logic [7:0] dyn [];
  logic [7:0] none [];
  logic [7:0] q [$];
  logic [7:0] ram [2][1:0];
  logic [7:0] rb [2][1:0];
  bit [7:0] aa [bit [31:0]];
  bit [7:0] ab [bit [31:0]];
  bit [7:0] z [int];
  initial begin
    dyn = new[4];
    foreach (dyn[k]) dyn[k] = 8'ha0 + k;
    q = '{8'h01, 8'h02, 8'h03};
    $writememh("@dyn.hex@", dyn);
    $writememh("@dyn12.hex@", dyn, 1, 2);
    $writememh("@none.hex@", none);
    $writememh("@q.hex@", q);
    ram = '{'{8'h01, 8'h02}, '{8'h03, 8'h04}};
    $writememh("@ram.hex@", ram);
    $writememh("@row.hex@", ram[1]);
    aa[32'h10] = 8'h11; aa[32'h11] = 8'h22; aa[32'h20] = 8'h44;
    z[0] = 8'h01; z[1] = 8'h02;
    $writememh("@aa.hex@", aa);
    $writememh("@z.hex@", z);
    rb = '{default: 8'h00};
    $readmemh("@ram.hex@", rb);
    $display("T|ram %h %h %h %h", rb[0][0], rb[0][1], rb[1][0], rb[1][1]);
    $readmemh("@aa.hex@", ab);
    $display("T|assoc num=%0d %h %h %h", ab.num(), ab[32'h10], ab[32'h11], ab[32'h20]);
  end
endmodule
"#,
        &dir,
        &names,
    );
    let got = t_lines(&src);
    let text = files(&dir, &names);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        text,
        [
            "a0\na1\na2\na3\n",
            "a1\na2\n",
            "",
            "01\n02\n03\n",
            "02\n01\n04\n03\n",
            "04\n03\n",
            "@10 11\n@11 22\n@20 44\n",
            "@0 01\n@1 02\n",
        ]
    );
    assert_eq!(got, ["T|ram 02 01 04 03", "T|assoc num=3 11 22 44"]);
}

/// A fixed unpacked class property, named bare in a method, writes its
/// words as `$readmem*` loads them (by name), not zeros; reading the file
/// back into another property restores them. The reference simulator does
/// not resolve a class property as the memory of `$writememh` (it writes
/// only its header); §21.5 takes any unpacked array.
#[test]
fn writemem_of_a_class_property() {
    let dir = scratch("prop");
    let names = ["p.hex"];
    let src = with_paths(
        r#"
module t;
  class C;
    logic [7:0] p [2:5];
    logic [7:0] b [2:5];
    function void dump(string f);
      foreach (p[i]) p[i] = 8'h50 + i;
      $writememh(f, p);
      $readmemh(f, b);
    endfunction
  endclass
  initial begin
    C o = new;
    o.dump("@p.hex@");
    $display("T|%h %h %h %h", o.b[2], o.b[3], o.b[4], o.b[5]);
  end
endmodule
"#,
        &dir,
        &names,
    );
    let got = t_lines(&src);
    let text = files(&dir, &names);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(text, ["52\n53\n54\n55\n"]);
    assert_eq!(got, ["T|52 53 54 55"]);
}

/// Out-of-range endpoints clip to the declared storage in either direction.
#[test]
fn writemem_clips_ranges_in_both_directions() {
    let dir = scratch("clipped");
    let up = dir.join("up.hex");
    let down = dir.join("down.hex");
    let sv = r#"
module t;
  logic [7:0] mem [4:6];
  initial begin
    mem = '{8'ha4, 8'hb5, 8'hc6};
    $writememh("@UP@", mem, -10000, 5);
    $writememh("@DOWN@", mem, 10000, 5);
  end
endmodule
"#
    .replace("@UP@", up.to_str().unwrap())
    .replace("@DOWN@", down.to_str().unwrap());
    t_lines(&sv);
    assert_eq!(std::fs::read_to_string(&up).unwrap(), "a4\nb5\n");
    assert_eq!(std::fs::read_to_string(&down).unwrap(), "c6\nb5\n");
    std::fs::remove_dir_all(dir).unwrap();
}
