//! Diagnostic activity sketches. These observations never authorize execution.

use super::*;
#[allow(unused_imports)]
use crate::log_eprintln as eprintln;
use serde::Serialize;

const SKETCH_SIZE: usize = 16;
const DEFAULT_STRIDE: u64 = 64;
const HASH_SEED: u64 = 0xd1b54a32d192ed03;

fn mix64(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

/// A deterministic content fingerprint, not a cryptographic integrity check.
/// Streaming avoids allocating another serialized copy of a large design.
struct FingerprintWriter(u64, u64);

impl Default for FingerprintWriter {
    fn default() -> Self {
        Self(0xcbf29ce484222325, 0x84222325cbf29ce4)
    }
}

impl std::io::Write for FingerprintWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x100000001b3);
            self.1 = (self.1 ^ (u64::from(byte) + 0x9d)).wrapping_mul(0x9e3779b185ebca87);
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn fingerprint<T: Serialize>(value: &T) -> Result<String, String> {
    let mut writer = FingerprintWriter::default();
    bincode::serde::encode_into_std_write(value, &mut writer, xezim_core::xez_bincode_config())
        .map_err(|e| e.to_string())?;
    Ok(format!("{:016x}{:016x}", writer.0, writer.1))
}

#[derive(Serialize)]
struct EntryActivity {
    id: usize,
    read_signals: usize,
    write_signals: usize,
    logical_read_bytes: u64,
    logical_write_bytes: u64,
    static_instructions: Option<usize>,
    unresolved_reads: bool,
    evaluations: u64,
    active_epochs: u64,
    sampled_active_epochs: u64,
    // None means no sampled activation, not an empty set matching other
    // unobserved entries. One shared hash vector is used by every entry.
    minhash: Option<[u64; SKETCH_SIZE]>,
    #[serde(skip)]
    last_epoch: u64,
}

#[derive(Serialize)]
struct ActivityReport {
    schema: u32,
    build_revision: &'static str,
    crate_version: &'static str,
    jit_compiled: bool,
    tick_seconds: f64,
    design_fingerprint: String,
    layout_fingerprint: String,
    execution_fingerprint: String,
    identity_scope: &'static str,
    // Captured for provenance, not treated as a proof of safe admission.
    runtime_options: std::collections::BTreeMap<String, String>,
    epoch_definition: &'static str,
    layout_stage: &'static str,
    scope: &'static str,
    excluded_evaluation_paths: [&'static str; 2],
    gate_lane_disabled: bool,
    sampling_policy: &'static str,
    hash_algorithm: &'static str,
    hash_seed: u64,
    sketch_size: usize,
    sampling_stride: u64,
    epochs: u64,
    deferred_epochs: u64,
    sampled_epochs: u64,
    first_time: Option<u64>,
    last_time: Option<u64>,
    entries: Vec<EntryActivity>,
}

pub(super) struct ActivityProfile {
    path: PathBuf,
    report: ActivityReport,
    selected: bool,
    hashes: [u64; SKETCH_SIZE],
}

impl ActivityProfile {
    pub(super) fn requested(module: &ElaboratedModule) -> Option<Self> {
        let path = std::env::var_os("XEZIM_ACTIVITY_PROFILE")?;
        if path.is_empty() {
            return None;
        }
        // The BSP path has different evaluation boundaries.
        // Refuse an unsupported backend rather than export partial counts.
        if std::env::var_os("XEZIM_BSP_SETTLE").is_some()
            || std::env::var_os("XEZIM_BSP_SHADOW").is_some()
        {
            eprintln!("[ACTIVITY] profile disabled: canonical settle backend required");
            return None;
        }
        let stride = match std::env::var("XEZIM_ACTIVITY_STRIDE") {
            Ok(s) => match s.parse::<u64>() {
                Ok(n) if n > 0 => n,
                _ => {
                    eprintln!("[ACTIVITY] profile disabled: stride must be a positive integer");
                    return None;
                }
            },
            Err(_) => DEFAULT_STRIDE,
        };
        let identity = match fingerprint(module) {
            Ok(identity) => identity,
            Err(e) => {
                eprintln!("[ACTIVITY] profile disabled: design fingerprint failed: {e}");
                return None;
            }
        };
        let options = crate::env_vars::ENV_VARS
            .iter()
            .filter(|(k, _)| *k != "XEZIM_ACTIVITY_PROFILE")
            .filter_map(|(k, _)| {
                std::env::var_os(k).map(|v| ((*k).to_string(), v.to_string_lossy().into_owned()))
            })
            .collect();
        let mut profile = Self::new(path.into(), identity, stride, options, module.tick_s);
        let executable = if cfg!(target_os = "linux") {
            PathBuf::from("/proc/self/exe")
        } else {
            match std::env::current_exe() {
                Ok(path) => path,
                Err(e) => {
                    eprintln!("[ACTIVITY] profile disabled: executable identity failed: {e}");
                    return None;
                }
            }
        };
        let result = (|| -> Result<String, std::io::Error> {
            let mut file = std::fs::File::open(executable)?;
            let mut hash = FingerprintWriter::default();
            std::io::copy(&mut file, &mut hash)?;
            Ok(format!("{:016x}{:016x}", hash.0, hash.1))
        })();
        match result {
            Ok(identity) => profile.report.execution_fingerprint = identity,
            Err(e) => {
                eprintln!("[ACTIVITY] profile disabled: executable identity failed: {e}");
                return None;
            }
        }
        Some(profile)
    }

