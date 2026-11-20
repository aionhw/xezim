//! IEEE 1800-2017 §6.16.3: a character select `s[i]` (and `s.getc(i)`) of a
//! string is a `byte` — signed 8-bit. It read as an unsigned byte, so
//! `s[0] + s[1]` of "ab" was 195 instead of -61 and `"\xff"[0]` was 255, and a
//! character select compared as text (`s[0] < 0` was 0).
//! IEEE 1800-2023 §6.16.6 and Table 6-9 order strings by unsigned bytes.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

const SRC: &str = r#"
class C;
  function void f(string s);
    int i; byte b; bit [7:0] u; logic [15:0] w; int k;
    k = 1;
    b = s[0]; i = s[0]; u = s[0]; w = {s[1], s[0]};
    $display("T|c s0=%0d s1=%0d sum=%0d b=%0d i=%0d u=%0d w=%h lt0=%0d gt=%0d eq=%0d", s[0], s[1], s[0] + s[1], b, i, u, w, s[0] < 0, s[0] > s[1], s[0] == 8'hff);
    $display("T|c getc=%0d cast=%0d ucast=%0d mul=%0d neg=%0d shr=%0d sk=%0d sz=%0d", s.getc(0), byte'(s[0]), int'(s[0]), s[0] * 2, -s[0], s[0] >>> 1, s[k], $bits(s[0]));
  endfunction
  function string g(string a, int k);
    string s;
    s = a;
    $display("T|g k=%0d b=%0d lt0=%0d sum=%0d", k, s[k], s[k] < 0, s[0] + s[1]);
    return s;
  endfunction
endclass
module top;
  string s, t; int k;
  initial begin
    C c;
    s = "\xff\x80z"; k = 1;
    $display("T|m s0=%0d s1=%0d sum=%0d lt0=%0d getc=%0d neg=%0d sk=%0d", s[0], s[1], s[0] + s[1], s[0] < 0, s.getc(0), -s[0], s[k]);
    c = new;
    repeat (2) c.f(s);
    c.f("ab");
    t = c.g("ab", 1);
    t = c.g("\xffz", 0);
    s = "\x80"; t = "\x7f";
    $display("T|cmp_positive=%0d icmp=%0d lt=%0d gt=%0d", s.compare(t) > 0, s.icompare(t), s < t, s > t);
    s = "\xff"; t = "a";
    $display("T|cmp_positive=%0d icmp=%0d lt=%0d gt=%0d", s.compare(t) > 0, s.icompare(t), s < t, s > t);
  end
endmodule
"#;

#[test]
fn string_character_selects_are_signed_bytes() {
    let _ = super::compiled_method_test_env::eager();
    let want = [
        "T|m s0=-1 s1=-128 sum=127 lt0=1 getc=-1 neg=1 sk=-128",
        "T|c s0=-1 s1=-128 sum=127 b=-1 i=-1 u=255 w=80ff lt0=1 gt=1 eq=1",
        "T|c getc=-1 cast=-1 ucast=-1 mul=-2 neg=1 shr=-1 sk=-128 sz=8",
        "T|c s0=-1 s1=-128 sum=127 b=-1 i=-1 u=255 w=80ff lt0=1 gt=1 eq=1",
        "T|c getc=-1 cast=-1 ucast=-1 mul=-2 neg=1 shr=-1 sk=-128 sz=8",
        "T|c s0=97 s1=98 sum=-61 b=97 i=97 u=97 w=6261 lt0=0 gt=0 eq=0",
        "T|c getc=97 cast=97 ucast=97 mul=194 neg=-97 shr=48 sk=98 sz=8",
        "T|g k=1 b=98 lt0=0 sum=-61",
        "T|g k=0 b=-1 lt0=1 sum=121",
        "T|cmp_positive=1 icmp=1 lt=0 gt=1",
        "T|cmp_positive=1 icmp=158 lt=0 gt=1",
    ];
    assert_eq!(t_lines(SRC), want);
}
