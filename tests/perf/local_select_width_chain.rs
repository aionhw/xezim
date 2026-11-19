//! §11.5.1, §11.6.1: a select of a vector is as wide as the select says —
//! one bit for `d[i]`, `|l - r| + 1` for `d[l:r]`, `w` for `d[i +: w]` —
//! never as its value, and `a ^ b` is as wide as its wider operand, so the
//! width of a chain `d[3] ^ d[9] ^ ...` follows from the declarations alone.
//! The interpreter sized a select by evaluating it and kept nothing on the
//! node, nor did any operator above it, so every level of a chain of `n`
//! selects evaluated every select under it again: n(n+1)/2 evaluations, the
//! cost of the square of the chain's length, for a chain over a function's
//! local and over a design variable alike (#1295).
//!
//! The guard counts the name lookups of a design (`XEZIM_NAME_STATS`:
//! resolutions plus frame reads) at two chain lengths, four times apart: it
//! must grow about four times, not sixteen. The values are checked by the
//! designs themselves against a reduction over a mask. The third test pins
//! the widths of the selects the change sizes and of those it leaves to
//! evaluation; its expected values are those of the reference simulator.

fn run(src: &str, env: &[(&str, &str)]) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_select_width_chain_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, src).expect("write temporary design");
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.args([
        "--simulate",
        "-s",
        "top",
        path.to_str().unwrap(),
        "--no-cache",
    ])
    .env_remove("XEZIM_METHOD_TIER")
    .env_remove("XEZIM_METHOD_CACHE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "design failed:\n{text}");
    text
}

/// A function folding a 256-bit vector and a 32-bit seed through 8 chains,
/// each XOR-ing `terms` bit-selects of the vector (or `terms` two-bit
/// part-selects with `part`) and four of the seed: the shape of a CRC or
/// parity function. With `local` the vector is a local of the function, a
/// copy of its formal; otherwise it is a design variable. The function
/// writes a design variable, so it is not inlined as a pure expression, and
/// it is called too few times to be compiled: its body is interpreted. Each
/// result is checked against the reduction XOR of the vector under a mask of
/// the chain's labels.
fn chain_design(terms: usize, local: bool, part: bool) -> String {
    const CHAINS: usize = 8;
    let w = if part { 2 } else { 1 };
    let sel = |v: &str, i: usize| {
        if part {
            format!("{v}[{} +: 2]", i)
        } else {
            format!("{v}[{i}]")
        }
    };
    let span = 256 - (w - 1);
    // Distinct labels per chain: 53 is coprime to every span used.
    let d_idx = |k: usize| (0..terms).map(move |t| (17 * k + 53 * t) % span);
    let c_idx = |k: usize| (0..4).map(move |t| (5 * k + 7 * t) % (32 - (w - 1)));
    let rw = CHAINS * w;
    let mut s = String::new();
    s.push_str("module top;\n  reg [31:0] touched;\n  int errors = 0;\n");
    if local {
        s.push_str(&format!(
            "  function automatic [{}:0] fold(input [255:0] din, input [31:0] seed);\n    reg [255:0] d;\n",
            rw - 1
        ));
    } else {
        s.push_str(&format!(
            "  reg [255:0] d;\n  function automatic [{}:0] fold(input [31:0] seed);\n",
            rw - 1
        ));
    }
    s.push_str(&format!(
        "    reg [31:0] c;\n    reg [{}:0] r;\n    begin\n",
        rw - 1
    ));
    s.push_str(if local {
        "      d = din; c = seed;\n"
    } else {
        "      c = seed;\n"
    });
    for k in 0..CHAINS {
        let mut parts: Vec<String> = d_idx(k).map(|i| sel("d", i)).collect();
        parts.extend(c_idx(k).map(|i| sel("c", i)));
        s.push_str(&format!(
            "      r[{} +: {w}] = {};\n",
            k * w,
            parts.join(" ^ ")
        ));
    }
    s.push_str("      touched = seed;\n      fold = r;\n    end\n  endfunction\n");
    // The expected value: for each bit b of a select's width, the XOR of
    // bit `label + b` over the chain's labels.
    s.push_str(&format!(
        "  function automatic [{}:0] want(input [255:0] v, input [31:0] seed);\n    reg [255:0] m; reg [31:0] cm;\n    begin\n      want = 0;\n",
        rw - 1
    ));
    for k in 0..CHAINS {
        for b in 0..w {
            let mut dm = vec![false; 256];
            for i in d_idx(k) {
                dm[i + b] ^= true;
            }
            let mut cm = vec![false; 32];
            for i in c_idx(k) {
                cm[i + b] ^= true;
            }
            let hex = |bits: &[bool]| {
                bits.chunks(4)
                    .rev()
                    .map(|n| {
                        let v = n
                            .iter()
                            .enumerate()
                            .fold(0, |a, (j, &x)| a | ((x as u32) << j));
                        char::from_digit(v, 16).unwrap()
                    })
                    .collect::<String>()
            };
            s.push_str(&format!(
                "      m = 256'h{}; cm = 32'h{};\n      want[{}] = ^(v & m) ^ ^(seed & cm);\n",
                hex(&dm),
                hex(&cm),
                k * w + b
            ));
        }
    }
    s.push_str("    end\n  endfunction\n");
    s.push_str("  reg [255:0] data;\n  reg [31:0] seed;\n  initial begin\n");
    let vectors = [
        (
            "0123456789abcdef_fedcba9876543210_0f1e2d3c4b5a6978_8796a5b4c3d2e1f0",
            "b5834f4c",
        ),
        (
            "ffffffff00000000_aaaaaaaa55555555_deadbeefcafef00d_0000000000000001",
            "00000000",
        ),
        (
            "8000000000000000_0000000000000000_0000000000000000_0000000000000000",
            "ffffffff",
        ),
    ];
    for (k, (v, sd)) in vectors.iter().enumerate() {
        s.push_str(&format!("    data = 256'h{v}; seed = 32'h{sd};\n"));
        if !local {
            s.push_str("    d = data;\n");
        }
        let call = if local {
            "fold(data, seed)"
        } else {
            "fold(seed)"
        };
        s.push_str(&format!(
            "    if ({call} === want(data, seed)) $display(\"PASS vector {k}\");\n    else begin errors++; $display(\"FAIL vector {k}: %h %h\", {call}, want(data, seed)); end\n"
        ));
    }
    s.push_str("    $display(\"DONE: %0d errors\", errors);\n    $finish;\n  end\nendmodule\n");
    s
}