    fn new(
        path: PathBuf,
        design_fingerprint: String,
        stride: u64,
        runtime_options: std::collections::BTreeMap<String, String>,
        tick_seconds: f64,
    ) -> Self {
        Self {
            path,
            report: ActivityReport {
                schema: 1,
                build_revision: env!("XEZIM_GIT_HASH"),
                crate_version: env!("CARGO_PKG_VERSION"),
                jit_compiled: cfg!(feature = "jit"),
                tick_seconds,
                design_fingerprint,
                layout_fingerprint: String::new(),
                execution_fingerprint: String::new(),
                identity_scope: "elaborated input, final ordered layout, executable file; noncryptographic; external firmware and libraries require separate receipts",
                runtime_options,
                epoch_definition: "canonical_settle_invocation; repeated passes share an epoch; equal timestamps do not",
                layout_stage: "final_ordered_comb_entries",
                scope: "canonical combinational worklist only; zero observations do not imply inactivity",
                excluded_evaluation_paths: ["eager_clock_tree", "direct_port_negedge"],
                gate_lane_disabled: true,
                sampling_policy: "mix64(epoch xor seed) modulo stride equals zero",
                hash_algorithm: "splitmix64-finalizer-v1",
                hash_seed: HASH_SEED,
                sketch_size: SKETCH_SIZE,
                sampling_stride: stride,
                epochs: 0,
                deferred_epochs: 0,
                sampled_epochs: 0,
                first_time: None,
                last_time: None,
                entries: Vec::new(),
            },
            selected: false,
            hashes: [0; SKETCH_SIZE],
        }
    }

    fn prepare(
        &mut self,
        entries: &[CombEntry],
        widths: &[u32],
        signed: &[bool],
        real: &[bool],
    ) -> Result<(), String> {
        // Exact ordered item/bytecode/dependency identity, including operands,
        // constants, widths and signedness. Equal entry counts are insufficient.
        self.report.layout_fingerprint = fingerprint(&(entries, widths, signed, real))?;
        let bytes = |ids: &[usize]| -> u64 {
            ids.iter()
                .map(|&id| u64::from(widths.get(id).copied().unwrap_or(0)).div_ceil(8))
                .sum()
        };
        self.report.entries = entries
            .iter()
            .enumerate()
            .map(|(id, e)| EntryActivity {
                id,
                read_signals: e.cold.read_signal_ids.len(),
                write_signals: e.cold.write_signal_ids.len(),
                logical_read_bytes: bytes(&e.cold.read_signal_ids),
                logical_write_bytes: bytes(&e.cold.write_signal_ids),
                static_instructions: match &e.item {
                    CombItem::CompiledContAssign { compiled }
                    | CombItem::CompiledAlwaysBlock { compiled, .. } => {
                        Some(compiled.instructions.len())
                    }
                    _ => None,
                },
                unresolved_reads: e.has_unresolved_reads,
                evaluations: 0,
                active_epochs: 0,
                sampled_active_epochs: 0,
                minhash: None,
                last_epoch: 0,
            })
            .collect();
        self.report.epochs = 0;
        self.report.deferred_epochs = 0;
        self.report.sampled_epochs = 0;
        self.report.first_time = None;
        self.report.last_time = None;
        self.selected = false;
        Ok(())
    }

