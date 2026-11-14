//! Annex D.9 / §8.3 / #1116: derived property indexes share restored classes.

use super::*;

#[test]
fn checkpoint_rebuilds_property_owners_without_serializing_class_copies() {
    let mut sim = crate::simulate(
        r#"
class Base;
  int inherited = 3;
  int shadowed = 5;
endclass
class Derived extends Base;
  int own = 7;
  int shadowed = 11;
endclass
module top;
  Base b;
  Derived d;
  initial begin
    b = new;
    d = new;
    #1;
    if (b.inherited != 3 || d.own != 7 || d.shadowed != 11) $fatal(1);
    $finish;
  end
endmodule
"#,
        10,
    )
    .unwrap();
    assert_eq!(sim.error_count, 0);
    sim.prop_owners("Base");
    let owners = sim.prop_owners("Derived");
    assert_eq!(owners["inherited"].0, "Base");
    assert_eq!(owners["shadowed"].0, "Derived");
    assert_eq!(sim.prop_owner_index.borrow().len(), 2);

    let mut fields = Vec::new();
    with_session(|| sim.checkpoint_encode_state(&mut fields)).unwrap();
    assert!(
        fields.iter().all(|(name, _)| name != "prop_owner_index"),
        "the derived index must not encode a class copy per property"
    );
    let path =
        std::env::temp_dir().join(format!("xezim_property_cache_{}.ckpt", std::process::id()));
    sim.checkpoint_save(path.to_str().unwrap()).unwrap();
    // Saving must not evict the uninterrupted run's derived cache.
    assert_eq!(sim.prop_owner_index.borrow().len(), 2);
    sim.checkpoint_restore(path.to_str().unwrap()).unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(sim.prop_owner_index.borrow().is_empty());
    for name in ["Base", "Derived"] {
        let owners = sim.prop_owners(name);
        for (declaring, class) in owners.values() {
            assert!(Arc::ptr_eq(class, &sim.module.classes[declaring]));
        }
    }
    let owners = sim.prop_owners("Derived");
    assert_eq!(owners["inherited"].0, "Base");
    assert_eq!(owners["shadowed"].0, "Derived");
    assert_eq!(owners["own"].0, "Derived");
}

#[test]
fn obsolete_checkpoint_format_is_rejected_before_decoding() {
    let mut sim = crate::simulate("module top; initial $finish; endmodule", 10).unwrap();
    let path =
        std::env::temp_dir().join(format!("xezim_obsolete_format_{}.ckpt", std::process::id()));
    let mut obsolete = *MAGIC;
    obsolete[7] = obsolete[7].wrapping_sub(1);
    std::fs::write(&path, obsolete).unwrap();
    let error = sim.checkpoint_restore(path.to_str().unwrap()).unwrap_err();
    std::fs::remove_file(path).unwrap();
    assert_eq!(error, "written in another checkpoint format");
}