/// The design's name lookups: resolutions plus frame reads.
fn lookups(text: &str) -> u64 {
    let line = text
        .lines()
        .find(|l| l.starts_with("[NAME-STATS]"))
        .unwrap_or_else(|| panic!("missing name stats:\n{text}"));
    ["resolve_hier_name=", "frame_gets="]
        .iter()
        .map(|k| {
            line.split_whitespace()
                .find_map(|tok| tok.strip_prefix(k))
                .and_then(|n| n.parse::<u64>().ok())
                .unwrap_or_else(|| panic!("missing {k}:\n{line}"))
        })
        .sum()
}

/// The lookups of the chain design with `terms` selects per chain, after
/// checking its values.
fn chain_lookups(terms: usize, local: bool, part: bool) -> u64 {
    let text = run(
        &chain_design(terms, local, part),
        &[("XEZIM_NAME_STATS", "1")],
    );
    assert!(
        !text.lines().any(|l| l.starts_with("FAIL")) && text.contains("DONE: 0 errors"),
        "{terms} terms:\n{text}"
    );
    assert_eq!(
        text.lines().filter(|l| l.starts_with("PASS")).count(),
        3,
        "{text}"
    );
    lookups(&text)
}

/// Linear work grows four times from 32 to 128 selects per chain; work in
/// the square of the length, sixteen times (13 times on these designs).
fn assert_linear(local: bool, part: bool) {
    let short = chain_lookups(32, local, part);
    let long = chain_lookups(128, local, part);
    assert!(
        long < 6 * short,
        "local={local} part={part}: {short} lookups at 32 selects, {long} at 128"
    );
}

#[test]
fn chain_of_selects_of_a_local_costs_work_in_proportion_to_its_length() {
    assert_linear(true, false);
    assert_linear(true, true);
}

#[test]
fn chain_of_selects_of_a_design_variable_costs_work_in_proportion_to_its_length() {
    assert_linear(false, false);
}

