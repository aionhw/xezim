//! `$writememh` / `$writememb` write one word per line, with an `@<address>`
//! line only where `$readmem*` would not otherwise load the next word at its
//! address: before the first word of a range that does not start at the
//! memory's first address, and after a gap (an associative array's missing
//! keys). xezim wrote `@<address> <word>` on every line, which no other
//! simulator does: Verilator writes the data alone, and Icarus Verilog one
//! `// 0x<address>` comment before it. xezim now writes `@<address>` only
//! where the addresses are not contiguous. Tools that compare dumps (NVDLA's `checktest_synthtb.pl`)
//! accept the data with at most one extra leading line. IEEE 1800-2023
//! §21.5 leaves the layout open beyond §21.5.3; each file here reads back
//! with `$readmemh` / `$readmemb` (§21.4) to the values written.
//! `--writemem-every-address` keeps the old layout.

use std::path::{Path, PathBuf};
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

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xezim_wmaddr_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// The issue's reproducer (NVDLA's `dump_mem`): a range of a large memory
/// gets one address line, then the data.
#[test]
fn writememh_range_of_large_memory() {
    let dir = scratch("nvdla");
    let dump = dir.join("dump.hex");
    let src = r#"
module t;
  reg [31:0] memory [0:(2**20)-1];
  reg [31:0] back [0:(2**20)-1];
  initial begin
    memory[20'h10000] = 32'h00000000;
    memory[20'h10001] = 32'h7fff0000;
    memory[20'h10002] = 32'h00007fff;
    memory[20'h10003] = 32'h7fff7fff;
    $writememh("@DUMP@", memory, 20'h10000, 20'h10003);
    $readmemh("@DUMP@", back);
    $display("T|%h %h %h %h %h", back[20'hffff], back[20'h10000], back[20'h10001],
             back[20'h10003], back[20'h10004]);
  end
endmodule
"#
    .replace("@DUMP@", &dump.display().to_string());
    let got = t_lines(&src);
    let text = read(&dump);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(text, "@10000\n00000000\n7fff0000\n00007fff\n7fff7fff\n");
    assert_eq!(got, ["T|xxxxxxxx 00000000 7fff0000 7fff7fff xxxxxxxx"]);
}

/// Unpacked memories: a whole memory is data only, whatever its first
/// index; a range that starts elsewhere, or runs downward, starts with its
/// address; `$writememb` writes binary. Each reads back.
#[test]
fn writemem_unpacked_layouts_read_back() {
    let dir = scratch("unpacked");
    let f = |n: &str| dir.join(n);
    let src = r#"
module t;
  logic [7:0] m [0:3];
  logic [7:0] h [4:7];
  logic [7:0] r [0:3];
  logic [7:0] rh [4:7];
  initial begin
    m = '{8'haa, 8'hbb, 8'hcc, 8'hdd};
    h = '{8'h44, 8'h55, 8'h66, 8'h77};
    $writememh("@WHOLE@", m);
    $writememh("@HIGH@", h);
    $writememh("@RANGE@", m, 1, 2);
    $writememh("@DOWN@", m, 3, 1);
    $writememb("@BIN@", m, 2);
    r = '{default: 8'h00};
    $readmemh("@WHOLE@", r);
    $display("T|whole %h %h %h %h", r[0], r[1], r[2], r[3]);
    rh = '{default: 8'h00};
    $readmemh("@HIGH@", rh);
    $display("T|high %h %h %h %h", rh[4], rh[5], rh[6], rh[7]);
    r = '{default: 8'h00};
    $readmemh("@RANGE@", r);
    $display("T|range %h %h %h %h", r[0], r[1], r[2], r[3]);
    r = '{default: 8'h00};
    $readmemh("@DOWN@", r, 3, 1);
    $display("T|down %h %h %h %h", r[0], r[1], r[2], r[3]);
    r = '{default: 8'h00};
    $readmemb("@BIN@", r);
    $display("T|bin %h %h %h %h", r[0], r[1], r[2], r[3]);
  end
endmodule
"#
    .replace("@WHOLE@", &f("whole.hex").display().to_string())
    .replace("@HIGH@", &f("high.hex").display().to_string())
    .replace("@RANGE@", &f("range.hex").display().to_string())
    .replace("@DOWN@", &f("down.hex").display().to_string())
    .replace("@BIN@", &f("m.bin").display().to_string());
    let got = t_lines(&src);
    let files: Vec<String> = ["whole.hex", "high.hex", "range.hex", "down.hex", "m.bin"]
        .iter()
        .map(|n| read(&f(n)))
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        files,
        [
            "aa\nbb\ncc\ndd\n",
            "44\n55\n66\n77\n",
            "@1\nbb\ncc\n",
            "@3\ndd\ncc\nbb\n",
            "@2\n11001100\n11011101\n",
        ]
    );
    assert_eq!(
        got,
        [
            "T|whole aa bb cc dd",
            "T|high 44 55 66 77",
            "T|range 00 bb cc 00",
            "T|down 00 bb cc dd",
            "T|bin 00 00 cc dd",
        ]
    );
}

