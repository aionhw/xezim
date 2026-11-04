//! §10.9 / §8.7: a class queue of unpacked structs takes its pattern, and a call reads the old size.
#[test]
fn class_queue_of_string_structs_takes_its_initializer() {
    let result = xezim::simulate(
        r#"
typedef struct { string name; int unsigned sz; bit [2:0] xwr; } region_t;
typedef struct { int a; int b; } inner_t; typedef struct { inner_t n; int c; } outer_t;
class cfg_c;
  region_t named[$] = '{ '{name:"r0", sz: 4096, xwr: 3'b111}, '{name:"r1", sz: 4096 * 16, xwr: 3'b101} };
  region_t pos[$] = '{ '{"p0", 8, 3'b001}, '{"p1", 16, 3'b010} };
  region_t made[$] = '{ '{name:"a", sz:4, xwr:1}, mk() };
  function region_t mk(); static int n; n++; mk.name = "m"; mk.sz = 8 * n; mk.xwr = made.size(); endfunction
  static int calls; outer_t nest[$] = '{ '{n: nk(), c: 7} };
  function inner_t nk(); calls++; nk.a = nest.size(); nk.b = 10; endfunction
endclass
module t; initial begin
  cfg_c c = new(); region_t cp[$];
  $display("named %0d %s/%0d/%0d %s/%0d/%0d", c.named.size(), c.named[0].name, c.named[0].sz, c.named[0].xwr, c.named[1].name, c.named[1].sz, c.named[1].xwr);
  $display("pos %0d %s/%0d/%0d %s/%0d/%0d", c.pos.size(), c.pos[0].name, c.pos[0].sz, c.pos[0].xwr, c.pos[1].name, c.pos[1].sz, c.pos[1].xwr);
  $display("made %0d %s/%0d/%0d %s/%0d/%0d", c.made.size(), c.made[0].name, c.made[0].sz, c.made[0].xwr, c.made[1].name, c.made[1].sz, c.made[1].xwr);
  c.named[1].name = "w1"; cp = c.named;
  $display("copy %0d %s/%0d %s/%0d", cp.size(), cp[0].name, cp[0].sz, cp[1].name, cp[1].sz);
  $display("nest %0d %0d %0d %0d %0d", cfg_c::calls, c.nest.size(), c.nest[0].n.a, c.nest[0].n.b, c.nest[0].c);
  $finish; end endmodule
"#, 100_000).unwrap();
    let lines: Vec<_> = result.output.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(lines, ["named 2 r0/4096/7 r1/65536/5", "pos 2 p0/8/1 p1/16/2", "made 2 a/4/1 m/8/0", "copy 2 r0/4096 w1/65536", "nest 1 1 0 10 7"]);
}
