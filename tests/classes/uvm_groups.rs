//! The 67 original UVM cases, registered as eight logical libtest groups.
//! Case implementations remain in their original modules. Every case gets a
//! fresh thread; failures are collected so later cases still run.

#[path = "uvm_group_runner.rs"]
mod runner;

use runner::Case;

#[derive(Clone, Copy)]
struct Group {
    selector: &'static str,
    cases: &'static [Case],
}

macro_rules! cases {
    ($($module:ident::$case:ident),+ $(,)?) => {
        &[$(Case {
            name: concat!(stringify!($module), "::", stringify!($case)),
            run: crate::$module::$case,
        }),+]
    };
}

const FOUNDATION_CONFIG_FACTORY: Group = Group {
    selector: "uvm_groups::uvm_foundation_config_factory",
    cases: cases![
        uvm_config_db_tests::test_config_db_inst_name,
        uvm_config_db_tests::test_config_db_wildcard,
        uvm_config_db_tests::test_config_db_hit_wildcard_and_miss,
        uvm_config_db_tests::test_config_db_exists_after_set,
        uvm_factory_linkage::unit_scope_extern_method_body_runs_and_persists_writes,
        uvm_factory_linkage::class_local_typedef_resolves_to_underlying_class,
        uvm_integration_tests::uvm_2020_config_db_reaches_build_phase,
        uvm_integration_tests::uvm_2020_factory_type_override,
        uvm_integration_tests::uvm_2020_factory_typedef_chain_honors_override,
        uvm_integration_tests::uvm_factory_overrides_and_by_name,
        uvm_resource_db_raw_type_roundtrip::uvm_resource_db_raw_type_roundtrip,
    ],
};

const PHASING_VERSIONS: Group = Group {
    selector: "uvm_groups::uvm_phasing_versions",
    cases: cases![
        uvm_feature_coverage::uvm_phases_run_in_hierarchy_order,
        uvm_feature_coverage::uvm_objection_drain_time_extends_the_phase,
        uvm_feature_coverage::uvm_global_timeout_ends_a_hung_run_phase,
        uvm_genuine_2017::objection_gated_run_phase_reaches_target_time,
        uvm_integration_tests::uvm_1_2_bare_run_phase_completes,
        uvm_integration_tests::uvm_1800_2_2017_bare_run_phase_completes,
        uvm_integration_tests::uvm_1800_2_2020_bare_run_phase_completes,
        uvm_integration_tests::uvm_queued_completion_wait_remains_blocked,
        uvm_objection_bridge::raise_then_drop_idiom_blocks_until_drop_not_t0,
        uvm_objection_bridge::wait_idiom_works_with_a_variable_threshold,
    ],
};

const SEQUENCES_TRAFFIC: Group = Group {
    selector: "uvm_groups::uvm_sequences_traffic",
    cases: cases![
        uvm_feature_coverage::uvm_virtual_sequence_drives_two_agents,
        uvm_feature_coverage::uvm_sequence_receives_matching_responses,
        uvm_feature_coverage::uvm_locked_sequence_items_are_contiguous,
        uvm_integration_tests::uvm_2020_complete_bench_runs_traffic,
        uvm_integration_tests::uvm_2020_traffic_is_independent_of_method_policy,
        uvm_integration_tests::uvm_1_2_sequencer_driver_handshake,
        uvm_integration_tests::uvm_1_2_user_sequencer_redeclares_req_fifo,
        uvm_integration_tests::uvm_2020_sequence_field_get_type_resolves_declared_type,
        uvm_integration_tests::stress_bench_items_flow_end_to_end_1_2,
    ],
};

const TLM: Group = Group {
    selector: "uvm_groups::uvm_tlm",
    cases: cases![
        uvm_integration_tests::uvm_2020_analysis_imp_decl_routes_to_suffixed_write,
        uvm_integration_tests::uvm_tlm1_blocking_ports_fifo_analysis,
        uvm_integration_tests::uvm_tlm1_nonblocking_family_analysis_fifo,
        uvm_integration_tests::uvm_tlm2_b_transport,
        uvm_tlm_coverage::uvm_tlm1_transport_and_channels,
        uvm_tlm_coverage::uvm_tlm1_connection_topology,
        uvm_tlm_coverage::uvm_tlm1_connection_errors,
        uvm_tlm_coverage::uvm_tlm2_passthrough_and_nonblocking_sockets,
    ],
};

