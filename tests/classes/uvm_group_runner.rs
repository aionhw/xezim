//! Fresh-thread execution and failure collection for the grouped UVM cases.

#[derive(Clone, Copy)]
pub(super) struct Case {
    pub(super) name: &'static str,
    pub(super) run: fn(),
}

pub(super) struct Failure {
    pub(super) case: &'static str,
    pub(super) message: String,
}

pub(super) fn collect_failures(group: &'static str, cases: &[Case]) -> Vec<Failure> {
    let mut failures = Vec::new();
    for &case in cases {
        eprintln!("[uvm case] {}", case.name);
        // libtest previously supplied a fresh thread to every case. Keep that
        // TLS boundary, rather than sharing simulator state between cases.
        let result = std::thread::Builder::new()
            .name(case.name.to_string())
            .spawn(move || {
                crate::compiled_method_test_env::with_uvm_case_route(group, case.name, case.run);
            });
        match result {
            Ok(thread) => {
                if let Err(payload) = thread.join() {
                    let message = if let Some(message) = payload.downcast_ref::<String>() {
                        message.clone()
                    } else if let Some(message) = payload.downcast_ref::<&str>() {
                        (*message).to_string()
                    } else {
                        "non-string panic payload".to_string()
                    };
                    failures.push(Failure {
                        case: case.name,
                        message,
                    });
                }
            }
            Err(error) => failures.push(Failure {
                case: case.name,
                message: format!("could not start case thread: {error}"),
            }),
        }
    }
    failures
}
