//! Opt-in DVCon research observer. Sampling happens only at an explicit
//! testbench task call; this is not a scheduler-wide transition stream.
//! IEEE 1800 §4.4/§9.4: the caller owns the sample timing, not this observer.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use std::collections::VecDeque;
use std::io::Write;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Trigger {
    r#type: String,
    event: String,
    correlation_key: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Capture {
    fields: Vec<String>,
    pretrigger_events: usize,
    posttrigger_events: usize,
    max_events: usize,
    max_bytes: usize,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    unknown_field: String,
    on_budget_exhaustion: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rule {
    schema_version: u32,
    name: String,
    trigger: Trigger,
    watch: Vec<String>,
    capture: Capture,
    policy: Policy,
}
impl Rule {
    fn parse(text: &str) -> Result<Self, String> {
        let r: Self = serde_json::from_str(text).map_err(|e| format!("rule schema: {e}"))?;
        let fields = [
            "time", "phase", "object", "old", "new", "writer", "source", "txn_id",
        ];
        if r.schema_version != 1
            || r.name.is_empty()
            || r.name.len() > 64
            || r.watch.is_empty()
            || r.watch.len() > 64
            || r.watch.iter().any(|p| {
                p.len() > 256
                    || p.is_empty()
                    || !p
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._[]".contains(&b))
            })
            || r.watch
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != r.watch.len()
            || !matches!(r.trigger.r#type.as_str(), "testbench_event" | "always")
            || r.trigger.event.is_empty()
            || r.trigger.event.len() > 64
            || r.trigger.correlation_key != "txn_id"
            || r.capture.fields.len() != fields.len()
            || fields
                .iter()
                .any(|f| !r.capture.fields.iter().any(|v| v == f))
            || r.capture.pretrigger_events > 4096
            || r.capture.posttrigger_events > 4096
            || !(1..=8192).contains(&r.capture.max_events)
            || !(512..=16_777_216).contains(&r.capture.max_bytes)
            || r.policy.unknown_field != "null_with_reason"
            || r.policy.on_budget_exhaustion != "emit_truncation_record"
        {
            return Err("unsupported or out-of-bounds rule contract".into());
        }
        Ok(r)
    }
}
#[derive(Default, Serialize)]
struct Stats {
    candidates_seen: u64,
    emitted_events: u64,
    filtered_events: u64,
    ring_overflows: u64,
    dropped_events: u64,
    truncation_count: u64,
    output_bytes: u64,
    schema_failures: u64,
    triggered: bool,
    completed: bool,
    posttrigger_remaining: usize,
    quality: Vec<String>,
}
pub(super) struct Observer {
    rule: Rule,
    out: std::fs::File,
    stats_path: std::path::PathBuf,
    ring: VecDeque<Json>,
    previous: std::collections::BTreeMap<String, String>,
    stats: Stats,
    seq: u64,
    run_id: String,
    closed: bool,
    ring_bytes: usize,
    experimental: bool,
}
impl Observer {
    fn new(rule: Rule, manifest: Json) -> Result<Self, String> {
        fn path(key: &str) -> Result<std::path::PathBuf, String> {
            std::env::var_os(key)
                .map(Into::into)
                .ok_or_else(|| format!("{key} is required"))
        }
        let out_path = path("DVCON_OBS_OUT")?;
        let manifest_path = path("DVCON_OBS_MANIFEST")?;
        let stats_path = path("DVCON_OBS_STATS")?;
        if out_path == manifest_path || out_path == stats_path || manifest_path == stats_path {
            return Err("output paths must be distinct".into());
        }
        // create_new protects existing raw evidence and source from overwrite.
        let out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(out_path)
            .map_err(|e| e.to_string())?;
        let mut mf = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(manifest_path)
            .map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(&mut mf, &manifest).map_err(|e| e.to_string())?;
        if stats_path.exists() {
            return Err("stats output already exists".into());
        }
        let triggered = rule.trigger.r#type == "always";
        let run_id = std::env::var("DVCON_RUN_ID").unwrap_or_else(|_| "unspecified".into());
        if run_id.is_empty() || run_id.len() > 128
            || !run_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        {
            return Err("run id must use 1..128 ASCII identifier characters".into());
        }
        Ok(Self {
            rule,
            out,
            stats_path,
            ring: VecDeque::new(),
            previous: Default::default(),
            stats: Stats {
                triggered,
                ..Default::default()
            },
            seq: 0,
            run_id,
            closed: false,
            ring_bytes: 0,
            experimental: std::env::var("DVCON_EXPERIMENTAL").as_deref() == Ok("1"),
        })
    }
    fn emit(&mut self, record: Json) -> Result<(), String> {
        let line = serde_json::to_vec(&record).map_err(|e| e.to_string())?;
        // Reserve one event and 384 bytes for the one truncation record.
        if self.stats.emitted_events as usize >= self.rule.capture.max_events.saturating_sub(1)
            || self.stats.output_bytes as usize + line.len() + 1
                > self.rule.capture.max_bytes.saturating_sub(384)
        {
            self.stats.dropped_events += 1;
            if self.stats.truncation_count == 0 {
                self.stats.truncation_count = 1;
                let trunc = json!({"schema_version":1,"kind":"truncation","seq":record["seq"],
                    "run_id":self.run_id,"time_ticks":record["time_ticks"],"quality":["output_budget_exhausted"]});
                let b = serde_json::to_vec(&trunc).map_err(|e| e.to_string())?;
                self.out
                    .write_all(&b)
                    .and_then(|_| self.out.write_all(b"\n"))
                    .map_err(|e| e.to_string())?;
                self.stats.output_bytes += (b.len() + 1) as u64;
                self.stats.emitted_events += 1;
            }
            return Ok(());
        }
        self.out
            .write_all(&line)
            .and_then(|_| self.out.write_all(b"\n"))
            .map_err(|e| e.to_string())?;
        self.stats.emitted_events += 1;
        self.stats.output_bytes += (line.len() + 1) as u64;
        Ok(())
    }
    fn candidate(&mut self, record: Json, fire: bool) -> Result<(), String> {
        self.stats.candidates_seen += 1;
        if self.closed {
            self.stats.filtered_events += 1;
            return Ok(());
        }
        if !self.stats.triggered {
            if fire {
                self.stats.triggered = true;
                self.stats.posttrigger_remaining = self.rule.capture.posttrigger_events;
                while let Some(r) = self.ring.pop_front() {
                    self.emit(r)?;
                }
                self.ring_bytes = 0;
                self.emit(record)?;
                if self.stats.posttrigger_remaining == 0 {
                    self.closed = true;
                }
            } else if self.rule.capture.pretrigger_events > 0 {
                let size = serde_json::to_vec(&record)
                    .map_err(|e| e.to_string())?
                    .len()
                    + 1;
                while !self.ring.is_empty()
                    && (self.ring.len() == self.rule.capture.pretrigger_events
                        || self.ring_bytes + size > self.rule.capture.max_bytes)
                {
                    let old = self.ring.pop_front().unwrap();
                    self.ring_bytes -=
                        serde_json::to_vec(&old).map_err(|e| e.to_string())?.len() + 1;
                    self.stats.ring_overflows += 1;
                }
                if size <= self.rule.capture.max_bytes {
                    self.ring_bytes += size;
                    self.ring.push_back(record);
                } else {
                    self.stats.dropped_events += 1;
                    if !self
                        .stats
                        .quality
                        .iter()
                        .any(|s| s == "pretrigger_event_exceeds_byte_budget")
                    {
                        self.stats
                            .quality
                            .push("pretrigger_event_exceeds_byte_budget".into());
                    }
                }
            } else {
                self.stats.filtered_events += 1;
            }
        } else if self.rule.trigger.r#type == "always" {
            self.emit(record)?;
        } else if self.stats.posttrigger_remaining > 0 {
            self.emit(record)?;
            self.stats.posttrigger_remaining -= 1;
            if self.stats.posttrigger_remaining == 0 {
                self.closed = true;
            }
        } else {
            self.stats.filtered_events += 1;
        }
        Ok(())
    }
    fn record(
        &mut self,
        time: u64,
        tick_s: f64,
        event: &str,
        txn: Option<&str>,
        values: Vec<(String, String)>,
    ) -> Result<(), String> {
        let mut quality = vec![
            "writer_not_available",
            "source_not_available",
            "scheduler_region_not_exported",
        ];
        if txn.is_none() {
            quality.push("correlation_unknown");
        }
        let kind = if event == "sample" {
            "signal_sample"
        } else {
            "testbench_event"
        };
        let rows: Vec<(Option<String>, Option<String>)> = if event == "sample" {
            values
                .into_iter()
                .map(|(p, v)| (Some(p), Some(v)))
                .collect()
        } else {
            vec![(None, None)]
        };
        for (object, new) in rows {
            let old = object.as_ref().and_then(|p| {
                new.as_ref()
                    .and_then(|v| self.previous.insert(p.clone(), v.clone()))
            });
            let mut event_quality = quality.clone();
            if object.is_some() && old.is_none() {
                event_quality.push("old_sample_not_available");
            }
            let r = json!({"schema_version":1,"run_id":self.run_id,"seq":self.seq,"time_ticks":time,
                "tick_seconds":tick_s,"phase":"testbench_call","kind":kind,"event":event,
                "object":object,"old":old,"new":new,"writer":null,"source":null,"txn_id":txn,
                "correlation_basis":txn.map(|_|"explicit_testbench_tag"),"experimental":self.experimental,"quality":event_quality});
            self.seq += 1;
            self.candidate(r, event == self.rule.trigger.event)?;
        }
        Ok(())
    }
    fn finish(&mut self) -> Result<(), String> {
        self.stats.completed = true;
        if !self.stats.triggered {
            self.stats.quality.push("trigger_not_seen".into());
        }
        if self.stats.ring_overflows > 0 {
            self.stats.quality.push("pretrigger_history_evicted".into());
        }
        if self.stats.posttrigger_remaining > 0 {
            self.stats
                .quality
                .push("posttrigger_incomplete_at_finish".into());
        }
        self.stats.filtered_events += self.ring.len() as u64;
        self.ring.clear();
        self.ring_bytes = 0;
        self.out.flush().map_err(|e| e.to_string())?;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.stats_path)
            .map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(&mut f, &self.stats).map_err(|e| e.to_string())
    }
}
impl Simulator {
    pub(super) fn dvcon_observe(&mut self, args: &[Expression]) {
        if let Err(e) = self.dvcon_observe_inner(args) {
            let m = format!("[dvcon-observation] {e}");
            self.stdout_writeln(&m);
            self.record_output(m);
            self.abort_run();
        }
    }
    fn dvcon_observe_inner(&mut self, args: &[Expression]) -> Result<(), String> {
        // No source hooks => no environment checks and no observation work.
        // Hooks with the extension disabled are inert.
        let Some(path) = std::env::var_os("DVCON_OBS_RULE") else {
            return Ok(());
        };
        if args.len() != 2 {
            return Err("$dvcon_observe requires (event, txn_id)".into());
        }
        // This narrow research ABI admits literals only: evaluating a function
        // actual here could write HDL state from an observation-only request.
        let (ExprKind::StringLiteral(event), ExprKind::StringLiteral(txn)) =
            (&args[0].kind, &args[1].kind)
        else {
            return Err("$dvcon_observe requires literal event and transaction strings".into());
        };
        let event = event.clone();
        let txn = txn.clone();
        if event.len() > 64 || txn.len() > 128 || txn.chars().any(char::is_control) {
            return Err("invalid event/tag".into());
        }
        if self.dvcon_observation.is_none() {
            let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
            if meta.len() > 65536 {
                return Err("rule exceeds 64 KiB".into());
            }
            let rule = Rule::parse(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)?;
            for p in &rule.watch {
                let n = p
                    .strip_prefix(&format!("{}.", self.module.name))
                    .unwrap_or(p);
                if self
                    .get_signal(n)
                    .or_else(|| self.get_signal(p))
                    .is_none_or(|v| v.width > 4096)
                {
                    return Err(format!("unresolved approved object: {p}"));
                }
            }
            let manifest = json!({"schema_version":1,"implementation":"new_native_prototype",
                "watch":rule.watch,"phase":"testbench_call","tick_seconds":self.tick_s,
                "capabilities":["read_approved_signals","explicit_testbench_events","bounded_event_history"],
                "unsupported":["arbitrary_predicates","native_control","automatic_transaction_inference","scheduler_wide_changes"],
                "rule_name":rule.name});
            self.dvcon_observation = Some(Box::new(Observer::new(rule, manifest)?));
        }
        let mut obs = self.dvcon_observation.take().unwrap();
        let result = if event == "finish" {
            obs.finish()
        } else {
            let values = if event == "sample" {
                obs.rule
                    .watch
                    .iter()
                    .map(|p| {
                        let n = p
                            .strip_prefix(&format!("{}.", self.module.name))
                            .unwrap_or(p);
                        let v = self
                            .get_signal(n)
                            .or_else(|| self.get_signal(p))
                            .unwrap()
                            .to_bin();
                        (p.clone(), v)
                    })
                    .collect()
            } else {
                Vec::new()
            };
            obs.record(
                self.time,
                self.tick_s,
                &event,
                if txn.is_empty() {
                    None
                } else {
                    Some(txn.as_str())
                },
                values,
            )
        };
        self.dvcon_observation = Some(obs);
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn base() -> Json {
        json!({"schema_version":1,"name":"test","trigger":{"type":"testbench_event","event":"timeout","correlation_key":"txn_id"},
        "watch":["t.dut.a"],"capture":{"fields":["time","phase","object","old","new","writer","source","txn_id"],
        "pretrigger_events":2,"posttrigger_events":2,"max_events":10,"max_bytes":4096},
        "policy":{"unknown_field":"null_with_reason","on_budget_exhaustion":"emit_truncation_record"}})
    }
    #[test]
    fn rule_contract_validation() {
        assert!(Rule::parse(&base().to_string()).is_ok());
        for (pointer, value) in [
            ("/schema_version", json!(2)),
            ("/watch", json!([])),
            ("/watch", json!(["t.a", "t.a"])),
            ("/watch", json!(["../private"])),
            ("/capture/pretrigger_events", json!(4097)),
            ("/capture/posttrigger_events", json!(-1)),
            ("/capture/max_events", json!(0)),
            ("/capture/max_bytes", json!(32)),
            ("/capture/fields", json!(["time"])),
            ("/trigger/type", json!("value_guess")),
            ("/trigger/correlation_key", json!("guess")),
            ("/policy/unknown_field", json!("guess")),
            ("/policy/on_budget_exhaustion", json!("silent")),
        ] {
            let mut b = base();
            *b.pointer_mut(pointer).unwrap() = value;
            assert!(Rule::parse(&b.to_string()).is_err(), "{pointer}");
        }
        let mut b = base();
        b["execute"] = json!("anything");
        assert!(Rule::parse(&b.to_string()).is_err());
    }
}