const WIDTHS: &str = r##"
module top;
  int errors = 0;
  task automatic check(string what, logic [127:0] got, logic [127:0] want);
    if (got === want) $display("PASS %s", what);
    else begin errors++; $display("FAIL %s: got %h, expected %h", what, got, want); end
  endtask

  reg [15:0] n;                       // design variable named like the locals
  logic [1:0][7:0] pa;

  // a local hides a wider design variable of its name: `{n + 8'h01}` is 8 bits
  function automatic [15:0] shadow_width(input [7:0] v);
    reg [7:0] n;
    begin n = v; shadow_width = {n + 8'h01}; end
  endfunction
  // the same for a select of the local
  function automatic [15:0] shadow_sel(input [7:0] v);
    reg [7:0] n;
    begin n = v; shadow_sel = {n[3:0] + 4'h1, n[7:4] + 4'h1}; end
  endfunction
  // selects sum in the assignment context: 3, not 1
  function automatic [7:0] ctx_sum(input [7:0] v);
    reg [7:0] d;
    begin d = v; ctx_sum = d[0] + d[1] + d[2]; end
  endfunction
  // self-determined: `{d[0] + d[1] + d[2]}` is one bit
  function automatic [7:0] self_sum(input [7:0] v);
    reg [7:0] d;
    begin d = v; self_sum = {d[0] + d[1] + d[2]}; end
  endfunction
  // part-selects of an ascending local in a self-determined sum
  function automatic [7:0] part_sum(input [7:0] v);
    reg [0:7] d;
    begin d = v; part_sum = {d[0:3] + d[4 +: 4] + d[7 -: 4]}; end
  endfunction
  // an element of a packed array local is its element width
  function automatic [15:0] packed_elem(input [15:0] v);
    logic [1:0][7:0] p;
    begin p = v; packed_elem = {p[0] + p[1] + 8'h00}; end
  endfunction
  // an element of an unpacked array local
  function automatic [15:0] unpacked_elem(input [7:0] a, input [7:0] b);
    logic [7:0] m [0:1];
    begin m[0] = a; m[1] = b; unpacked_elem = {m[0] + m[1] + 8'h00}; end
  endfunction
  // a typedef'd vector local
  typedef logic [3:0] nib_t;
  function automatic [7:0] typedef_sum(input [3:0] v);
    nib_t t;
    begin t = v; typedef_sum = {t[0] + t[1] + t[3:2]}; end
  endfunction
  // a method of a type-parameterized class
  class P #(type T = logic [7:0]);
    function logic [15:0] f(input T d);
      return {(d[0] + d[1]) + 1'b0};
    endfunction
  endclass
  // a design variable's selects in a function
  function automatic [15:0] module_sel();
    module_sel = {n[3:0] + n[7:4] + 4'h0};
  endfunction
  // an element of a design packed array
  function automatic [15:0] module_pa();
    module_pa = {pa[0] + pa[1] + 8'h00};
  endfunction

  P #(logic [7:0]) pv;
  int k;
  initial begin
    pv = new;
    n = 16'h1234; pa = 16'h0302;
    for (k = 0; k < 3; k++) begin
      check("local hides a design variable, whole", shadow_width(8'hff), 16'h0000);
      check("local hides a design variable, select", shadow_sel(8'hff), 16'h0000);
      check("selects summed in context", ctx_sum(8'h07), 8'h03);
      check("selects summed self-determined", self_sum(8'h07), 8'h01);
      check("part-selects summed", part_sum(8'h12), 8'h05);
      check("packed array element", packed_elem(16'h0102), 16'h0003);
      check("unpacked array element", unpacked_elem(8'h80, 8'h81), 16'h0001);
      check("typedef'd local", typedef_sum(4'b1111), 8'h01);
      check("class method", pv.f(8'h03), 16'h0000);
      check("design variable selects", module_sel(), 16'h0007);
      check("design packed array element", module_pa(), 16'h0005);
    end
    $display("DONE: %0d errors", errors);
    $finish;
  end
endmodule
"##;

#[test]
fn selects_keep_their_own_widths() {
    for env in [&[][..], &[("XEZIM_COMPILE_METHODS", "0")][..]] {
        let text = run(WIDTHS, env);
        let fails: Vec<&str> = text.lines().filter(|l| l.starts_with("FAIL")).collect();
        assert!(fails.is_empty(), "{env:?}: {fails:#?}\n{text}");
        assert_eq!(
            text.lines().filter(|l| l.starts_with("PASS")).count(),
            33,
            "{env:?}:\n{text}"
        );
    }
}
