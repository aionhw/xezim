//! IEEE 1800-2023 §7.2.1, §7.3.1: a member of a packed struct or union is a
//! slice of it, also when the packed struct is itself a member of an
//! UNPACKED-struct class property. Such a field (`s.e.key`, `s.u.a`, and
//! `s.item[0].key` / `s.pitem[1].key` of an array member) had no storage of
//! its own and was neither read nor written through its member (#1192).
//! §18.4: the rand members of an unpacked-struct property are random
//! variables of the object, so the same paths are drawn and constrained by
//! `randomize()`, a `foreach` over an array member included, and a rand
//! sub-object's packed-struct array members are constrained through a
//! member path with constant selects (§18.5.9). Expected lines were recorded
//! from the reference simulator.

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| !m.starts_with("[xezim]") && !m.starts_with("Simulation"))
        .collect()
}

#[test]
fn packed_member_of_unpacked_struct_property_is_a_slice() {
    let src = r#"
typedef struct packed { logic valid; logic [5:0] key; logic [3:0] sub; } ent_t;
typedef union packed { logic [5:0] a; logic [1:0][2:0] b; } pun_t;
typedef struct { ent_t e; ent_t item [0:1]; ent_t [0:1] pitem; pun_t u; } s_t;
class a_c;
  s_t s;
  function void t();
    s.e = {1'b0, 6'd5, 4'd0}; $display("e.key=%0d", s.e.key);
    s.e.key = 9; $display("e=%h", s.e);
    s.item[1] = {1'b1, 6'd7, 4'd1}; $display("item1.key=%0d", s.item[1].key);
    s.item[0].key = 3; $display("item0=%h", s.item[0]);
    s.pitem = '0; s.pitem[1].key = 2; $display("pitem=%h pitem1.key=%0d", s.pitem, s.pitem[1].key);
    s.u = 6'd5; $display("u.a=%0d u.b1=%0d", s.u.a, s.u.b[1]);
    s.u.b[0] = 3'd7; $display("u=%h", s.u);
  endfunction
endclass
module t1192b;
  a_c a = new;
  initial begin
    a.t();
    a.s.e.sub = 4'hA; $display("e2=%h sub=%h", a.s.e, a.s.e.sub);
    a.s.pitem[0].valid = 1; $display("pitem2=%h", a.s.pitem);
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "e.key=5",
            "e=090",
            "item1.key=7",
            "item0=X3x",
            "pitem=000020 pitem1.key=2",
            "u.a=5 u.b1=0",
            "u=07",
            "e2=09a sub=a",
            "pitem2=200020",
        ]
    );
}

#[test]
fn rand_members_of_unpacked_struct_property_are_solved() {
    let src = r#"
typedef struct packed { logic valid; logic [5:0] key; logic [3:0] sub; } ent_t;
typedef union packed { logic [5:0] a; logic [1:0][2:0] b; } pun_t;
typedef struct packed { logic [5:0] tag; ent_t [0:2] item; } grp_t;
typedef struct { rand ent_t item [0:2]; rand logic [5:0] tag; } rgrp_t;
typedef struct { rand ent_t [0:2] pitem; } rpug_t;
typedef struct { rand ent_t item [0:1][0:2]; } r2d_t;
typedef struct { rand ent_t e; rand pun_t u; } rm_t;
class u1_c; rand rgrp_t rg;
  constraint c { foreach (rg.item[i]) rg.item[i].key inside {[4:5]}; } endclass
class u3_c; rand rpug_t rp;
  constraint c { foreach (rp.pitem[i]) rp.pitem[i].key inside {[4:5]}; } endclass
class r1_c; rand r2d_t r;
  constraint c { foreach (r.item[i, j]) r.item[i][j].key inside {[4:5]}; } endclass
class r3_c; rand rm_t r;
  constraint c { r.e.key == 6'd5; r.u.a == 6'd9; } endclass
class msub_c; rand grp_t g; endclass
class m1_c; rand msub_c a;
  function new(); a = new(); endfunction
  constraint c { foreach (a.g.item[i]) a.g.item[i].key inside {[0:3]}; a.g.tag == 6'd7; } endclass
module t1192c;
  u1_c u1 = new; u3_c u3 = new; r1_c r1 = new; r3_c r3 = new; m1_c m1 = new;
  int bad [string];
  task automatic check(string tag, bit ok, bit cond);
    if (!ok || !cond) bad[tag]++;
  endtask
  initial begin
    repeat (20) begin
      check("U1", u1.randomize(), u1.rg.item[0].key inside {[4:5]} && u1.rg.item[1].key inside {[4:5]} && u1.rg.item[2].key inside {[4:5]});
      check("U3", u3.randomize(), u3.rp.pitem[0].key inside {[4:5]} && u3.rp.pitem[2].key inside {[4:5]});
      check("R1", r1.randomize(), r1.r.item[0][0].key inside {[4:5]} && r1.r.item[1][2].key inside {[4:5]});
      check("R3", r3.randomize(), r3.r.e.key == 5 && r3.r.u.a == 9);
      check("M1", m1.randomize(), (m1.a.g.item[0].key | m1.a.g.item[1].key | m1.a.g.item[2].key) <= 3 && m1.a.g.tag == 7);
    end
    foreach (bad[t]) $display("FAIL %s %0d", t, bad[t]);
    $display("done %0d", bad.num());
  end
endmodule
"#;
    assert_eq!(lines(src), ["done 0"]);
}
