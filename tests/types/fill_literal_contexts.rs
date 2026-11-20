//! IEEE 1800-2023 §5.7.1, §6.24.1, §10.7, §11.4.5–6, §11.7 and
//! §13.4.1: a bare fill takes its context width. A signing conversion or a
//! typed destination yields an ordinary value with its own width.

fn check(src: &str, expected: &[&str]) {
    let sim = xezim::simulate(src, 20_000).expect("simulate fill literal contexts");
    let actual: Vec<_> = sim
        .output
        .iter()
        .filter_map(|line| line.message.strip_prefix("T|"))
        .collect();
    assert_eq!(actual, expected);
}

const ISSUE: &str = r#"`timescale 1ns/1ps
module top;
  logic [7:0] v = 8'hff;
  initial begin
    $display("T|%b %b %b", v ==? '1, v !=? '1, v == $unsigned('1));
    $finish;
  end
endmodule
"#;

#[test]
fn bare_fill_and_unsigned_conversion_have_different_contexts() {
    check(ISSUE, &["1 0 0"]);
}

const WILDCARD_FILL: &str = r#"`timescale 1ns/1ps
module top;
  logic [7:0] v;
  initial begin
    v = 8'hff;
    $display("T|rhs %b %b %b %b", v ==? 'x, v !=? 'x, v ==? 'z, v !=? 'z);
    $display("T|lhs %b %b %b %b", 'x ==? v, 'x !=? v, 'z ==? v, 'z !=? v);
    v = 8'h00;
    $display("T|zero %b %b", v ==? '0, v !=? '0);
    $finish;
  end
endmodule
"#;

/// §5.7.1 and §11.4.6: X and Z fill either side, with masking on the right.
#[test]
fn wildcard_comparison_fills_x_and_z_on_either_side() {
    check(WILDCARD_FILL, &["rhs 1 0 1 0", "lhs x x x x", "zero 1 0"]);
}

const COMPARE_CASTS: &str = r#"`timescale 1ns/1ps
module top;
  logic [7:0] v;
  initial begin
    v = 8'hff;
    $display("T|ones %b %b %b %b",
             v == $unsigned('1), v == $signed('1),
             v == unsigned'('1), v == signed'('1));
    $display("T|unknown %b %b %b %b",
             v == $unsigned('x), v == $signed('z),
             v == unsigned'('z), v == signed'('x));
    $display("T|zero %b %b", v == $unsigned('0), 8'h00 == $signed('0));
    $finish;
  end
endmodule
"#;

#[test]
fn signing_conversions_keep_the_self_determined_operand_width() {
    check(
        COMPARE_CASTS,
        &["ones 0 0 0 0", "unknown 0 0 0 0", "zero 0 1"],
    );
}

const COMPARE_STORES: &str = r#"`timescale 1ns/1ps
class Flags;
  logic prop;
  logic assoc[string];
  function void set();
    prop = '1;
    assoc["one"] = '1;
  endfunction
endclass

module top;
  logic [63:0] wide;
  logic scalar;
  logic [31:0] word32;
  logic assoc[int];
  integer words[int];
  logic [7:0] byte_value;
  Flags flags;
  function automatic logic compare_local(input logic [63:0] operand);
    logic local_bit;
    local_bit = '1;
    return operand == local_bit;
  endfunction
  initial begin
    wide = 64'hffff_ffff_ffff_ffff;
    scalar = '1;
    word32 = '1;
    assoc[1] = '1;
    assoc[2] = 'z;
    words[0] = 'bx;
    flags = new;
    flags.set();
    byte_value = $unsigned('1);
    #1;
    $display("T|stored %b %b %b %b %b",
             wide == scalar, wide == word32, wide == assoc[1],
             wide == flags.prop, wide == flags.assoc["one"]);
    $display("T|local %b", compare_local(wide));
    $display("T|unknowns %b %b", wide == assoc[2],
             64'hffff_ffff_0000_0000 == words[0]);
    $display("T|byte %h", byte_value);
    $finish;
  end
endmodule
"#;

#[test]
fn fixed_width_stores_consume_the_fill() {
    check(
        COMPARE_STORES,
        &["stored 0 0 0 0 0", "local 0", "unknowns 0 0", "byte 01"],
    );
}

const COMPARE_RETURNS: &str = r#"`timescale 1ns/1ps
class Source;
  function logic one();
    return '1;
  endfunction
endclass

module top;
  logic [7:0] v;
  Source source;
  function automatic logic one();
    return '1;
  endfunction
  function automatic integer unknown_word();
    return 'bx;
  endfunction
  initial begin
    v = 8'hff;
    source = new;
    $display("T|return %b %b %b", v == one(), v == source.one(),
             64'hffff_ffff_0000_0000 == unknown_word());
    $finish;
  end
endmodule
"#;

#[test]
fn function_and_method_returns_take_the_declared_width() {
    check(COMPARE_RETURNS, &["return 0 0 0"]);
}

const SIGNED_STORES: &str = r#"`timescale 1ns/1ps
class Source;
  struct packed { logic [7:0] hi, lo; } pair;
  function void write_member();
    pair = 16'h0000;
    pair.hi = $signed('1);
  endfunction
  function logic [7:0] unknown_byte();
    return $signed('x);
  endfunction
endclass

module top;
  logic [15:0] constant_slice, indexed_slice;
  struct packed { logic [7:0] hi, lo; } pair;
  logic [7:0] matrix[2][2], hi, lo;
  Source source;
  function automatic logic [7:0] high_impedance_byte();
    return signed'('z);
  endfunction
  initial begin
    source = new;
    source.write_member();
    constant_slice = 16'h0000;
    constant_slice[15:8] = $signed('1);
    indexed_slice = 16'h0000;
    indexed_slice[4 +: 8] = $signed('1);
    pair = 16'h0000;
    pair.hi = $signed('x);
    matrix[0][0] = signed'('z);
    {hi, lo} = $signed('x);
    #1;
    $display("T|slices %h %h", constant_slice, indexed_slice);
    $display("T|members %h %h", pair, source.pair);
    $display("T|matrix and concat %h %h%h", matrix[0][0], hi, lo);
    $display("T|returns %h %h", high_impedance_byte(), source.unknown_byte());
    $finish;
  end
endmodule
"#;

/// §10.7 and §11.8.4: a signed one-bit result sign-extends through these assignments.
#[test]
fn signed_conversions_extend_through_packed_and_array_destinations() {
    check(
        SIGNED_STORES,
        &[
            "slices ff00 0ff0",
            "members xx00 ff00",
            "matrix and concat zz xxxx",
            "returns zz xx",
        ],
    );
}

const TIER: &str = r#"`timescale 1ns/1ps
class Source;
  function logic [7:0] z_byte(input bit take_fill);
    if (take_fill)
      return signed'('z);
    return 8'h5a;
  endfunction
endclass

module top;
  Source source;
  int function_mismatches, method_mismatches;
  function automatic logic [7:0] x_byte(input bit take_fill);
    if (take_fill)
      return $signed('x);
    return 8'ha5;
  endfunction
  initial begin
    source = new;
    function_mismatches = 0;
    method_mismatches = 0;
    for (int i = 0; i < 1200; i++) begin
      if (x_byte(i[0]) !== (i[0] ? 8'hxx : 8'ha5))
        function_mismatches++;
      if (source.z_byte(i[0]) !== (i[0] ? 8'hzz : 8'h5a))
        method_mismatches++;
    end
    $display("T|mismatches %0d %0d", function_mismatches, method_mismatches);
    $finish;
  end
endmodule
"#;

/// §10.7, §11.8.4 and §13.4.1: both return conversions have the same
/// values on every call, including after compilation at the default tier.
#[test]
fn signed_fill_returns_agree_before_and_after_tiering() {
    check(TIER, &["mismatches 0 0"]);
}

const WILDCARD_SIZED: &str = r#"`timescale 1ns/1ps
class Source;
  logic prop;
  function void set();
    prop = '1;
  endfunction
endclass

module top;
  logic [7:0] v;
  logic scalar;
  logic assoc[int];
  Source source;
  function automatic logic one();
    return '1;
  endfunction
  initial begin
    v = 8'hff;
    scalar = '1;
    assoc[1] = '1;
    source = new;
    source.set();
    #1;
    $display("T|casts %b %b %b %b %b",
             v ==? $unsigned('1), v ==? $signed('1), v ==? unsigned'('1),
             v ==? signed'('1), v ==? $unsigned('x));
    $display("T|stored %b %b %b %b",
             v ==? scalar, v ==? source.prop, v ==? assoc[1], v !=? assoc[1]);
    $display("T|return %b", v ==? one());
    $finish;
  end
endmodule
"#;

/// §6.24.1, §10.7, §11.4.6, §11.7 and §13.4.1: wildcard comparisons
/// extend typed unsigned operands with zeros, including a cast of 'x.
#[test]
fn wildcard_comparison_respects_cast_store_and_return_widths() {
    check(
        WILDCARD_SIZED,
        &["casts 0 0 0 0 0", "stored 0 0 0 1", "return 0"],
    );
}

const MIXED_SET_WIDTHS: &str = r#"`timescale 1ns/1ps
module top;
  logic [3:0] nibble;
  int selected;
  initial begin
    nibble = 4'hf;
    case (nibble) inside
      8'hf0: selected = 1;
      '1: selected = 2;
      default: selected = 3;
    endcase
    $display("T|case %0d set %b reverse %b",
             selected, nibble inside {8'hf0, '1}, nibble inside {'1, 8'hf0});
    $finish;
  end
endmodule
"#;

/// §5.7.1, §11.4.13, §12.5 and §12.5.4: the wider value item sets the
/// comparison width, at which the zero-extended selector misses both items.
#[test]
fn set_membership_uses_the_wider_item_with_a_fill_literal() {
    check(MIXED_SET_WIDTHS, &["case 3 set 0 reverse 0"]);
}

const COMPUTED_CASTS: &str = r#"`timescale 1ns/1ps
module top;
  logic clk = 0;
  logic [7:0] a, b;
  logic signed [7:0] s;
  logic [7:0] u;
  always @(posedge clk) begin
    s <= $signed(a + b);
    u <= $unsigned(a + b);
  end
  initial begin
    a = 8'h80;
    b = 8'h01;
    #1 clk = 1;
    #1 $display("T|first %0d %0d", s, u);
    clk = 0;
    a = 8'hff;
    b = 8'h01;
    #1 clk = 1;
    #1 $display("T|second %0d %0d", s, u);
    $finish;
  end
endmodule
"#;

/// §11.6.1 and §11.7: signing conversions preserve both the eight-bit sum
/// and its width while changing how its high bit is interpreted.
#[test]
fn signing_a_computed_operand_preserves_its_bits_and_width() {
    check(COMPUTED_CASTS, &["first -127 129", "second 0 0"]);
}

const NATIVE_ARRAY_STORES: &str = r#"`timescale 1ns/1ps
module top;
  logic clk = 0;
  logic sel;
  int idx;
  logic signed [3:0] s4;
  logic [7:0] u8;
  logic [7:0] dynamic_index[0:1], constant_index[0:1];
  always @(posedge clk) case (sel)
    1'b0: dynamic_index[idx] <= s4;
    1'b1: dynamic_index[idx] <= u8[3:0];
  endcase
  always @(posedge clk) case (sel)
    1'b0: constant_index[1] <= s4;
    1'b1: constant_index[1] <= u8[3:0];
  endcase
  initial begin
    sel = 1;
    idx = 1;
    s4 = 4'sb1010;
    u8 = 8'hca;
    #2 clk = 1;
    #2 $display("T|arrays %h %h", dynamic_index[1], constant_index[1]);
    $finish;
  end
endmodule
"#;

/// §10.7 and §11.8.1: the unsigned part-select zero-extends even when
/// the other case arm stores a signed value.
#[test]
fn unsigned_part_select_zero_extends_in_array_stores() {
    check(NATIVE_ARRAY_STORES, &["arrays 0a 0a"]);
}

#[cfg(unix)]
mod cli {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let template = std::env::temp_dir().join("xezim-fill-literal.XXXXXX");
            let output = Command::new("mktemp")
                .arg("-d")
                .arg(template)
                .output()
                .expect("run mktemp");
            assert!(output.status.success(), "{output:?}");
            Self(PathBuf::from(
                String::from_utf8(output.stdout).unwrap().trim_end(),
            ))
        }

        fn simulate(&self, src: &str, options: &[(&str, &Path)], env: &[(&str, &str)]) -> String {
            let input = self.0.join("top.sv");
            std::fs::write(&input, src).unwrap();
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
            cmd.current_dir(&self.0)
                .args(["--no-cache", "-s", "top"])
                .arg(input)
                .env_remove("XEZIM_METHOD_CACHE")
                .envs(env.iter().copied());
            for (flag, path) in options {
                cmd.arg(flag).arg(path);
            }
            let output = cmd.output().expect("run fill literal fixture");
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(
                output.status.success(),
                "{stdout}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            stdout
                .lines()
                .filter_map(|line| line.strip_prefix("T|"))
                .collect::<Vec<_>>()
                .join("\n")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(feature = "jit")]
    #[test]
    fn native_array_stores_zero_extend_unsigned_part_selects() {
        for aot in ["0", "1"] {
            let actual = Scratch::new().simulate(
                super::NATIVE_ARRAY_STORES,
                &[],
                &[("XEZIM_JIT", "1"), ("XEZIM_AOT", aot)],
            );
            assert_eq!(actual, "arrays 0a 0a", "XEZIM_AOT={aot}");
        }
    }

    const DPI: &str = r#"`timescale 1ns/1ps
module top;
  import "DPI-C" function int take_bits(input bit [7:0] v);
  import "DPI-C" function int take_logic(input logic [7:0] v);
  initial begin
    $display("T|dpi %0h %0h %0h",
             take_bits($signed('1)), take_logic($signed('x)), take_logic(signed'('z)));
    $finish;
  end
endmodule
"#;

    const DPI_C: &str = r#"
#include <svdpi.h>

int take_bits(const svBitVecVal* v) { return v[0] & 0xff; }

int take_logic(const svLogicVecVal* v) {
    return ((v[0].aval & 0xff) << 8) | (v[0].bval & 0xff);
}
"#;

    /// §10.7, §13.5.1, §35.5.6 and Annex H.7.7: the signed actual is
    /// extended to the packed formal width before DPI marshals its bits.
    #[test]
    fn signed_fill_extends_to_packed_dpi_formals() {
        let scratch = Scratch::new();
        let source = scratch.0.join("packed.c");
        let lib = scratch.0.join("packed.so");
        std::fs::write(&source, DPI_C).unwrap();
        let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("include");
        let output = Command::new(std::env::var_os("CC").unwrap_or_else(|| "clang".into()))
            .args(["-shared", "-fPIC", "-I"])
            .arg(include)
            .arg(source)
            .arg("-o")
            .arg(&lib)
            .output()
            .expect("compile packed DPI fixture");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            scratch.simulate(DPI, &[("--dpi-lib", &lib)], &[]),
            "dpi ff ffff ff"
        );
    }
}
