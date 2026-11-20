//! `$readmemh`/`$readmemb` into an associative array loaded
//! nothing and reported `'mem' does not name an unpacked memory`. IEEE
//! 1800-2023 §21.4 allows the memory to be an associative array whose index
//! type is integral; each address is used as the key. §21.5 lets
//! `$writememh`/`$writememb` dump one: the elements that exist, in index
//! order. The VeeR EH1/EH2/EL2 testbenches load their program this way
//! (`bit [7:0] mem [bit [31:0]]` in an instance, `$readmemh(.., lmem.mem)`).
//! Verilator 5.046 loads the reproducer.

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
    let dir = std::env::temp_dir().join(format!("xezim_rmaa_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The issue's reproducer: a local associative array, one in another
/// instance (`lmem.mem`), and an unpacked memory alongside.
#[test]
fn readmem_into_associative_array_reproducer() {
    let dir = scratch("repro");
    let hex = dir.join("p.hex");
    std::fs::write(&hex, "@10\n11 22 33\n@20\n44\n").unwrap();
    let src = r#"
module m; bit [7:0] mem [bit [31:0]]; endmodule
module u; bit [7:0] arr [0:63]; endmodule
module t;
  m lmem ();
  u umem ();
  bit [7:0] mem [bit [31:0]];
  initial begin
    $readmemh("@HEX@", mem);
    $readmemh("@HEX@", lmem.mem);
    $readmemh("@HEX@", umem.arr);
    if (mem.num() == 4 && mem[32'h10] == 8'h11 && mem[32'h12] == 8'h33 && mem[32'h20] == 8'h44)
      $display("T|PASS local assoc");
    else $display("T|FAIL local assoc num=%0d", mem.num());
    if (lmem.mem.num() == 4 && lmem.mem[32'h20] == 8'h44) $display("T|PASS hier assoc");
    else $display("T|FAIL hier assoc num=%0d", lmem.mem.num());
    if (umem.arr[16] == 8'h11 && umem.arr[32] == 8'h44) $display("T|PASS hier unpacked");
    else $display("T|FAIL hier unpacked");
  end
endmodule
"#
    .replace("@HEX@", &hex.display().to_string());
    let got = t_lines(&src);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        got,
        [
            "T|PASS local assoc",
            "T|PASS hier assoc",
            "T|PASS hier unpacked"
        ]
    );
}

/// §21.4 addressing: the file's own addresses; address 0 when the file has
/// none (keys already present are kept); a start address; start and finish
/// addresses, ascending and descending; `$readmemb`; a signed and an 8-bit
/// index type; a member-access target inside a task. §21.5: `$writememh`
/// round-trips, whole and over an address range.
#[test]
fn readmem_writemem_associative_array_forms() {
    let dir = scratch("forms");
    let a = dir.join("a.hex");
    let b = dir.join("b.hex");
    let c = dir.join("c.mem");
    let w = dir.join("w.hex");
    let w2 = dir.join("w2.hex");
    std::fs::write(&a, "@10\n11 22 33\n@20\n44\n").unwrap();
    std::fs::write(&b, "aa\nbb\ncc\n").unwrap();
    std::fs::write(&c, "@3\n1010\n0101\n").unwrap();
    let src = r#"
module m;
  bit [7:0] mem [bit [31:0]];
endmodule
module top;
  m lmem ();
  bit   [7:0] la [bit [31:0]];
  logic [7:0] nb [int];
  bit   [7:0] st [bit [31:0]];
  bit   [7:0] se [bit [31:0]];
  bit   [7:0] ds [bit [31:0]];
  bit   [3:0] bb [byte unsigned];
  bit   [7:0] rb [bit [31:0]];
  bit   [7:0] rr [bit [31:0]];
  task load;
    $readmemh("@A@", lmem.mem);
  endtask
  initial begin
    $readmemh("@A@", la);
    $display("T|local num=%0d [10]=%h [11]=%h [12]=%h [20]=%h", la.num(), la[32'h10], la[32'h11], la[32'h12], la[32'h20]);
    load();
    $display("T|task num=%0d [10]=%h [20]=%h", lmem.mem.num(), lmem.mem[32'h10], lmem.mem[32'h20]);
    nb[100] = 8'h77;
    $readmemh("@B@", nb);
    $display("T|noaddr num=%0d [0]=%h [1]=%h [2]=%h [100]=%h", nb.num(), nb[0], nb[1], nb[2], nb[100]);
    $readmemh("@B@", st, 5);
    $display("T|start num=%0d [5]=%h [7]=%h", st.num(), st[5], st[7]);
    $readmemh("@B@", se, 5, 6);
    $display("T|range num=%0d [5]=%h [6]=%h exists7=%0d", se.num(), se[5], se[6], se.exists(7));
    $readmemh("@B@", ds, 7, 5);
    $display("T|desc num=%0d [7]=%h [6]=%h [5]=%h", ds.num(), ds[7], ds[6], ds[5]);
    $readmemb("@C@", bb);
    $display("T|bin num=%0d [3]=%h [4]=%h", bb.num(), bb[3], bb[4]);
    $writememh("@W@", la);
    $readmemh("@W@", rb);
    $display("T|writemem num=%0d [10]=%h [11]=%h [12]=%h [20]=%h", rb.num(), rb[32'h10], rb[32'h11], rb[32'h12], rb[32'h20]);
    $writememh("@W2@", la, 32'h11, 32'h20);
    $readmemh("@W2@", rr);
    $display("T|writemem-range num=%0d exists10=%0d [11]=%h [20]=%h", rr.num(), rr.exists(32'h10), rr[32'h11], rr[32'h20]);
  end
endmodule
"#
    .replace("@A@", &a.display().to_string())
    .replace("@B@", &b.display().to_string())
    .replace("@C@", &c.display().to_string())
    .replace("@W2@", &w2.display().to_string())
    .replace("@W@", &w.display().to_string());
    let got = t_lines(&src);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        got,
        [
            "T|local num=4 [10]=11 [11]=22 [12]=33 [20]=44",
            "T|task num=4 [10]=11 [20]=44",
            "T|noaddr num=4 [0]=aa [1]=bb [2]=cc [100]=77",
            "T|start num=3 [5]=aa [7]=cc",
            "T|range num=2 [5]=aa [6]=bb exists7=0",
            "T|desc num=3 [7]=aa [6]=bb [5]=cc",
            "T|bin num=2 [3]=a [4]=5",
            "T|writemem num=4 [10]=11 [11]=22 [12]=33 [20]=44",
            "T|writemem-range num=3 exists10=0 [11]=22 [20]=44",
        ],
    );
}

/// §21.4 asks for an integral index type: a string-indexed array is still
/// no memory.
#[test]
fn readmem_rejects_string_indexed_array() {
    let dir = scratch("str");
    let hex = dir.join("p.hex");
    std::fs::write(&hex, "11\n").unwrap();
    let src = r#"
module t;
  bit [7:0] sm [string];
  initial begin
    $readmemh("@HEX@", sm);
    $display("T|num=%0d", sm.num());
  end
endmodule
"#
    .replace("@HEX@", &hex.display().to_string());
    let got = t_lines(&src);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        got,
        [
            "[xezim][error] $readmemh: 'sm' does not name an unpacked memory (t=0)",
            "T|num=0",
        ]
    );
}

/// Keys outside `0..=i64::MAX` keep their value through a dump and a load
/// (§21.4, §21.5, §7.8): a negative key of an `int` index is written as its
/// 64-bit two's complement and read back as negative (an `int`-width
/// `@ffffffff` reads as -1 too), and a `bit [63:0]` key above `i64::MAX`
/// keeps its value in the file, in a start address and in the default
/// finish address. Negative indices of an unpacked memory round-trip the
/// same way.
#[test]
fn readmem_writemem_keys_beyond_i64_range() {
    let dir = scratch("wide");
    let f = |n: &str| dir.join(n).display().to_string();
    std::fs::write(dir.join("i.hex"), "@ffffffff\n5a\n").unwrap();
    std::fs::write(dir.join("u.hex"), "@8000000000000000\naa bb\n").unwrap();
    std::fs::write(dir.join("n.hex"), "aa bb cc\n").unwrap();
    let src = r#"
module t;
  bit [7:0] sm [int];
  bit [7:0] sr [int];
  bit [7:0] si [int];
  bit [7:0] um [bit [63:0]];
  bit [7:0] ur [bit [63:0]];
  bit [7:0] uf [bit [63:0]];
  bit [7:0] us [longint unsigned];
  bit [7:0] pm [-2:1];
  bit [7:0] pr [-2:1];
  initial begin
    sm[-1] = 8'h11; sm[-2] = 8'h22; sm[5] = 8'h33;
    $writememh("@SW@", sm);
    $readmemh("@SW@", sr);
    $display("T|signed num=%0d [-2]=%h [-1]=%h [5]=%h exists0=%0d", sr.num(), sr[-2], sr[-1], sr[5], sr.exists(0));
    $readmemh("@I@", si);
    $display("T|int-width num=%0d [-1]=%h", si.num(), si[-1]);
    um[64'h8000_0000_0000_0000] = 8'h44; um[64'hffff_ffff_ffff_fffe] = 8'h55; um[1] = 8'h66;
    $writememh("@UW@", um);
    $readmemh("@UW@", ur);
    $display("T|unsigned num=%0d [1]=%h [8000..]=%h [ff..fe]=%h exists0=%0d", ur.num(), ur[1],
             ur[64'h8000_0000_0000_0000], ur[64'hffff_ffff_ffff_fffe], ur.exists(0));
    $readmemh("@U@", uf);
    $display("T|unsigned-file num=%0d [8000..]=%h [8000..01]=%h", uf.num(),
             uf[64'h8000_0000_0000_0000], uf[64'h8000_0000_0000_0001]);
    $readmemh("@B@", us, 64'hffff_ffff_ffff_fffe);
    $display("T|unsigned-start num=%0d [ff..fe]=%h [ff..ff]=%h", us.num(),
             us[64'hffff_ffff_ffff_fffe], us[64'hffff_ffff_ffff_ffff]);
    pm = '{8'h01, 8'h02, 8'h03, 8'h04};
    $writememh("@PW@", pm, -1, 1);
    $readmemh("@PW@", pr);
    $display("T|unpacked [-2]=%h [-1]=%h [0]=%h [1]=%h", pr[-2], pr[-1], pr[0], pr[1]);
  end
endmodule
"#
    .replace("@SW@", &f("sw.hex"))
    .replace("@UW@", &f("uw.hex"))
    .replace("@PW@", &f("pw.hex"))
    .replace("@I@", &f("i.hex"))
    .replace("@U@", &f("u.hex"))
    .replace("@B@", &f("n.hex"));
    let got = t_lines(&src);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        got,
        [
            "T|signed num=3 [-2]=22 [-1]=11 [5]=33 exists0=0",
            "T|int-width num=1 [-1]=5a",
            "T|unsigned num=3 [1]=66 [8000..]=44 [ff..fe]=55 exists0=0",
            "T|unsigned-file num=2 [8000..]=aa [8000..01]=bb",
            "T|unsigned-start num=2 [ff..fe]=aa [ff..ff]=bb",
            "T|unpacked [-2]=00 [-1]=02 [0]=03 [1]=04",
        ]
    );
}

/// An index type wider than 64 bits keeps every key (§21.4, §21.5, §7.8):
/// the default range of a dump covers the whole index type, and an address
/// line or a start or finish argument above 64 bits keeps its value. A
/// signed wide index type writes a negative key as its 128-bit two's
/// complement, and a positive key of 64 one bits stays positive; both read
/// back. A load that reaches the largest key of a 128-bit index type ends
/// there instead of wrapping to the most negative key.
#[test]
fn readmem_writemem_index_wider_than_64_bits() {
    let dir = scratch("wider");
    let f = |n: &str| dir.join(n).display().to_string();
    std::fs::write(dir.join("l.hex"), "@10000000000000000\nab cd\n").unwrap();
    std::fs::write(
        dir.join("m.hex"),
        "@7fffffffffffffffffffffffffffffff\naa bb\n",
    )
    .unwrap();
    let src = r#"
module t;
  bit [7:0] mx [bit signed [127:0]];
  bit [7:0] wm [bit [79:0]];
  bit [7:0] wr [bit [79:0]];
  bit [7:0] lm [bit [79:0]];
  bit [7:0] rg [bit [79:0]];
  bit [7:0] sm [bit signed [79:0]];
  bit [7:0] sr [bit signed [79:0]];
  bit signed [79:0] big = 80'sh1_0000_0000_0000_0000;
  initial begin
    wm[5] = 8'h11;
    wm[80'h1_0000_0000_0000_0000] = 8'h22;
    wm[80'hffff_0000_0000_0000_0000] = 8'h33;
    $writememh("@WW@", wm);
    $readmemh("@WW@", wr);
    $display("T|wide num=%0d %h %h %h", wr.num(), wr[5], wr[80'h1_0000_0000_0000_0000],
             wr[80'hffff_0000_0000_0000_0000]);
    $readmemh("@L@", lm);
    $display("T|load num=%0d %h %h", lm.num(), lm[80'h1_0000_0000_0000_0000],
             lm[80'h1_0000_0000_0000_0001]);
    $writememh("@RG@", wm, 80'h1_0000_0000_0000_0000, 80'hffff_ffff_ffff_ffff_ffff);
    $readmemh("@RG@", rg);
    $display("T|range num=%0d %h %h", rg.num(), rg[80'h1_0000_0000_0000_0000],
             rg[80'hffff_0000_0000_0000_0000]);
    sm[-1] = 8'h44;
    sm[-big] = 8'h55;
    sm[80'shffff_ffff_ffff_ffff] = 8'h66;
    $writememh("@SW@", sm);
    $readmemh("@SW@", sr);
    $display("T|signed num=%0d %h %h %h", sr.num(), sr[-1], sr[-big], sr[80'shffff_ffff_ffff_ffff]);
    $readmemh("@M@", mx);
    $display("T|max num=%0d %h", mx.num(), mx[128'sh7fff_ffff_ffff_ffff_ffff_ffff_ffff_ffff]);
  end
endmodule
"#
    .replace("@WW@", &f("ww.hex"))
    .replace("@L@", &f("l.hex"))
    .replace("@RG@", &f("rg.hex"))
    .replace("@SW@", &f("sw.hex"))
    .replace("@M@", &f("m.hex"));
    let got = t_lines(&src);
    let files: Vec<String> = ["ww.hex", "sw.hex"]
        .iter()
        .map(|n| std::fs::read_to_string(dir.join(n)).unwrap_or_default())
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        files,
        [
            "@5 11\n@10000000000000000 22\n@ffff0000000000000000 33\n",
            "@ffffffffffffffff0000000000000000 55\n@ffffffffffffffffffffffffffffffff 44\n\
             @ffffffffffffffff 66\n",
        ]
    );
    assert_eq!(
        got,
        [
            "T|wide num=3 11 22 33",
            "T|load num=2 ab cd",
            "T|range num=2 22 33",
            "T|signed num=3 44 55 66",
            "T|max num=1 aa",
        ]
    );
}

/// An unsigned 128-bit index type keeps its keys above `i128::MAX` (§21.4,
/// §21.5, §7.8). An array holding only key 2^127 dumped nothing, since the
/// key read back as a negative address outside the default range. Now its
/// dump holds the key, a dump of the whole array and of a range reads back,
/// a start address and a file address at or past 2^127 load there, and a
/// load that reaches the largest key ends there. Verilator 5.046 does not
/// build `$readmem*`/`$writemem*` on an index type wider than 64 bits, so
/// the values follow §21.4 and §21.5.
#[test]
fn readmem_writemem_unsigned_128_bit_index() {
    let dir = scratch("u128");
    let f = |n: &str| dir.join(n).display().to_string();
    std::fs::write(
        dir.join("h.hex"),
        "@ffffffffffffffffffffffffffffffff\naa bb\n",
    )
    .unwrap();
    std::fs::write(dir.join("b.hex"), "aa bb\n").unwrap();
    std::fs::write(dir.join("m.hex"), "@80000000000000000000000000000001\ncc\n").unwrap();
    let src = r#"
module t;
  typedef bit [127:0] k_t;
  localparam k_t TOP = 128'h8000_0000_0000_0000_0000_0000_0000_0000;
  localparam k_t MAX = 128'hffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff;
  bit [7:0] one [k_t];
  bit [7:0] oner [k_t];
  bit [7:0] um [k_t];
  bit [7:0] ur [k_t];
  bit [7:0] rg [k_t];
  bit [7:0] uh [k_t];
  bit [7:0] us [k_t];
  bit [7:0] uf [k_t];
  initial begin
    one[TOP] = 8'h5a;
    $writememh("@ONE@", one);
    $readmemh("@ONE@", oner);
    $display("T|one num=%0d %h", oner.num(), oner[TOP]);
    um[1] = 8'h11;
    um[TOP] = 8'h22;
    um[MAX] = 8'h33;
    $writememh("@UW@", um);
    $readmemh("@UW@", ur);
    $display("T|all num=%0d %h %h %h", ur.num(), ur[1], ur[TOP], ur[MAX]);
    $writememh("@RG@", um, TOP, MAX);
    $readmemh("@RG@", rg);
    $display("T|range num=%0d %h %h", rg.num(), rg[TOP], rg[MAX]);
    $readmemh("@H@", uh);
    $display("T|top num=%0d %h", uh.num(), uh[MAX]);
    $readmemh("@B@", us, TOP - 1);
    $display("T|start num=%0d %h %h", us.num(), us[TOP - 1], us[TOP]);
    $readmemh("@M@", uf);
    $display("T|file num=%0d %h", uf.num(), uf[TOP + 1]);
  end
endmodule
"#
    .replace("@ONE@", &f("one.hex"))
    .replace("@UW@", &f("uw.hex"))
    .replace("@RG@", &f("rg.hex"))
    .replace("@H@", &f("h.hex"))
    .replace("@B@", &f("b.hex"))
    .replace("@M@", &f("m.hex"));
    let got = t_lines(&src);
    let one = std::fs::read_to_string(dir.join("one.hex")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        one.contains("@80000000000000000000000000000000") && one.contains("5a"),
        "{one:?}"
    );
    assert_eq!(
        got,
        [
            "T|one num=1 5a",
            "T|all num=3 11 22 33",
            "T|range num=2 22 33",
            "T|top num=1 aa",
            "T|start num=2 aa bb",
            "T|file num=1 cc",
        ]
    );
}
