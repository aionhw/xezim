//! Temporary formal-conversion storage must not become a class-object root.
use super::*;

#[test]
fn scratch_reuse_releases_values_and_preserves_returned_handle_clones() {
    let mut sim = crate::simulate(
        r#"
class Node; int id; function new(int n);id=n;endfunction endclass
typedef struct {Node node; int count;} Packet;
module top;
 Node first, second;
 initial begin first=new(777);second=new(888);$finish;end
endmodule
"#,
        20,
    )
    .expect("scratch lifetime fixture");
    let DataType::Struct(packet) = sim.module.typedef_types.get("Packet").unwrap().clone() else {
        panic!("Packet must be an unpacked struct");
    };
    let first = sim.get_signal_value_by_name("first").unwrap();
    let second = sim.get_signal_value_by_name("second").unwrap();
    let number = |value: &Value| {
        Expression::new(
            ExprKind::Number(NumberLiteral::Integer {
                size: Some(value.width),
                signed: false,
                base: NumberBase::Binary,
                value: value.to_bin(),
                cached_val: Cell::new(None),
            }),
            crate::ast::Span::dummy(),
        )
    };
    let pattern = |handle: &Value, count: u64| {
        let name = |s: &str| crate::ast::Identifier {
            name: s.into(),
            span: crate::ast::Span::dummy(),
        };
        Expression::new(
            ExprKind::AssignmentPattern(vec![
                AssignmentPatternItem::Named(name("node"), number(handle)),
                AssignmentPatternItem::Named(name("count"), number(&Value::from_u64(count, 32))),
            ]),
            crate::ast::Span::dummy(),
        )
    };
    let keys = vec![
        ("p.node".into(), first.width, false),
        ("p.count".into(), 32, false),
    ];
    let bound_first = sim
        .struct_pattern_leaf_values("p", &packet, &pattern(&first, 55), &keys)
        .unwrap();
    assert_eq!(bound_first[0], first);
    assert_eq!(bound_first[1].to_u64(), Some(55));
    let bound_second = sim
        .struct_pattern_leaf_values("p", &packet, &pattern(&second, 66), &keys)
        .unwrap();
    assert_eq!(bound_second[0], second);
    assert_eq!(bound_second[1].to_u64(), Some(66));
    assert_eq!(
        bound_first[0], first,
        "scratch reuse changed a returned clone"
    );
    assert_eq!(
        sim.pattern_scratch_seq, 1,
        "scratch allocation was not reused"
    );
    assert!(
        !sim.signals
            .keys()
            .any(|key| key.starts_with("\u{2}pat_arg_")),
        "returned formal values remained owned by scratch signals"
    );
}

#[test]
fn named_struct_formal_returns_handle_without_retaining_the_temporary_copy() {
    let sim = crate::simulate(
        r#"
class Node;int id;function new(int n);id=n;endfunction endclass
typedef struct {Node node; int pad[1];} Packet;
function Node echo(input Packet packet);return packet.node;endfunction
module top;
 Node original, returned, owner;
 Packet __pat_arg_1;
 initial begin
  original=new(777);owner=new(111);
  __pat_arg_1.node=owner;__pat_arg_1.pad[0]=99;
  returned=echo('{node:original,pad:'{55}});
  original=null;
  $display("T|bound|%0d|%0d|%0d",returned.id,__pat_arg_1.node.id,__pat_arg_1.pad[0]);
  $finish;
 end
endmodule
"#,
        20,
    )
    .expect("named struct handle fixture");
    assert_eq!(sim.error_count, 0);
    assert!(sim
        .output
        .iter()
        .any(|line| line.message == "T|bound|777|111|99"));
    assert!(
        sim.pattern_scratch_seq > 0,
        "formal did not use scratch conversion"
    );
    assert!(
        !sim.signals
            .keys()
            .any(|key| key.starts_with("\u{2}pat_arg_")),
        "completed callback retained a temporary owner"
    );
}
