//! Keep engine policy tests independent of process-wide cached settings.

use std::cell::Cell;
use std::process::Command;

#[derive(Clone, Copy)]
struct ChildTestRoute {
    test: &'static str,
    case: &'static str,
}

thread_local! {
    static CHILD_TEST_ROUTE: Cell<Option<ChildTestRoute>> = const { Cell::new(None) };
}

/// Grouped cases keep their original thread name, while policy subprocesses
/// select the registered group and then that case. Restore the route on panic
/// too; other test threads and test targets retain their original behavior.
#[allow(dead_code)] // This helper is also included by the strings test target.
pub(super) fn with_uvm_case_route(test: &'static str, case: &'static str, run: fn()) {
    struct Restore(Option<ChildTestRoute>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CHILD_TEST_ROUTE.with(|route| route.set(self.0));
        }
    }
    let previous =
        CHILD_TEST_ROUTE.with(|route| route.replace(Some(ChildTestRoute { test, case })));
    let _restore = Restore(previous);
    run();
}

pub(super) fn eager() -> bool {
    policies(&[("1", "0")])
}

pub(super) fn policies(settings: &[(&str, &str)]) -> bool {
    let thread = std::thread::current();
    let name = thread.name().expect("named test thread");
    if std::env::var("XEZIM_TEST_METHOD_CHILD").as_deref() == Ok(name) {
        return true;
    }
    let route = CHILD_TEST_ROUTE.with(Cell::get);
    let selector = route.map_or(name, |route| route.test);
    for &(enabled, tier) in settings {
        let mut command = Command::new(std::env::current_exe().expect("test binary"));
        command
            .args(["--exact", selector, "--nocapture", "--test-threads=1"])
            .env("XEZIM_TEST_METHOD_CHILD", name)
            .env("XEZIM_COMPILE_METHODS", enabled)
            .env("XEZIM_METHOD_TIER", tier)
            .env_remove("XEZIM_METHOD_CACHE");
        if let Some(route) = route {
            command.env("XEZIM_UVM_CASE", route.case);
        }
        let output = command.output().expect("run isolated policy test");
        assert!(
            output.status.success(),
            "{name}: enabled={enabled}, tier={tier}\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "isolated test filter did not run {name}"
        );
    }
    false
}
