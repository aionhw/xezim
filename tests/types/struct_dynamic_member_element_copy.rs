//! §7.2.1 / §7.6: copying an unpacked struct copies its dynamic members by
//! value, and an element of such a member that is itself an unpacked struct
//! is copied member by member — including the element's own dynamic
//! members. Only the element cell (`d[j]`) was copied, which an unpacked
//! struct element does not have, so `m3 = m2` left `m3.d[0].a.d` empty.

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
fn nested_dynamic_members_copy_by_value() {
    let got = output(
        r#"
module top;
  typedef struct { int k; byte unsigned d[$]; } lv0;
  typedef struct { lv0 a; int d[$]; } lv1;
  typedef struct { int k; lv1 d[]; } lv2;
  lv2 m2, m3;
  lv2 q[$];
  initial begin
    lv1 e;
    lv2 f;
    e.a.d.push_back(8'h7); e.d.push_back(100);
    f.d = new[1];
    f.d[0] = e;
    m2.d = new[1];
    m2.d[0] = e;
    m3 = m2;
    $display("T|m3 %p", m3);
    m3 = f;
    m3.d[0].a.d.push_back(8'h8);
    q.push_back(f);
    $display("T|m3 %p f %p", m3, f);
    $display("T|q %p", q);
  end
endmodule
"#,
    );
    assert_eq!(
        got,
        [
            "m3 '{k:0, d:'{'{a:'{k:0, d:'{7}}, d:'{100}}}}",
            "m3 '{k:0, d:'{'{a:'{k:0, d:'{7, 8}}, d:'{100}}}} f '{k:0, d:'{'{a:'{k:0, d:'{7}}, d:'{100}}}}",
            "q '{'{k:0, d:'{'{a:'{k:0, d:'{7}}, d:'{100}}}}}",
        ],
        "{got:?}"
    );
}