    pub(super) fn begin_epoch(&mut self, time: u64, deferred: bool) {
        self.report.epochs += 1;
        self.report.deferred_epochs += u64::from(deferred);
        self.report.first_time.get_or_insert(time);
        self.report.last_time = Some(time);
        // Hash the epoch before sampling: every-Nth invocation can alias a
        // clock or a repeating scheduling pattern and miss it permanently.
        self.selected =
            mix64(self.report.epochs ^ HASH_SEED).is_multiple_of(self.report.sampling_stride);
        if self.selected {
            self.report.sampled_epochs += 1;
            for (i, h) in self.hashes.iter_mut().enumerate() {
                *h = mix64(
                    (self.report.epochs ^ HASH_SEED)
                        .wrapping_add((i as u64).wrapping_mul(0x9e3779b97f4a7c15)),
                );
            }
        }
    }

    #[inline]
    pub(super) fn record(&mut self, id: usize) {
        let entry = &mut self.report.entries[id];
        entry.evaluations += 1;
        if entry.last_epoch != self.report.epochs {
            entry.last_epoch = self.report.epochs;
            entry.active_epochs += 1;
            if self.selected {
                entry.sampled_active_epochs += 1;
                let sketch = entry.minhash.get_or_insert([u64::MAX; SKETCH_SIZE]);
                for (value, hash) in sketch.iter_mut().zip(self.hashes) {
                    *value = (*value).min(hash);
                }
            }
        }
    }

    fn write(&self, time: u64, max_time: u64, plusargs: &[String]) -> Result<(), String> {
        #[derive(Serialize)]
        struct Snapshot<'a> {
            #[serde(flatten)]
            report: &'a ActivityReport,
            end_time_ticks: u64,
            max_time_ticks: u64,
            plusargs: &'a [String],
        }
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let name = self
            .path
            .file_name()
            .ok_or("profile path has no filename")?
            .to_string_lossy();
        let temporary = self.path.with_file_name(format!(
            ".{name}.{}.{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            let mut out = std::io::BufWriter::new(file);
            serde_json::to_writer(
                &mut out,
                &Snapshot {
                    report: &self.report,
                    end_time_ticks: time,
                    max_time_ticks: max_time,
                    plusargs,
                },
            )
            .map_err(|e| e.to_string())?;
            out.flush().map_err(|e| e.to_string())?;
            drop(out);
            std::fs::rename(&temporary, &self.path).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }
}

impl Simulator {
    pub(super) fn prepare_activity_profile(&mut self) {
        if let Some(profile) = self.activity_profile.as_mut() {
            if let Err(e) = profile.prepare(
                &self.comb_entries,
                &self.signal_widths,
                &self.signal_signed,
                &self.signal_real,
            ) {
                eprintln!("[ACTIVITY] profile disabled: layout fingerprint failed: {e}");
                self.activity_profile = None;
            }
        }
    }