const RAL: Group = Group {
    selector: "uvm_groups::uvm_ral",
    cases: cases![
        uvm_integration_tests::uvm_1_2_ral_frontdoor_read_write,
        uvm_integration_tests::uvm_1_2_ral_backdoor_peek_poke,
        uvm_ral_coverage::uvm_ral_field_access_policies,
        uvm_ral_coverage::uvm_ral_map_rights,
        uvm_ral_coverage::uvm_ral_backdoor_second_write_keeps_this,
        uvm_ral_coverage::uvm_ral_frontdoor_policies_and_update,
        uvm_ral_coverage::uvm_ral_hw_reset_seq,
        uvm_ral_coverage::uvm_ral_mirror_check_mismatch,
        uvm_ral_coverage::uvm_ral_explicit_predictor,
        uvm_ral_coverage::uvm_ral_mem_access,
    ],
};

const DPI: Group = Group {
    selector: "uvm_groups::uvm_dpi",
    cases: cases![
        uvm_dpi_library::uvm_1_2_dpi_regex_config_factory_cmdline,
        uvm_dpi_library::uvm_1_2_dpi_regex_diagnostics,
        uvm_dpi_library::uvm_1800_2_2017_dpi_empty_glob_and_wildcard_config,
        uvm_dpi_library::uvm_1_2_dpi_hdl_backdoor_paths,
        uvm_integration_tests::uvm_2020_concurrent_assertions_sample_with_dpi,
        uvm_integration_tests::uvm_1800_2_2017_config_db_trace_plusarg_with_dpi,
    ],
};

const PRINTING_REPORTING: Group = Group {
    selector: "uvm_groups::uvm_printing_reporting",
    cases: cases![
        uvm_feature_coverage::uvm_report_verbosity_actions_and_overrides,
        uvm_feature_coverage::uvm_cmdline_testname_and_verbosity,
        uvm_integration_tests::uvm_resource_db_report_catcher,
        uvm_integration_tests::uvm_2020_printer_renders_enum_array_members,
        uvm_printer_fixes::static_string_len_works,
        uvm_printer_fixes::static_string_substr_works,
        uvm_printer_fixes::fwrite_with_string_returning_call,
        uvm_printer_fixes::write_with_bare_string_call_in_class_method,
        uvm_printer_fixes::pkg_qualified_static_call,
        uvm_printer_fixes::pkg_qualified_static_factory_create,
    ],
};

const OBJECTS_FEATURES: Group = Group {
    selector: "uvm_groups::uvm_objects_features",
    cases: cases![
        uvm_feature_coverage::uvm_callbacks_add_and_delete,
        uvm_integration_tests::test_uvm_hello_world,
        uvm_integration_tests::uvm_events_barriers_pools,
        uvm_integration_tests::uvm_field_automation_clone_pack,
    ],
};

const GROUPS: &[Group] = &[
    FOUNDATION_CONFIG_FACTORY,
    PHASING_VERSIONS,
    SEQUENCES_TRAFFIC,
    TLM,
    RAL,
    DPI,
    PRINTING_REPORTING,
    OBJECTS_FEATURES,
];

fn run_group(group: Group) {
    // Policy subprocesses select one original case inside the registered
    // group. The same exact-name filter is useful when debugging a case.
    let filter = std::env::var("XEZIM_UVM_CASE").ok();
    if let Some(name) = &filter {
        assert!(
            GROUPS
                .iter()
                .any(|g| g.cases.iter().any(|case| case.name == name)),
            "unknown XEZIM_UVM_CASE={name:?}; see the case manifest in tests/classes/uvm_groups.rs"
        );
    }
    let selected: Vec<Case> = group
        .cases
        .iter()
        .copied()
        .filter(|case| filter.as_ref().is_none_or(|name| case.name == name))
        .collect();
    let failures = runner::collect_failures(group.selector, &selected);
    let details = failures
        .iter()
        .map(|failure| format!("{}: {}", failure.case, failure.message))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        failures.is_empty(),
        "{} UVM case(s) failed in {}:\n{details}",
        failures.len(),
        group.selector
    );
}

#[test]
fn uvm_foundation_config_factory() {
    run_group(FOUNDATION_CONFIG_FACTORY);
}

#[test]
fn uvm_phasing_versions() {
    run_group(PHASING_VERSIONS);
}

#[test]
fn uvm_sequences_traffic() {
    run_group(SEQUENCES_TRAFFIC);
}

#[test]
fn uvm_tlm() {
    run_group(TLM);
}

#[test]
fn uvm_ral() {
    run_group(RAL);
}

#[test]
fn uvm_dpi() {
    run_group(DPI);
}

#[test]
fn uvm_printing_reporting() {
    run_group(PRINTING_REPORTING);
}

#[test]
fn uvm_objects_features() {
    run_group(OBJECTS_FEATURES);
}
