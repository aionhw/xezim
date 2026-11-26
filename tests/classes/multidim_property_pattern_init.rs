//! IEEE 1800-2023 §10.9.1, §8.7: an assignment pattern initializing a
//! multidimensional fixed-size class property spreads over every dimension,
//! with nested `default:`, index keys and type keys, in the declared
//! direction, and for a dimension sized by a class parameter.

#[test]
fn multidimensional_class_property_initializers() {
    let sim = xezim::simulate(
        r#"
module top;
  class K #(int N = 2);
    int a2[2][2] = '{default: '{0: 1, default: 2}};
    int a3[2][2] = '{'{1, 2}, '{3, 4}};
    int b2[1:0][0:1] = '{'{1, 2}, '{3, 4}};
    int l2[2][2] = '{default: 10};
    int p2[N][2] = '{default: 7};
    int c3[2][2][2] = '{default: '{default: '{0: 5, 1: 6}}};
    byte t2[2][2] = '{byte: 9};
  endclass
  class D extends K #(3);
    int d2[2][2] = '{1: '{8, 9}, default: '{0, 0}};
  endclass
  K k; D d;
  initial begin
    k = new; d = new;
    $display("T|a2=%p a3=%p", k.a2, k.a3);
    $display("T|b2=%0d %0d %0d %0d l2=%p", k.b2[1][0], k.b2[1][1], k.b2[0][0], k.b2[0][1], k.l2);
    $display("T|p2=%p c3=%p t2=%p", k.p2, k.c3, k.t2);
    $display("T|d.p2=%p d2=%p d.a2=%p", d.p2, d.d2, d.a2);
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|a2='{'{1, 2}, '{1, 2}} a3='{'{1, 2}, '{3, 4}}",
            "T|b2=1 2 3 4 l2='{'{10, 10}, '{10, 10}}",
            "T|p2='{'{7, 7}, '{7, 7}} c3='{'{'{5, 6}, '{5, 6}}, '{'{5, 6}, '{5, 6}}} t2='{'{9, 9}, '{9, 9}}",
            "T|d.p2='{'{7, 7}, '{7, 7}, '{7, 7}} d2='{'{0, 0}, '{8, 9}} d.a2='{'{1, 2}, '{1, 2}}",
        ]
    );
}
