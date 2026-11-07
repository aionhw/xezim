//! Scope randomize over array chunks with dist, bounds, and affine offsets,
//! and factorized solve with top-level array element pins.
//!
//! Exercises:
//! - `std::randomize(chunk) with { foreach(chunk[i]) ... }` with dist, affine
//!   constraints (`chunk[i] + start inside { [0:255] }`), and relational bounds.
//! - Class randomize factorized solve with top-level array element pins
//!   (`p08[0].member == val`) in inline `with` blocks.

use xezim::simulate;

fn run(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn test_std_randomize_chunked_affine_dist() {
    let src = r#"
module top;
   int unsigned chunk [1024];
   int lim = 128;
   int start = 3;
   int failures = 0;
   int lo_cnt = 0;

   initial begin
      for (int c = 0; c < 2; c++) begin
         if (std::randomize(chunk) with {
            foreach (chunk[i]) {
               chunk[i] dist { [0:lim] :/ 9, [lim:2*lim] :/ 2 };
               chunk[i] < 255;
               (chunk[i]) inside { [0 : 255] };
               (chunk[i] + start) inside { [0 : 255] };
            }
         }) begin
            foreach (chunk[i]) begin
               if (chunk[i] >= 255 || (chunk[i] + start) > 255)
                  failures++;
               if (chunk[i] <= lim)
                  lo_cnt++;
            end
         end else begin
            failures++;
         end
      end
      if (failures == 0 && lo_cnt > 1400 && lo_cnt < 1900)
         $display("T|PASS chunked_dist");
      else
         $display("T|FAIL chunked_dist failures=%0d lo_cnt=%0d", failures, lo_cnt);
      $finish;
   end
endmodule
"#;
    assert_eq!(run(src), ["T|PASS chunked_dist"]);
}

#[test]
fn test_class_factorized_with_pinned_array_elem() {
    let src = r#"
typedef struct packed {
   logic [31:0] data;
   logic [3:0]  tag;
} elem_t;

class pkt;
   rand bit [7:0] len;
   rand elem_t arr [];

   constraint c_sz {
      arr.size() == len + 1;
      len <= 15;
   }
   constraint c_fe {
      foreach (arr[j]) {
         arr[j].tag inside { [1:14] };
      }
   }
endclass

module top;
   pkt p;
   int failures = 0;

   initial begin
      for (int k = 0; k < 20; k++) begin
         p = new();
         if (p.randomize() with {
            len == 3;
            arr[0].data == 32'hDEAD_BEEF;
            arr[0].tag == 4'h7;
         }) begin
            if (p.arr.size() != 4) failures++;
            if (p.arr[0].data !== 32'hDEAD_BEEF) failures++;
            if (p.arr[0].tag !== 4'h7) failures++;
            for (int j = 1; j < 4; j++) begin
               if (p.arr[j].tag < 1 || p.arr[j].tag > 14) failures++;
            end
         end else begin
            failures++;
         end
      end
      if (failures == 0)
         $display("T|PASS pinned_elem");
      else
         $display("T|FAIL pinned_elem failures=%0d", failures);
      $finish;
   end
endmodule
"#;
    assert_eq!(run(src), ["T|PASS pinned_elem"]);
}
