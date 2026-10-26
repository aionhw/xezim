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
