//! §18.4, §18.5.8.1, §18.5.9: IPv4-style header + packet where the header has
//! its own dynamic `options[]` array sized from `ihl`, and the enclosing packet
//! constrains `data[]` and `pad[]` sizes from `header.total_length` and
//! `header.ihl`.

use std::time::{Duration, Instant};
use xezim::simulate;

#[test]
fn ipv4_header_and_packet_all_five_relations_hold() {
    let src = r#"
class hdr;
  rand bit [3:0]  ihl;
  rand bit [15:0] total_length;
  rand bit [31:0] options[];
  constraint ihl_c { ihl >= 5; }
  constraint total_length_c { total_length >= (ihl * 4) & total_length <= 576; }
  constraint options_c { solve ihl before options; options.size() == (ihl - 5); }
endclass
class pkt;
  rand int min_frame_size;
  rand hdr header;
  rand bit [7:0] data[];
  rand bit [7:0] pad[];
  rand bit use_correct_fcs;
  constraint min_frame_size_c { min_frame_size == 64; }
  constraint use_correct_fcs_c { use_correct_fcs == 1; }
  constraint pad_c {
    solve header.total_length before pad;
    foreach (pad[i]) { pad[i] == 8'hFF; }
    pad.size() == (min_frame_size - ((header.total_length + (2 * (48 / 8)) + 6)));
  }
  constraint data_c { data.size() == (header.total_length - (header.ihl * 4)); }
  function new(); header = new(); min_frame_size = 64; endfunction
endclass
module top;
  initial begin
    pkt p = new();
    int fails = 0;
    for (int i = 0; i < 20; i++) begin
      int ok = p.randomize();
      if (!ok) begin
        fails++;
        $display("FAIL call=%0d", i);
        break;
      end else begin
        if (p.header.ihl < 5 ||
            p.header.total_length > 46 ||
            p.header.total_length < p.header.ihl * 4 ||
            p.header.options.size() != p.header.ihl - 5 ||
            p.data.size() != p.header.total_length - p.header.ihl * 4 ||
            p.pad.size() != 64 - (p.header.total_length + 18) ||
            p.use_correct_fcs != 1 ||
            p.min_frame_size != 64) begin
          fails++;
          $display("BAD tl=%0d ihl=%0d opts=%0d data=%0d pad=%0d",
                   p.header.total_length, p.header.ihl,
                   p.header.options.size(), p.data.size(), p.pad.size());
        end else begin
          foreach (p.pad[j]) if (p.pad[j] !== 8'hFF) fails++;
        end
      end
    end
    if (fails == 0) $display("PASS"); else $display("FAIL (%0d)", fails);
    $finish;
  end
endmodule
"#;
    let res = simulate(src, 100_000).expect("simulate failed");
    let lines: Vec<&str> = res.output.iter().map(|e| e.message.as_str()).collect();
    assert!(lines.contains(&"PASS"), "{lines:#?}");
}

#[test]
fn ipv4_unsatisfiable_when_ihl_too_large_for_frame() {
    let src = r#"
class hdr;
  rand bit [3:0]  ihl;
  rand bit [15:0] total_length;
  rand bit [31:0] options[];
  constraint ihl_c { ihl >= 12; }
  constraint total_length_c { total_length >= (ihl * 4) & total_length <= 576; }
  constraint options_c { solve ihl before options; options.size() == (ihl - 5); }
endclass
class pkt;
  rand int min_frame_size;
  rand hdr header;
  rand bit [7:0] data[];
  rand bit [7:0] pad[];
  constraint min_frame_size_c { min_frame_size == 64; }
  constraint pad_c {
    foreach (pad[i]) pad[i] == 8'hFF;
    pad.size() == (min_frame_size - (header.total_length + 18));
  }
  constraint data_c { data.size() == (header.total_length - (header.ihl * 4)); }
  function new(); header = new(); min_frame_size = 64; endfunction
endclass
module top;
  initial begin
    pkt p = new();
    int ok = p.randomize();
    $display("RES ok=%0d", ok);
    $finish;
  end
endmodule
"#;
    let start = Instant::now();
    let res = simulate(src, 100_000).expect("simulate failed");
    let elapsed = start.elapsed();
    let lines: Vec<&str> = res.output.iter().map(|e| e.message.as_str()).collect();
    assert!(lines.contains(&"RES ok=0"), "{lines:#?}");
    assert!(
        elapsed < Duration::from_secs(2),
        "unsatisfiable ipv4 took too long: {elapsed:?}"
    );
}