    pub(super) fn dump_activity_profile(&self) {
        if let Some(profile) = self.activity_profile.as_ref() {
            match profile.write(self.time, self.max_time, &self.plusargs) {
                Ok(()) => eprintln!("[ACTIVITY] profile written: {}", profile.path.display()),
                Err(e) => eprintln!("[ACTIVITY] profile write failed: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(stride: u64) -> ActivityProfile {
        let mut p = ActivityProfile::new(
            PathBuf::new(),
            "design".into(),
            stride,
            Default::default(),
            1e-9,
        );
        let entries: Vec<_> = (0..3)
            .map(|_| CombEntry {
                item: CombItem::Noop,
                cold: Box::new(CombEntryCold {
                    scope_hint: None,
                    read_signal_ids: vec![0],
                    write_signal_ids: vec![1],
                    span: crate::ast::Span::dummy(),
                    cov: None,
                }),
                has_unresolved_reads: false,
                defer_at_time0: false,
            })
            .collect();
        p.prepare(&entries, &[8, 65], &[false, false], &[false, false])
            .unwrap();
        p
    }

    #[test]
    fn equal_time_invocations_are_distinct_but_repasses_are_not() {
        let mut p = profile(1);
        p.begin_epoch(10, false);
        p.record(0);
        p.record(0);
        p.record(1);
        assert_eq!(p.report.entries[0].evaluations, 2);
        assert_eq!(p.report.entries[0].active_epochs, 1);
        assert_eq!(p.report.entries[0].minhash, p.report.entries[1].minhash);
        p.begin_epoch(10, true);
        p.record(0);
        assert_eq!(p.report.entries[0].active_epochs, 2);
        assert_eq!(p.report.entries[1].active_epochs, 1);
        assert_eq!(p.report.deferred_epochs, 1);
        assert_eq!(p.report.entries[2].minhash, None);
    }

    #[test]
    fn sparse_sampling_keeps_exact_frequencies_and_shared_hashes() {
        let mut p = profile(4);
        for i in 0..12 {
            p.begin_epoch(i, false);
            p.record(0);
            p.record(1);
        }
        assert!(p.report.sampled_epochs > 0 && p.report.sampled_epochs < 12);
        assert_eq!(p.report.entries[0].active_epochs, 12);
        assert_eq!(
            p.report.entries[0].sampled_active_epochs,
            p.report.sampled_epochs
        );
        assert_eq!(p.report.entries[0].minhash, p.report.entries[1].minhash);
        assert_eq!(p.report.entries[0].logical_read_bytes, 1);
        assert_eq!(p.report.entries[0].logical_write_bytes, 9);
    }

    #[test]
    fn layout_identity_covers_more_than_entry_count() {
        let a = fingerprint(&(vec![1u32, 2], vec![8u32, 65], vec![false, false])).unwrap();
        let b = fingerprint(&(vec![2u32, 1], vec![8u32, 65], vec![false, false])).unwrap();
        let c = fingerprint(&(vec![1u32, 2], vec![8u32, 64], vec![false, false])).unwrap();
        let d = fingerprint(&(vec![1u32, 2], vec![8u32, 65], vec![true, false])).unwrap();
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
    }

    #[test]
    fn actual_item_operands_and_dependencies_change_layout_identity() {
        let mut p = profile(1);
        let mut entries = vec![CombEntry {
            item: CombItem::DirectCopy {
                dst_id: 1,
                src_id: 0,
                width: 8,
            },
            cold: Box::new(CombEntryCold {
                scope_hint: None,
                read_signal_ids: vec![0],
                write_signal_ids: vec![1],
                span: crate::ast::Span::dummy(),
                cov: None,
            }),
            has_unresolved_reads: false,
            defer_at_time0: false,
        }];
        p.prepare(&entries, &[8, 8], &[false, false], &[false, false])
            .unwrap();
        let first = p.report.layout_fingerprint.clone();
        entries[0].item = CombItem::DirectCopy {
            dst_id: 0,
            src_id: 1,
            width: 8,
        };
        p.prepare(&entries, &[8, 8], &[false, false], &[false, false])
            .unwrap();
        assert_ne!(first, p.report.layout_fingerprint);
        let second = p.report.layout_fingerprint.clone();
        entries[0].cold.read_signal_ids = vec![1];
        p.prepare(&entries, &[8, 8], &[false, false], &[false, false])
            .unwrap();
        assert_ne!(second, p.report.layout_fingerprint);
        p.begin_epoch(0, false);
        p.record(0);
        p.prepare(&entries, &[8, 8], &[false, false], &[false, false])
            .unwrap();
        assert_eq!(p.report.epochs, 0);
        assert_eq!(p.report.entries[0].evaluations, 0);
    }

    #[test]
    fn periodic_activity_is_not_lost_to_stride_aliasing() {
        let mut p = profile(64);
        for i in 0..65536 {
            p.begin_epoch(i, false);
            if i % 64 == 1 {
                p.record(0);
            }
        }
        assert_eq!(p.report.entries[0].active_epochs, 1024);
        assert!(p.report.entries[0].sampled_active_epochs > 0);
        assert!(p.report.entries[0].sampled_active_epochs < 1024);
        assert!(p.report.entries[0].minhash.is_some());
    }

    #[test]
    fn fingerprint_is_independent_of_write_chunking() {
        let mut a = FingerprintWriter::default();
        let mut b = FingerprintWriter::default();
        a.write_all(b"abcd").unwrap();
        b.write_all(b"ab").unwrap();
        b.write_all(b"cd").unwrap();
        assert_eq!((a.0, a.1), (b.0, b.1));
    }

    #[test]
    fn output_replaces_complete_json_and_failed_rename_cleans_temporary() {
        let directory =
            std::env::temp_dir().join(format!("xezim_activity_writer_{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut p = profile(1);
        p.path = directory.join("profile.json");
        std::fs::write(&p.path, b"old output").unwrap();
        p.begin_epoch(3, false);
        p.record(0);
        p.write(3, 100, &[]).unwrap();
        let output: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&p.path).unwrap()).unwrap();
        assert_eq!(output["entries"][0]["evaluations"], 1);
        assert_eq!(output["end_time_ticks"], 3);
        let blocked = directory.join("directory");
        std::fs::create_dir_all(&blocked).unwrap();
        p.path = blocked.clone();
        assert!(p.write(3, 100, &[]).is_err());
        assert!(blocked.is_dir());
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 2);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
