//! §7.2 / §7.5 / §21.2.1.7: a member path of a child module's struct
//! variable (`u.s.d[0].f0.d`) is typed by walking the variable's declared
//! type. The walk took the first dotted segment (`u`, the instance) as the
//! variable, found nothing, and fell back to the type of the CONTAINER the
//! last select indexes (`u.s.d`), so a dynamic member `d` of an inner struct
//! took the element type of the outer struct's same-named member `d` — it
//! printed (and copied) as an array of the outer element struct. The same
//! module run as the top was correct.

fn output(src: &str) -> Vec<String> {
    let result = xezim::simulate(src, 1_000_000).expect("simulation failed");
    assert_eq!(result.error_count, 0, "unexpected simulation error");
    result
        .output
        .iter()
        .filter_map(|o| o.message.strip_prefix("T|").map(str::to_string))
        .collect()
}

#[test]
fn same_named_dynamic_members_keep_their_own_types() {
    let got = output(
        r#"
module sub;
  typedef struct { longint l; longint r; bit dn; byte unsigned d[]; } va;
  typedef struct { va f0; va f1; } rec;
  typedef struct { longint l; longint r; bit dn; rec d[]; } vr;
  vr s;
  initial begin
    rec e;
    e.f0.d = new[2]; e.f1.d = new[2]; e.f0.l = 1;
    s.d = new[2];
    s.d[0] = e;
    $display("T|%p", s);
  end
endmodule
module top; sub dut(); endmodule
"#,
    );
    assert_eq!(
        got,
        [
            "'{l:0, r:0, dn:0, d:'{'{f0:'{l:1, r:0, dn:0, d:'{0, 0}}, f1:'{l:0, r:0, dn:0, d:'{0, 0}}}, '{f0:'{l:0, r:0, dn:0, d:'{}}, f1:'{l:0, r:0, dn:0, d:'{}}}}}"
        ],
        "{got:?}"
    );
}

#[test]
fn colliding_queue_and_fixed_members_three_levels_deep() {
    let got = output(
        r#"
module sub;
  typedef struct { int k; byte unsigned d[$]; shortint x[2]; } lv0;
  typedef struct { lv0 a; lv0 b; int d[$]; } lv1;
  typedef struct { int k; lv1 d[]; lv0 x[2]; } lv2;
  typedef struct { lv2 d[$]; int x; } lv3;
  lv3 top3;
  lv2 m2;
  initial begin
    lv1 e;
    lv2 f;
    e.a.d.push_back(8'h7); e.a.d.push_back(8'h8);
    e.b.d.push_back(8'h9);
    e.d.push_back(100);
    e.a.x[1] = 5;
    f.d = new[1];
    f.d[0] = e;
    f.x[1].d.push_back(8'h4);
    f.x[0].x[0] = 3;
    top3.d.push_back(f);
    m2 = f;
    m2.d[0].a.d.push_back(8'h1);
    $display("T|e %p", e);
    $display("T|top3 %p", top3);
    $display("T|m2 %p", m2);
    $display("T|sz %0d %0d %0d %0d", top3.d[0].d[0].a.d.size(), m2.d[0].a.d.size(), top3.d[0].x[1].d.size(), m2.d[0].d.size());
  end
endmodule
module top; sub dut(); endmodule
"#,
    );
    assert_eq!(
        got,
        [
            "e '{a:'{k:0, d:'{7, 8}, x:'{0, 5}}, b:'{k:0, d:'{9}, x:'{0, 0}}, d:'{100}}",
            "top3 '{d:'{'{k:0, d:'{'{a:'{k:0, d:'{7, 8}, x:'{0, 5}}, b:'{k:0, d:'{9}, x:'{0, 0}}, d:'{100}}}, x:'{'{k:0, d:'{}, x:'{3, 0}}, '{k:0, d:'{4}, x:'{0, 0}}}}}, x:0}",
            "m2 '{k:0, d:'{'{a:'{k:0, d:'{7, 8, 1}, x:'{0, 5}}, b:'{k:0, d:'{9}, x:'{0, 0}}, d:'{100}}}, x:'{'{k:0, d:'{}, x:'{3, 0}}, '{k:0, d:'{4}, x:'{0, 0}}}}",
            "sz 2 3 1 1",
        ],
        "{got:?}"
    );
}
