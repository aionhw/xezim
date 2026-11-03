//! §11.4.10, §11.4.3, §11.6.1: the left operand of a shift and
//! of `**`, and both operands of `/` and `%`, take the operation's width.
//! In a compiled class method a property sized by a class parameter
//! (`bit [W-1:0] v` in `pk #(24)`) has no width known at compile time, so
//! such an operand was cut to one bit once the method crossed the compile
//! threshold. Properties of a fixed width are checked alongside.

use std::process::Command;

const SOURCE: &str = r#"
class base_c;
  bit [4:0] len;
  bit [31:0] imm;
  virtual function void extend();
    imm = imm << (32 - len);
    imm = imm >> (32 - len);
  endfunction
endclass
class comp_c extends base_c;
  int align;
  virtual function void extend();
    super.extend();
    imm = imm << align;
  endfunction
endclass
class k;
  bit [31:0] imm;
  bit [31:0] other;
  bit [63:0] wide;
  int signed s;
  bit [7:0] narrow;
  int unsigned res;
  function void f_copy();   other = imm << 4; endfunction
  function void f_self();   imm <<= 4; endfunction
  function void f_this();   this.imm = this.imm >> 4; endfunction
  function void f_wide();   res = wide >> 32; endfunction
  function void f_ashr();   s = s >>> 4; endfunction
  function void f_narrow(); res = narrow << 4; endfunction
endclass
class pk #(int W = 16);
  bit [W-1:0] v, d, r;
  bit signed [W-1:0] s, sd, sr;
  function void f_self();   v = v << 4; endfunction
  function void f_shl();    r = v << 4; endfunction
  function void f_shr();    r = v >> 4; endfunction
  function void f_ashl();   r = v <<< 4; endfunction
  function void f_div();    r = v / d; endfunction
  function void f_mod();    r = v % d; endfunction
  function void f_pow();    r = v ** 2; endfunction
  function void f_div_eq(); r = v; r /= d; endfunction
  function void f_mod_eq(); r = v; r %= d; endfunction
  function void f_shr_eq(); r = v; r >>= 4; endfunction
  function void f_sdiv();   sr = s / sd; endfunction
  function void f_smod();   sr = s % sd; endfunction
  function void f_sashr();  sr = s >>> 4; endfunction
  function void f_spow();   sr = s ** 2; endfunction
endclass
module top;
  initial begin
    comp_c c;
    base_c b;
    k o;
    pk #(24) p;
    int fixed, bad[string];
    string names[$] = '{"self", "shl", "shr", "ashl", "div", "mod", "pow", "div_eq",
                        "mod_eq", "shr_eq", "sdiv", "smod", "sashr", "spow"};
    c = new();
    o = new();
    p = new();
    c.len = 8;
    c.align = 2;
    b = c;
    fixed = 0;
    foreach (names[j]) bad[names[j]] = 0;
    for (int i = 0; i < 1200; i++) begin
      b.imm = 32'h13c;
      b.extend();
      if (b.imm != 32'hf0) fixed++;
      o.imm = 32'h13c; o.f_copy(); if (o.other != 32'h13c0) fixed++;
      o.imm = 32'h13c; o.f_self(); if (o.imm != 32'h13c0) fixed++;
      o.imm = 32'h13c; o.f_this(); if (o.imm != 32'h13) fixed++;
      o.wide = 64'h1234_5678_9abc_def0; o.f_wide(); if (o.res != 32'h1234_5678) fixed++;
      o.s = -256; o.f_ashr(); if (o.s != -16) fixed++;
      o.narrow = 8'hff; o.f_narrow(); if (o.res != 32'hff0) fixed++;
      p.v = 24'h12_3456; p.d = 24'h123; p.s = -70000; p.sd = 300;
      p.f_self();   if (p.v != 24'h23_4560) bad["self"]++;
      p.v = 24'h12_3456;
      p.f_shl();    if (p.r != 24'h23_4560) bad["shl"]++;
      p.f_shr();    if (p.r != 24'h01_2345) bad["shr"]++;
      p.f_ashl();   if (p.r != 24'h23_4560) bad["ashl"]++;
      p.f_div();    if (p.r != 24'h1003) bad["div"]++;
      p.f_mod();    if (p.r != 24'hed) bad["mod"]++;
      p.f_pow();    if (p.r != 24'hcb_0ce4) bad["pow"]++;
      p.f_div_eq(); if (p.r != 24'h1003) bad["div_eq"]++;
      p.f_mod_eq(); if (p.r != 24'hed) bad["mod_eq"]++;
      p.f_shr_eq(); if (p.r != 24'h01_2345) bad["shr_eq"]++;
      p.f_sdiv();   if (p.sr != -233) bad["sdiv"]++;
      p.f_smod();   if (p.sr != -100) bad["smod"]++;
      p.f_sashr();  if (p.sr != -4375) bad["sashr"]++;
      p.f_spow();   if (p.sr != 1052928) bad["spow"]++;
    end
    $display("T fixed %0d", fixed);
    foreach (names[j]) $display("T %s %0d", names[j], bad[names[j]]);
    $finish;
  end
endmodule
"#;

fn check(mode: &[(&str, &str)]) {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_member_operand_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("top.sv");
    std::fs::write(&path, SOURCE).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--no-cache", "--relax-implicit-static", "-s", "top"])
        .arg(&path)
        .env_remove("XEZIM_METHOD_TIER")
        .env_remove("XEZIM_COMPILE_METHODS")
        .env_remove("XEZIM_METHOD_CACHE")
        .envs(mode.iter().copied())
        .output()
        .expect("run simulator");
    std::fs::remove_dir_all(&dir).unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{text}");
    let got: Vec<&str> = text.lines().filter(|l| l.starts_with("T ")).collect();
    let want: Vec<String> = [
        "fixed", "self", "shl", "shr", "ashl", "div", "mod", "pow", "div_eq", "mod_eq",
        "shr_eq", "sdiv", "smod", "sashr", "spow",
    ]
    .iter()
    .map(|n| format!("T {n} 0"))
    .collect();
    assert_eq!(got, want, "{text}");
}

#[test]
fn operand_widths_past_the_compile_threshold() {
    check(&[]);
}

#[test]
fn operand_widths_eager() {
    check(&[("XEZIM_METHOD_TIER", "0")]);
}

#[test]
fn operand_widths_interpreted() {
    check(&[("XEZIM_COMPILE_METHODS", "0")]);
}
