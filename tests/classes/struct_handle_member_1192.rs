//! IEEE 1800-2023 §7.2, §8.4: a class-handle member of an unpacked-struct
//! class property is a handle like any other. `r.h = new();` constructs the
//! member's class, and the object's properties are read and written through
//! it, in a method and from outside the class (`o.r.h.k`), also for a nested
//! struct, an array of handles and an array of structs. Before, the
//! constructor did not run and a write through the member was dropped
//! (#1192). §18.4: the object of a `rand` handle member is randomized with
//! the object that holds the struct, under its own constraints and those of
//! the holder that read its members (§18.5.9); a handle member that is not
//! `rand` is state. Expected lines were recorded from the reference
//! simulator.

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn handle_member_of_unpacked_struct_property_is_a_handle() {
    let src = r#"
class leaf_c; int v; function new(int a = 3); v = a; endfunction endclass
class rsub_c; rand logic [5:0] k; int arr[2]; leaf_c lf; string s;
  function new(); k = 7; lf = new(4); s = "abc"; endfunction
  function int get(); return k; endfunction
  function void set(int x); k = x; endfunction
endclass
typedef struct { rsub_c h; logic [3:0] t; } rh_t;
typedef struct { rh_t in; rsub_c hs[2]; } outer_t;
class c_c; rh_t r; outer_t o; rh_t ra[2];
  function new(); r.h = new(); endfunction
  function void run();
    $display("T|m1 %0d %0d", r.h == null, r.h.k);
    r.h.k = 9; $display("T|m2 %0d %0d", r.h.k, r.h.get());
    this.r.h.k = 10; $display("T|m3 %0d %0d", this.r.h.k, this.r.h.get());
    r.h.set(12); $display("T|m4 %0d", r.h.k);
    r.h.arr[1] = 5; $display("T|m5 %0d", r.h.arr[1]);
    r.h.lf.v = 6; $display("T|m6 %0d", r.h.lf.v);
    r.h.s = "xy"; $display("T|m7 %s", r.h.s);
    r.h.k++; $display("T|m8 %0d", r.h.k);
    r.h.k += 2; $display("T|m9 %0d", r.h.k);
    o.in.h = new(); o.in.h.k = 21; $display("T|m10 %0d", o.in.h.k);
    o.hs[1] = new(); o.hs[1].k = 22; $display("T|m11 %0d %0d", o.hs[1].k, o.hs[0] == null);
    ra[1].h = new(); ra[1].h.k = 23; $display("T|m12 %0d %0d", ra[1].h.k, ra[0].h == null);
    r.h = null; $display("T|m13 %0d", r.h == null);
  endfunction
endclass
module top;
  c_c c = new;
  rsub_c x;
  initial begin
    c.run();
    c.r.h = new();
    $display("T|t1 %0d %0d", c.r.h == null, c.r.h.k);
    x = c.r.h; x.k = 32; $display("T|t4 %0d", c.r.h.k);
    c.r.h.lf.v = 33; $display("T|t5 %0d", c.r.h.lf.v);
    c.o.in.h.k = 34; $display("T|t6 %0d", c.o.in.h.k);
    c.o.hs[1].k = 35; $display("T|t7 %0d", c.o.hs[1].k);
    c.ra[1].h.k = 36; $display("T|t8 %0d", c.ra[1].h.k);
    c.r.h.arr[0] = 37; $display("T|t9 %0d", c.r.h.arr[0]);
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "T|m1 0 7",
            "T|m2 9 9",
            "T|m3 10 10",
            "T|m4 12",
            "T|m5 5",
            "T|m6 6",
            "T|m7 xy",
            "T|m8 13",
            "T|m9 15",
            "T|m10 21",
            "T|m11 22 1",
            "T|m12 23 1",
            "T|m13 1",
            "T|t1 0 7",
            "T|t4 32",
            "T|t5 33",
            "T|t6 34",
            "T|t7 35",
            "T|t8 36",
            "T|t9 37"
        ]
    );
}

#[test]
fn rand_handle_member_of_unpacked_struct_property_is_randomized() {
    let src = r#"
class rsub_c; rand logic [5:0] k; rand logic [3:0] m;
  constraint cm { m inside {[2:3]}; }
endclass
typedef struct { rand rsub_c h; rand logic [3:0] t; } rh_t;
typedef struct { rsub_c h; rand logic [3:0] t; } nh_t;
class r4_c; rand rh_t r;
  function new(); r.h = new(); endfunction
  constraint c { r.h.k == 6'd5; }
endclass
class r5_c; rand rh_t r;
  function new(); r.h = new(); endfunction
  constraint c { r.h.k inside {[10:12]}; r.t == r.h.m; }
endclass
class r6_c; rand nh_t r;   // handle member not rand: object not randomized, k is state
  function new(); r.h = new(); r.h.k = 9; r.h.m = 1; endfunction
  constraint c { r.t == r.h.k[3:0]; }
endclass
class r7_c; rand rh_t r;   // null handle member
  constraint c { r.t == 4'd6; }
endclass
module top;
  r4_c r4 = new; r5_c r5 = new; r6_c r6 = new; r7_c r7 = new;
  int ok, bad;
  initial begin
    ok = r4.randomize();
    $display("T|r4 ok=%0d null=%0d k=%0d m_ok=%0d", ok, r4.r.h == null, r4.r.h.k, r4.r.h.m inside {[2:3]});
    bad = 0;
    repeat (20) begin
      ok = r5.randomize();
      if (!ok || !(r5.r.h.k inside {[10:12]}) || r5.r.t != r5.r.h.m || !(r5.r.h.m inside {[2:3]})) bad++;
    end
    $display("T|r5 bad=%0d", bad);
    ok = r6.randomize();
    $display("T|r6 ok=%0d k=%0d m=%0d t=%0d", ok, r6.r.h.k, r6.r.h.m, r6.r.t);
    ok = r7.randomize();
    $display("T|r7 ok=%0d null=%0d t=%0d", ok, r7.r.h == null, r7.r.t);
    ok = r4.randomize() with { r.h.m == 3; };
    $display("T|r4w ok=%0d k=%0d m=%0d", ok, r4.r.h.k, r4.r.h.m);
    ok = r4.randomize() with { r.h.k == 6; };
    $display("T|r4x ok=%0d", ok);
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "T|r4 ok=1 null=0 k=5 m_ok=1",
            "T|r5 bad=0",
            "T|r6 ok=1 k=9 m=1 t=9",
            "T|r7 ok=1 null=1 t=6",
            "T|r4w ok=1 k=5 m=3",
            "T|r4x ok=0"
        ]
    );
}
