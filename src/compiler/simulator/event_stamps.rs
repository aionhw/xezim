//! Dense last-trigger stamps for named events (§15.5.3).
//!
//! Interning records a literal spelling, without resolving aliases or marking
//! the event as fired. Dynamic class, automatic and container event keys can
//! therefore join the table lazily while the parent keeps their existing
//! resolution and scheduling rules.

use std::rc::Rc;
use std::sync::Arc;
use xezim_core::hasher::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(super) struct EventId(pub(super) usize);

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct NamedEventTarget {
    pub(super) name: Arc<str>,
    pub(super) signal_id: usize,
    pub(super) stamp: EventId,
}

/// Ordered candidate spellings, with their exact signal and stamp identities.
/// Multiple spellings may address the same signal; retaining every entry keeps
/// the parent's toggle and covergroup-sampling order unchanged.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct NamedEventPlan {
    pub(super) targets: Box<[NamedEventTarget]>,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(super) struct EventStampStore {
    names: HashMap<Arc<str>, EventId>,
    names_by_id: Vec<Arc<str>>,
    times: Vec<Option<u64>>,
    plans: Vec<Option<Rc<NamedEventPlan>>>,
}

impl EventStampStore {
    pub(super) fn intern(&mut self, name: &str) -> EventId {
        if let Some(&id) = self.names.get(name) {
            return id;
        }
        let id = EventId(self.names_by_id.len());
        let name: Arc<str> = Arc::from(name);
        self.names.insert(Arc::clone(&name), id);
        self.names_by_id.push(name);
        self.times.push(None);
        self.plans.push(None);
        id
    }

    pub(super) fn get(&self, name: &str) -> Option<&u64> {
        let id = self.names.get(name)?;
        self.times[id.0].as_ref()
    }

    /// Match the old stamp map: an interned but unfired name is absent.
    pub(super) fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Compatibility with name-based callers, including dynamic event stamps.
    pub(super) fn insert(&mut self, name: String, time: u64) -> Option<u64> {
        let id = self.intern(&name);
        super::note_event_trigger();
        self.times[id.0].replace(time)
    }

    pub(super) fn mark(&mut self, id: EventId, time: u64) {
        super::note_event_trigger();
        self.times[id.0] = Some(time);
    }

    pub(super) fn plan(&self, id: EventId) -> Option<Rc<NamedEventPlan>> {
        self.plans[id.0].as_ref().map(Rc::clone)
    }

    pub(super) fn set_plan(&mut self, id: EventId, plan: NamedEventPlan) -> Rc<NamedEventPlan> {
        let plan = Rc::new(plan);
        self.plans[id.0] = Some(Rc::clone(&plan));
        plan
    }

    pub(super) fn name(&self, id: EventId) -> &Arc<str> {
        &self.names_by_id[id.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_does_not_fire_an_event_at_time_zero_or_max() {
        let mut stamps = EventStampStore::default();
        assert_eq!(stamps.get("ev"), None);
        let id = stamps.intern("ev");
        assert_eq!(stamps.get("ev"), None);
        assert!(!stamps.contains_key("ev"));
        assert_eq!(stamps.intern("ev"), id);
        assert_eq!(&**stamps.name(id), "ev");
        stamps.mark(id, 0);
        assert_eq!(stamps.get("ev"), Some(&0));
        assert!(stamps.contains_key("ev"));
        assert_eq!(stamps.insert("ev".into(), u64::MAX), Some(0));
        assert_eq!(stamps.get("ev"), Some(&u64::MAX));
        assert_eq!(stamps.insert("ev".into(), 0), Some(u64::MAX));
    }

    #[test]
    fn name_based_operations_match_the_previous_stamp_map() {
        let mut stamps = EventStampStore::default();
        let mut previous: HashMap<String, u64> = HashMap::default();
        let names = ["ev", "tb.ev", ".ev", "events[3]", "\x02inst7#done", ""];
        // Pre-registering names for cached plans must not change recognition
        // of a container's string as an event that has already fired.
        for name in names {
            stamps.intern(name);
        }
        for (name, time) in [
            ("ev", 0),
            ("tb.ev", 0),
            ("events[3]", 12),
            ("ev", 12),
            ("\x02inst7#done", u64::MAX),
            (".ev", 0),
            ("", 1),
            ("tb.ev", u64::MAX),
        ] {
            assert_eq!(
                stamps.insert(name.to_string(), time),
                previous.insert(name.to_string(), time)
            );
            for key in names {
                assert_eq!(stamps.get(key), previous.get(key));
                assert_eq!(stamps.contains_key(key), previous.contains_key(key));
            }
        }
    }

    #[test]
    fn literal_names_reuse_ids_without_consolidating_signal_targets() {
        let mut stamps = EventStampStore::default();
        let bare = stamps.intern("ev");
        let qualified = stamps.intern("tb.ev");
        assert_ne!(bare, qualified);
        let plan = stamps.set_plan(
            bare,
            NamedEventPlan {
                targets: vec![
                    NamedEventTarget {
                        name: Arc::clone(stamps.name(bare)),
                        signal_id: 7,
                        stamp: bare,
                    },
                    NamedEventTarget {
                        name: Arc::clone(stamps.name(qualified)),
                        signal_id: 7,
                        stamp: qualified,
                    },
                ]
                .into_boxed_slice(),
            },
        );
        assert_eq!(plan.targets.len(), 2);
        assert_eq!(&*plan.targets[0].name, "ev");
        assert_eq!(&*plan.targets[1].name, "tb.ev");
        assert_eq!(plan.targets[0].signal_id, plan.targets[1].signal_id);
        assert_ne!(plan.targets[0].stamp, plan.targets[1].stamp);
        // Aliases are resolved outside this store. Returning to a previously
        // resolved target must recover its original ID and plan.
        assert_eq!(stamps.intern("ev"), bare);
        assert!(Rc::ptr_eq(&stamps.plan(bare).unwrap(), &plan));
    }

    #[test]
    fn owned_plans_survive_stamp_updates_replacement_and_dynamic_growth() {
        let mut stamps = EventStampStore::default();
        let id = stamps.intern("ev");
        let plan = stamps.set_plan(
            id,
            NamedEventPlan {
                targets: vec![NamedEventTarget {
                    name: Arc::clone(stamps.name(id)),
                    signal_id: 3,
                    stamp: id,
                }]
                .into_boxed_slice(),
            },
        );
        let active = stamps.plan(id).unwrap();
        stamps.mark(id, 0);
        for index in 0..257 {
            let name = format!("\x02inst{index}#ready");
            let dynamic = stamps.intern(&name);
            stamps.mark(dynamic, index);
            assert_eq!(stamps.get(&name), Some(&index));
        }
        assert_eq!(stamps.get("ev"), Some(&0));
        assert!(Rc::ptr_eq(&plan, &stamps.plan(id).unwrap()));
        let replacement = stamps.set_plan(
            id,
            NamedEventPlan {
                targets: Box::new([]),
            },
        );
        assert!(Rc::ptr_eq(&replacement, &stamps.plan(id).unwrap()));
        assert_eq!(active.targets.len(), 1);
        assert_eq!(active.targets[0].stamp, id);
        assert_eq!(&*active.targets[0].name, "ev");
        // The active plan's IDs still address the original stamp after growth.
        stamps.mark(active.targets[0].stamp, u64::MAX);
        assert_eq!(stamps.get("ev"), Some(&u64::MAX));
        assert_eq!(stamps.get("\x02inst256#ready"), Some(&256));
    }
}
