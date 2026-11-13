//! IEEE 1800-2023 §11.8.2, §13.4.1, §23.10.1 and §3.12.1:
//! constants retain enclosing widths, declared storage types and lexical scope.

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 100_000)
        .expect("simulate constant contexts")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|s| s.starts_with("PASS") || s.starts_with("FAIL") || s.starts_with("T|"))
        .collect()
}

#[test]
fn enclosing_widths_reach_constant_operands_1096() {
    assert_eq!(
        lines(include_str!("constant_expression_widths.sv")),
        [
            "PASS P=18",
            "PASS L=-6",
            "PASS Q=6",
            "PASS V=6",
            "PASS C=49",
            "PASS PE[1]=ffff",
            "PASS HW=00ff",
        ]
    );
}

#[test]
fn constant_function_real_locals_and_dimension_1125() {
    assert_eq!(
        lines(include_str!("constant_function_storage.sv")),
        ["PASS width", "PASS P"]
    );
}

#[test]
fn real_math_arguments_keep_width_and_unit_scope_1109() {
    let out = lines(include_str!("constant_math_unit_scope.sv"));
    assert_eq!(out.len(), 2, "{out:?}");
    assert!(out.iter().all(|s| s.starts_with("PASS")), "{out:?}");
}

#[test]
fn storage_conversions_preserve_real_and_integral_boundaries() {
    assert_eq!(
        lines(include_str!("constant_storage_neighbors.sv")),
        [
            "T|real 2.500000 2.500000 2.500000 0.500000 2.000000 2.000000",
            "T|integral 3 3 49",
            "T|fill 1 01 ff",
        ]
    );
}

#[test]
fn self_determined_boundaries_unknown_arms_and_defparam_scope() {
    assert_eq!(
        lines(include_str!("constant_context_neighbors.sv")),
        [
            "T|context 49 1 14 128 49 1 15 xxxx0000",
            "T|unknown xxxxxxxx",
            "T|override 00ff 0001 2.500000",
        ]
    );
}

#[test]
fn qualified_unit_constants_keep_dependencies_and_declared_ranges() {
    assert_eq!(
        lines(include_str!("constant_unit_neighbors.sv")),
        ["T|unit 2.000000 8.000000 16.000000 8 a5 8.000000 3.000000",]
    );
}

#[test]
fn real_destinations_do_not_supply_an_integral_width() {
    assert_eq!(
        lines(include_str!("constant_real_targets.sv")),
        [
            "T|targets 44.000000 44.000000 44.000000 44.000000 44.000000 44.000000 44.000000 44.000000 44.000000 300",
        ]
    );
}

#[test]
fn repeated_cast_and_concatenation_boundaries_do_not_repeat_evaluation() {
    for depth in [8, 16, 32] {
        let mut expr = "8'd1".to_string();
        for _ in 0..depth {
            expr = format!("8'({{{expr}}} + 8'd1)");
        }
        let src = format!(
            "module top; localparam P={expr}; initial $display(\"T|nested %0d\", P); endmodule"
        );
        assert_eq!(lines(&src), [format!("T|nested {}", depth + 1)]);
    }
}

#[test]
fn qualified_unit_parameters_keep_width_and_signedness_when_shadowed() {
    assert_eq!(
        lines(
            r#"
parameter logic [7:0] U=8'ha5;
parameter logic signed [7:0] S=-1;
module top;
  parameter logic [2:0] U=3'd3;
  parameter logic [2:0] S=3'd2;
  localparam Q=$unit::U;
  localparam R=$unit::S;
  initial $display("T|qualified %0d %0d %0d %0d %0d %0d", U,S,$bits(Q),Q,$bits(R),R);
endmodule
"#
        ),
        ["T|qualified 3 2 8 165 8 -1"]
    );
}

#[test]
fn constant_contexts_survive_design_cache_and_artifact_roundtrip() {
    use std::process::Command;
    let dir = std::env::temp_dir().join(format!("xezim_const_contexts_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let run = |command: &mut Command| {
        let out = command.output().expect("run constant contexts");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success(), "{text}");
        text.lines()
            .filter(|s| s.starts_with("T|") || s.starts_with("PASS") || s.starts_with("FAIL"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    for (name, src) in [
        ("widths", include_str!("constant_expression_widths.sv")),
        ("storage", include_str!("constant_storage_neighbors.sv")),
        ("unit", include_str!("constant_unit_neighbors.sv")),
    ] {
        let source = dir.join(format!("{name}.sv"));
        let artifact = dir.join(format!("{name}.xez"));
        let cache = dir.join(format!("{name}-cache"));
        std::fs::write(&source, src).unwrap();
        let expected = lines(src);
        for _ in 0..2 {
            assert_eq!(
                run(Command::new(env!("CARGO_BIN_EXE_xezim"))
                    .args(["--simulate", "-s", "top", "+seed=1", "--cache-dir"])
                    .arg(&cache)
                    .arg(&source)),
                expected
            );
        }
        run(Command::new(env!("CARGO_BIN_EXE_xezim"))
            .args(["--compile", "--no-cache", "-s", "top"])
            .arg(&source)
            .arg("-o")
            .arg(&artifact));
        assert_eq!(
            run(Command::new(env!("CARGO_BIN_EXE_xezim"))
                .arg("--simulate")
                .arg(&artifact)),
            expected
        );
        // Semantically stale elaborations must be recompiled, even when the
        // serialized field layout has not changed.
        let mut bytes = std::fs::read(&artifact).unwrap();
        bytes[7] = 0x40;
        let stale = dir.join(format!("{name}-stale.xez"));
        std::fs::write(&stale, bytes).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
            .arg("--simulate")
            .arg(stale)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("recompile"));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn default_pattern_items_keep_their_self_determined_widths() {
    assert_eq!(
        lines(
            r#"
module top;
  typedef struct packed { logic [7:0] a,b; } pair_t;
  localparam logic [1:0][7:0] A='{4'd15+4'd1, 4'd15+4'd1};
  localparam logic [1:0][7:0] B='{default:4'd15+4'd1};
  localparam pair_t C='{a:4'd15+4'd1, default:4'd15+4'd1};
  localparam logic [1:0][7:0] D='{default:'1};
  initial $display("T|patterns %h %h %h %h", A,B,C,D);
endmodule
"#
        ),
        ["T|patterns 1010 0000 1000 ffff"]
    );
}

#[test]
fn injected_unit_array_parameters_keep_declaring_dimensions_in_children() {
    assert_eq!(
        lines(
            r#"
parameter int N=4;
parameter int A[N]='{default:7};
module child;
  parameter int N=2;
  initial $display("T|unit-array %0d %0d %0d %0d", N,$size(A),A[0],A[3]);
endmodule
module top;
  child u();
endmodule
"#
        ),
        ["T|unit-array 2 4 7 7"]
    );
}
