//! IEEE 1800-2017 §20.15.1, §20.15.2, Annex N: `$random` and the `$dist_*`
//! functions run the standard's generators, so a seed gives the sequence
//! every conforming simulator gives (iverilog
//! agrees on `$random`). xezim used a generator of its own.
use xezim::simulate;

/// The `$display` lines of `src` that start with one of `prefixes`.
fn shown(src: &str, prefixes: &[&str]) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| prefixes.iter().any(|p| m.starts_with(p)))
        .collect()
}

/// No-seed `$random` starts at seed 0; a seeded call advances its seed in place.
#[test]
fn random_follows_annex_n() {
    let src = r#"module tb;
  integer s; integer r;
  initial begin
    $display("R0 %0d %0d %0d", $random, $random, $random);
    s = 0; r = $random(s); $display("R1 %0d %0d", r, s); r = $random(s); $display("R1 %0d %0d", r, s);
    s = 42; r = $random(s); $display("R2 %0d %0d", r, s);
    s = -7; r = $random(s); $display("R3 %0d %0d", r, s);
  end
endmodule
"#;
    let want = r#"R0 303379748 -1064739199 -2071669239
R1 303379748 -1844104698
R1 -1064739199 1082744015
R2 -2144582656 2900899
R3 2147000831 -483482"#;
    let got = shown(src, &["R"]);
    assert_eq!(got.join("\n"), want, "{got:#?}");
}

/// Every `$dist_*` function, including non-positive arguments, over several seeds.
#[test]
fn distributions_follow_annex_n() {
    let src = r#"module tb;
  integer s, k;
  initial begin
    for (k = 0; k < 4; k++) begin
      s = k * 1234 - 2; $display("U %0d %0d", $dist_uniform(s, -5, 20), s);
      s = k * 1234 - 2; $display("U2 %0d %0d", $dist_uniform(s, 0, 2147483647), s);
      s = k * 1234 - 2; $display("U3 %0d %0d", $dist_uniform(s, -2147483648, 7), s);
      s = k * 77 + 1; $display("N %0d %0d", $dist_normal(s, 100, 15), s);
      s = k * 77 + 1; $display("N2 %0d %0d", $dist_normal(s, -40, 3), s);
      s = k * 99; $display("E %0d %0d", $dist_exponential(s, 50), s);
      s = k * 99; $display("P %0d %0d", $dist_poisson(s, 6), s);
      s = k * 99; $display("C %0d %0d", $dist_chi_square(s, 3), s);
      s = k * 99; $display("C2 %0d %0d", $dist_chi_square(s, 4), s);
      s = k * 99; $display("T %0d %0d", $dist_t(s, 5), s);
      s = k * 99; $display("R %0d %0d", $dist_erlang(s, 3, 40), s);
      s = k * 99; $display("Z %0d %0d %0d", $dist_poisson(s, 0), $dist_exponential(s, -1), s);
    end
  end
endmodule
"#;
    let want = r#"U 20 -138137
U2 2147415039 -138137
U3 -68601 -138137
N 108 772999773
N2 -38 772999773
E 28 -1844104698
P 3 837833973
C 8 75814084
C2 4 1082744015
T 0 -958425333
R 80 75814084
Z 0 0 0
U -5 85093009
U2 42546693 85093009
U3 -2104936955 85093009
N 103 -489903912
N2 -39 -489903912
E 322 6837832
P 0 6837832
C 6 471364482
C2 13 -164184151
T 0 -269253343
R 91 -1321463978
Z 0 0 99
U -4 170324155
U2 85162250 170324155
U3 -2062321398 170324155
N 80 1641788823
N2 -44 1641788823
E 287 13675663
P 2 1176410805
C 4 1349468875
C2 12 -328437372
T 1 1977927818
R 95 1176410805
Z 0 0 198
U -4 255555301
U2 127777807 255555301
U3 -2019705841 255555301
N 57 2076183348
N2 -49 2076183348
E 267 20513494
P 3 -1796308475
C 1 -509780222
C2 11 -492690593
T 0 -860490079
R 75 -620681708
Z 0 0 297"#;
    let got = shown(
        src,
        &[
            "C ", "C2 ", "E ", "N ", "N2 ", "P ", "R ", "T ", "U ", "U2 ", "U3 ", "Z ",
        ],
    );
    assert_eq!(got.join("\n"), want, "{got:#?}");
}
