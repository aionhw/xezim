//! Element names of named 1-D arrays are virtual (`NameMap`): an array of
//! at least 257 cells stores only its base name, and `base[idx]` resolves
//! arithmetically. Every by-name path must still reach the element: event
//! controls, continuous-assign dependencies on a constant element,
//! hierarchical element references, force/release, waveform dumps.
use std::process::Command;
use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
}

#[test]
fn virtual_element_names_reach_every_by_name_path() {
    const SRC: &str = r#"
module sub;
  logic [15:0] regs [0:299];
endmodule
module top;
  logic [15:0] regs [0:299];
  logic [11:0] mem [0:299];
  real         rmem [0:299];
  logic [11:0] tap, seen, forced;
  logic [15:0] hier;
  int hits = 0;
  sub u();
  assign tap = mem[5];
  always @(mem[3]) hits++;
  initial begin
    mem[5] = 12'h123;
    rmem[5] = 3.5;
    #1 seen = tap;
    mem[3] = 12'h001;
    #1 mem[3] = 12'h002;
    #1 u.regs[7] = 16'hbeef;
    #1 hier = u.regs[7];
    force mem[9] = 12'h0aa;
    mem[9] = 12'h000;
    #1 forced = mem[9];
    release mem[9];
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(
        u(&sim, "seen"),
        0x123,
        "continuous assign of a constant element"
    );
    assert_eq!(u(&sim, "hits"), 2, "event control on an element");
    assert_eq!(u(&sim, "hier"), 0xbeef, "hierarchical element reference");
    assert_eq!(u(&sim, "forced"), 0xaa, "force on an element");
    assert_eq!(
        sim.get_signal_value("mem[5]")
            .or_else(|| sim.get_signal_value("top.mem[5]"))
            .and_then(|v| v.to_u64()),
        Some(0x123),
        "a packed element's value resolves by name"
    );
    assert_eq!(
        sim.get_signal("rmem[5]")
            .or_else(|| sim.get_signal("top.rmem[5]"))
            .map(|v| v.to_f64()),
        Some(3.5),
        "an element's virtual name resolves"
    );
}

#[test]
fn virtual_element_names_match_stored_names() {
    // The same program with every named array virtual and with none: the
    // transcript, VCD included, must not change.
    const SRC: &str = r#"
module vnames;
  logic [7:0] big [0:299];
  logic [7:0] tiny [0:3];
  logic [7:0] y;
  int i;
  assign y = big[2] ^ tiny[1];
  initial begin
    $dumpfile("vnames.vcd");
    $dumpvars(0, vnames);
    for (i = 0; i < 300; i++) big[i] = i[7:0];
    tiny[1] = 8'hf0;
    #1 $display("y=%h big7=%h", y, big[7]);
    big[2] = 8'h0f;
    #1 $display("y=%h", y);
    $finish;
  end
endmodule
"#;
    let dir = std::env::temp_dir().join(format!("xezim_vnames_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("vnames.sv");
    std::fs::write(&source, SRC).unwrap();
    let run = |min_cells: &str| {
        let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
            .args(["--no-cache", "--wave", "-s", "vnames", "--max-time", "100"])
            .arg(&source)
            .current_dir(&dir)
            .env("XEZIM_VIRTUAL_NAME_MIN_CELLS", min_cells)
            .output()
            .expect("run xezim");
        assert!(out.status.success(), "run failed: {:?}", out);
        let vcd = std::fs::read_to_string(dir.join("vnames.vcd")).unwrap_or_default();
        // The date line differs run to run.
        let vcd: String = vcd
            .lines()
            .skip_while(|l| !l.starts_with("$version"))
            .collect::<Vec<_>>()
            .join("\n");
        (String::from_utf8_lossy(&out.stdout).into_owned(), vcd)
    };
    let (all_out, all_vcd) = run("0");
    let (none_out, none_vcd) = run("1000000");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        all_out.contains("y=f2 big7=07"),
        "virtual run:\n{}",
        all_out
    );
    assert_eq!(all_out, none_out);
    assert!(
        all_vcd.contains("big[299]"),
        "elements are dumped:\n{}",
        all_vcd
    );
    assert_eq!(all_vcd, none_vcd);
}