/// Clipping a requested range still computes the first address marker from
/// the first word actually written.
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
    assert_eq!(read(&up), "a4\nb5\n");
    assert_eq!(read(&down), "@6\nc6\nb5\n");
    std::fs::remove_dir_all(dir).unwrap();
}

/// An associative array writes an address line where its keys skip; a
/// multidimensional array is addressed by element (§21.4.3).
#[test]
fn writemem_associative_and_multidim_read_back() {
    let dir = scratch("sparse");
    let f = |n: &str| dir.join(n);
    let src = r#"
module t;
  bit [7:0] aa [bit [31:0]];
  bit [7:0] ab [bit [31:0]];
  bit [7:0] z [int];
  logic [7:0] ram [2][1:0];
  logic [7:0] rb [2][1:0];
  initial begin
    aa[32'h10] = 8'h11; aa[32'h11] = 8'h22; aa[32'h12] = 8'h33; aa[32'h20] = 8'h44;
    z[0] = 8'h01; z[1] = 8'h02;
    $writememh("@AA@", aa);
    $writememh("@Z@", z);
    $readmemh("@AA@", ab);
    $display("T|assoc num=%0d %h %h %h", ab.num(), ab[32'h10], ab[32'h12], ab[32'h20]);
    ram = '{'{8'h01, 8'h02}, '{8'h03, 8'h04}};
    $writememh("@RAM@", ram);
    $writememh("@RAM1@", ram, 1, 2);
    rb = '{default: 8'h00};
    $readmemh("@RAM@", rb);
    $display("T|ram %h %h %h %h", rb[0][0], rb[0][1], rb[1][0], rb[1][1]);
    rb = '{default: 8'h00};
    $readmemh("@RAM1@", rb);
    $display("T|ram1 %h %h %h %h", rb[0][0], rb[0][1], rb[1][0], rb[1][1]);
  end
endmodule
"#
    .replace("@AA@", &f("aa.hex").display().to_string())
    .replace("@Z@", &f("z.hex").display().to_string())
    .replace("@RAM@", &f("ram.hex").display().to_string())
    .replace("@RAM1@", &f("ram1.hex").display().to_string());
    let got = t_lines(&src);
    let files: Vec<String> = ["aa.hex", "z.hex", "ram.hex", "ram1.hex"]
        .iter()
        .map(|n| read(&f(n)))
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        files,
        [
            "@10\n11\n22\n33\n@20\n44\n",
            "01\n02\n",
            "02\n01\n04\n03\n",
            "@1\n01\n04\n",
        ]
    );
    assert_eq!(
        got,
        [
            "T|assoc num=4 11 33 44",
            "T|ram 02 01 04 03",
            "T|ram1 00 01 04 00",
        ]
    );
}

/// `--writemem-every-address` writes the old `@<address> <word>` lines.
#[test]
fn writemem_every_address_option() {
    let dir = scratch("every");
    std::fs::write(
        dir.join("t.sv"),
        r#"
module t;
  logic [7:0] m [0:3];
  initial begin
    m = '{8'haa, 8'hbb, 8'hcc, 8'hdd};
    $writememh("w.hex", m);
    $writememh("r.hex", m, 2, 3);
  end
endmodule
"#,
    )
    .unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--no-cache", "--writemem-every-address", "t.sv"])
        .current_dir(&dir)
        .output()
        .expect("failed to run xezim");
    let whole = read(&dir.join("w.hex"));
    let range = read(&dir.join("r.hex"));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(whole, "@0 aa\n@1 bb\n@2 cc\n@3 dd\n");
    assert_eq!(range, "@2 cc\n@3 dd\n");
}

/// An associative `mem` in scope is the memory even when another instance
/// holds a multidimensional `mem`; and keys outside `0..=i64::MAX` keep
/// their value in the one-word-per-line layout: a negative key's address
/// line is its 64-bit two's complement, an unsigned 64-bit key above
/// `i64::MAX` is written as it is, and both read back.
#[test]
fn writemem_associative_name_collision_and_wide_keys() {
    let dir = scratch("assockeys");
    let f = |n: &str| dir.join(n);
    std::fs::write(f("a.hex"), "@10\n11 22\n").unwrap();
    let src = r#"
module other;
  bit [7:0] mem [2][2];
endmodule
module t;
  other o ();
  bit [7:0] mem [int];
  bit [7:0] sm [int];
  bit [7:0] sr [int];
  bit [7:0] um [bit [63:0]];
  bit [7:0] ur [bit [63:0]];
  initial begin
    $readmemh("@A@", mem);
    $writememh("@MW@", mem);
    $display("T|assoc num=%0d [10]=%h [11]=%h other=%h %h", mem.num(), mem[16], mem[17], o.mem[0][0], o.mem[0][1]);
    sm[-2] = 8'h22; sm[-1] = 8'h11; sm[5] = 8'h33;
    $writememh("@SW@", sm);
    $readmemh("@SW@", sr);
    $display("T|signed num=%0d [-2]=%h [-1]=%h [5]=%h", sr.num(), sr[-2], sr[-1], sr[5]);
    um[64'h8000_0000_0000_0000] = 8'h44; um[64'h8000_0000_0000_0001] = 8'h55;
    $writememh("@UW@", um);
    $readmemh("@UW@", ur);
    $display("T|unsigned num=%0d [8000..00]=%h [8000..01]=%h exists0=%0d", ur.num(),
             ur[64'h8000_0000_0000_0000], ur[64'h8000_0000_0000_0001], ur.exists(0));
  end
endmodule
"#
    .replace("@A@", &f("a.hex").display().to_string())
    .replace("@MW@", &f("mw.hex").display().to_string())
    .replace("@SW@", &f("sw.hex").display().to_string())
    .replace("@UW@", &f("uw.hex").display().to_string());
    let got = t_lines(&src);
    let files: Vec<String> = ["mw.hex", "sw.hex", "uw.hex"]
        .iter()
        .map(|n| read(&f(n)))
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        files,
        [
            "@10\n11\n22\n",
            "@fffffffffffffffe\n22\n11\n@5\n33\n",
            "@8000000000000000\n44\n55\n",
        ]
    );
    assert_eq!(
        got,
        [
            "T|assoc num=2 [10]=11 [11]=22 other=00 00",
            "T|signed num=3 [-2]=22 [-1]=11 [5]=33",
            "T|unsigned num=2 [8000..00]=44 [8000..01]=55 exists0=0",
        ]
    );
}

/// A dump that reaches the largest or the smallest key of a signed 128-bit
/// index type ends there instead of overflowing the next address: an
/// ascending and a descending dump of both endpoint keys are written and
/// read back.
#[test]
fn writemem_associative_endpoint_keys_of_a_128_bit_index() {
    let dir = scratch("endkeys");
    let f = |n: &str| dir.join(n);
    let src = r#"
module t;
  bit [7:0] em [bit signed [127:0]];
  bit [7:0] ea [bit signed [127:0]];
  bit [7:0] ed [bit signed [127:0]];
  bit signed [127:0] mx = 128'sh7fff_ffff_ffff_ffff_ffff_ffff_ffff_ffff;
  bit signed [127:0] mn = 128'sh8000_0000_0000_0000_0000_0000_0000_0000;
  initial begin
    em[mx] = 8'h11;
    em[mn] = 8'h22;
    $writememh("@A@", em);
    $writememh("@D@", em, mx, mn);
    $readmemh("@A@", ea);
    $readmemh("@D@", ed, mx, mn);
    $display("T|ascending num=%0d %h %h", ea.num(), ea[mx], ea[mn]);
    $display("T|descending num=%0d %h %h", ed.num(), ed[mx], ed[mn]);
  end
endmodule
"#
    .replace("@A@", &f("a.hex").display().to_string())
    .replace("@D@", &f("d.hex").display().to_string());
    let got = t_lines(&src);
    let files: Vec<String> = ["a.hex", "d.hex"].iter().map(|n| read(&f(n))).collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        files,
        [
            "@80000000000000000000000000000000\n22\n@7fffffffffffffffffffffffffffffff\n11\n",
            "@7fffffffffffffffffffffffffffffff\n11\n@80000000000000000000000000000000\n22\n",
        ]
    );
    assert_eq!(got, ["T|ascending num=2 11 22", "T|descending num=2 11 22"]);
}
