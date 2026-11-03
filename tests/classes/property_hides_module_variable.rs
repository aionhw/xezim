//! §8.4, §8.5: a class property is the object's storage, whatever module
//! variable shares its name.
//!
//! - In a method, a nonblocking write to a property, `s.f <= v` or
//!   `v[1] <= x`, was scheduled to the same-named module variable, so the
//!   module's value changed and the object's did not. The blocking write and
//!   `this.s.f <= v` were right.
//! - Through a handle, `h.v[1] = x` from module code wrote the module's
//!   array `v` instead of the object's property `v`.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 1_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("R "))
        .collect()
}

#[test]
fn nonblocking_writes_in_a_method_reach_the_property() {
    let out = lines(
        r#"
typedef struct { logic [7:0] f; } s_t;
class C;
  s_t s;
  logic [7:0] x;
  logic [7:0] v [2];
  task run(); s.f <= 8'h33; x <= 8'h66; v[1] <= 8'h77; endtask
  task run_this(); this.s.f <= 8'h44; endtask
  task run_blk(); s.f = 8'h55; endtask
endclass
module tb;
  s_t s;
  logic [7:0] x, v [2];
  C h, h2, h3;
  initial begin
    s.f = 8'h11; x = 8'h22; v[1] = 8'h44;
    h = new; h2 = new; h3 = new;
    h.run(); h2.run_this(); h3.run_blk();
    #1 $display("R %h %h %h | %h %h %h | %h %h", s.f, x, v[1], h.s.f, h.x, h.v[1], h2.s.f, h3.s.f);
  end
endmodule
"#,
    );
    assert_eq!(out, ["R 11 22 44 | 33 66 77 | 44 55"]);
}

#[test]
fn element_write_through_a_handle_reaches_the_property() {
    let out = lines(
        r#"
class C;
  logic [7:0] v [2];
  logic [7:0] w [2];
endclass
module tb;
  logic [7:0] v [2];
  C h;
  initial begin
    v[1] = 8'h44;
    h = new;
    h.v[1] = 8'h01;
    h.w[1] = 8'h02;
    #1 $display("R %h | %h %h", v[1], h.v[1], h.w[1]);
  end
endmodule
"#,
    );
    assert_eq!(out, ["R 44 | 01 02"]);
}
