//! §25.5.4 modport EXPRESSIONS: `modport lo8 (output .b(word[7:0]), input
//! .hi(word[31:24]), input .pair({t, word[7:0]}))`. The member stands for an
//! expression over the interface's signals; reads substitute it, writes land
//! in the underlying bits, and a bit-select of the member (`p.b[0]`) folds to
//! a bit-select of the base. Reference-validated (Q1/Q2/Q3 byte-for-byte).
use std::path::PathBuf;
use std::process::Command;

const DESIGN: &str = r#"
interface ifc;
  logic [31:0] word;
  logic [7:0] t;
  modport lo8 (output .b(word[7:0]), input .hi(word[31:24]), input .pair({t, word[7:0]}), output t);
  modport full (input word);
endinterface
module writer(ifc.lo8 p);
  initial begin
    #1 p.b = 8'hab; p.t = p.hi;
    #1 $display("Q1 t=%h hi=%h b=%h b3=%b pair=%h", p.t, p.hi, p.b, p.b[3], p.pair);
    #1 p.b[0] = 1'b0;
    #1 $display("Q2 b=%h", p.b);
  end
endmodule
module reader(ifc.full q);
  initial #5 $display("Q3 word=%h", q.word);
endmodule
module top;
  ifc i();
  writer w(.p(i));
  reader r(.q(i));
  initial begin i.word = 32'hcd0000cd; #6 $finish; end
endmodule
"#;

#[test]
fn modport_expression_members_read_write_and_select() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("modport_expression_ports");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("top.sv");
    std::fs::write(&src, DESIGN).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "top", src.to_str().unwrap(), "--no-cache"])
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "run failed:\n{text}");
    for want in ["Q1 t=cd hi=cd b=ab b3=1 pair=cdab", "Q2 b=aa", "Q3 word=cd0000aa"] {
        assert!(text.contains(want), "missing `{want}`:\n{text}");
    }
}
