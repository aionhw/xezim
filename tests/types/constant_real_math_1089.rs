//! §11.2.1, §20.5 and §20.8.2: constant real math agrees with run-time
//! math, including nested bit conversions and integer dimension results.

use std::process::Command;

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 100_000)
        .expect("simulate constant math")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|s| s.starts_with("PASS") || s.starts_with("FAIL") || s.starts_with("T|"))
        .collect()
}

#[test]
fn reporter_math_parameters_and_dimension() {
    let output = lines(
        r#"
`timescale 1ns/1ps
module top;
  localparam real LN = $ln(8.0);
  localparam real EX = $exp(1.0);
  localparam real PW = $pow(2.0, 3.0);
  localparam real AT = $atan2(1.0, 1.0);
  parameter real PLN = $ln(8.0);
  localparam ULN = $ln(8.0);
  localparam int N = $rtoi($log10(1000.0));
  logic [N-1:0] v;
  real x = 8.0;
  initial begin
    #1;
    if (LN == $ln(x)) $display("PASS ln %f", LN); else $display("FAIL ln %f (expected %f)", LN, $ln(x));
    if (EX == $exp(x / 8.0)) $display("PASS exp %f", EX); else $display("FAIL exp %f (expected %f)", EX, $exp(x / 8.0));
    if (PW == 8.0) $display("PASS pow %f", PW); else $display("FAIL pow %f (expected 8.0)", PW);
    if (AT == $atan2(x, x)) $display("PASS atan2 %f", AT); else $display("FAIL atan2 %f (expected %f)", AT, $atan2(x, x));
    if (PLN == $ln(x)) $display("PASS parameter %f", PLN); else $display("FAIL parameter %f (expected %f)", PLN, $ln(x));
    if (ULN == $ln(x)) $display("PASS untyped %f", ULN); else $display("FAIL untyped %f (expected %f)", ULN, $ln(x));
    if ($bits(v) == 3) $display("PASS width %0d", $bits(v)); else $display("FAIL width %0d (expected 3)", $bits(v));
    $finish;
  end
endmodule
"#,
    );
    assert_eq!(
        output,
        [
            "PASS ln 2.079442",
            "PASS exp 2.718282",
            "PASS pow 8.000000",
            "PASS atan2 0.785398",
            "PASS parameter 2.079442",
            "PASS untyped 2.079442",
            "PASS width 3",
        ]
    );
}

fn expected() -> [&'static str; 25] {
    [
        "T|ln 2.079442",
        "T|log10 3.000000",
        "T|exp 2.718282",
        "T|sqrt 3.000000",
        "T|floor -2.000000",
        "T|ceil -1.000000",
        "T|pow 8.000000",
        "T|sin 0.479426",
        "T|cos 0.877583",
        "T|tan 0.546302",
        "T|asin 0.523599",
        "T|acos 1.047198",
        "T|atan 0.463648",
        "T|atan2 2.356194",
        "T|hypot 5.000000",
        "T|sinh 0.521095",
        "T|cosh 1.127626",
        "T|tanh 0.462117",
        "T|asinh 0.481212",
        "T|acosh 1.316958",
        "T|atanh 0.549306",
        "T|bits 0.000000 2.079442 0.000000",
        "T|width 3",
        "T|signature 1 256 3 5",
        "T|package 2.079442 8.000000",
    ]
}

#[test]
fn table_functions_conversions_and_typed_signature_parameters() {
    assert_eq!(
        lines(include_str!("constant_real_math_1089.sv")),
        expected()
    );
}

fn transcript(command: &mut Command) -> String {
    let output = command.output().expect("run constant math command");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{text}");
    text
}

#[test]
fn constants_survive_cold_warm_cache_and_compiled_artifact() {
    let bin = env!("CARGO_BIN_EXE_xezim");
    let dir = std::env::temp_dir().join(format!("xezim_const_math_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("design.sv");
    let cache = dir.join("cache");
    let artifact = dir.join("design.xez");
    std::fs::write(&source, include_str!("constant_real_math_1089.sv")).unwrap();
    let check = |text: String| {
        assert_eq!(
            text.lines()
                .filter(|s| s.starts_with("T|"))
                .collect::<Vec<_>>(),
            expected(),
            "{text}"
        );
    };
    for _ in 0..2 {
        check(transcript(
            Command::new(bin)
                .args(["--simulate", "-s", "top", "--cache-dir"])
                .arg(&cache)
                .arg(&source),
        ));
    }
    transcript(
        Command::new(bin)
            .args(["--compile", "--no-cache", "-s", "top"])
            .arg(&source)
            .arg("-o")
            .arg(&artifact),
    );
    check(transcript(
        Command::new(bin).arg("--simulate").arg(&artifact),
    ));
    std::fs::remove_dir_all(dir).unwrap();
}
