//! Checkpoint gating visits shared source once and ignores runtime value tables.

use super::{Walk, scan_sources};

#[test]
fn checkpoint_scan_deduplicates_deferred_bodies() {
    let source = r#"
module leaf; initial begin #1; end endmodule
module top; leaf u[2000](); endmodule
"#;
    let (_, module) =
        xezim_core::parse_and_elaborate_multi(&[source.into()], Some("top"), &[], &[], &[])
            .unwrap();
    assert_eq!(module.pending_initial.len(), 2000);
    let mut walk = Walk::default();
    scan_sources(&module, &mut walk).unwrap();
    assert!(
        walk.seen.len() < 10,
        "shared syntax was revisited {} times",
        walk.seen.len()
    );
}

#[test]
fn checkpoint_scan_ignores_runtime_data_and_finds_uncalled_bodies() {
    for checkpoint in ["$save", "$incsave", "$restart"] {
        let source = format!(
            r#"
module leaf;
  task restore_later; if (0) {checkpoint}("x.ckpt"); endtask
  initial begin #1; end
endmodule
module top; leaf u(); endmodule
"#
        );
        let (_, module) =
            xezim_core::parse_and_elaborate_multi(&[source], Some("top"), &[], &[], &[]).unwrap();
        assert!(super::in_design(&module), "{checkpoint}");
    }
    let (_, mut module) = xezim_core::parse_and_elaborate_multi(
        &["module top; string text=\"$restart\"; initial #1; endmodule".into()],
        Some("top"),
        &[],
        &[],
        &[],
    )
    .unwrap();
    for n in 0..10_000 {
        let mut signal = module.signals["text"].clone();
        signal.name = format!("data{n}").into();
        module.signals.insert(format!("data{n}"), signal);
    }
    let mut walk = Walk::default();
    scan_sources(&module, &mut walk).unwrap();
    assert!(walk.seen.len() < 10);
}
