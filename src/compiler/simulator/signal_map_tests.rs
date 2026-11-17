// The name-keyed store keeps the elements of a queue or dynamic array in a
// dense run (§7.5, §7.10); every accessor has to answer exactly as a plain
// map of the same keys would.
use super::{SignalMap, Value};
use std::collections::BTreeMap;

/// The plain map a `SignalMap` must be indistinguishable from.
type Model = BTreeMap<String, Value>;

fn base_of(k: &str) -> Option<&str> {
    k.find('[').map(|i| &k[..i])
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

/// Every read accessor of `m` against the model.
fn check(m: &SignalMap, model: &Model, names: &[&str], step: usize) {
    assert_eq!(m.len(), model.len(), "len at step {step}");
    assert_eq!(m.is_empty(), model.is_empty());
    let keys = sorted(m.keys().map(|k| k.into_owned()).collect());
    let want: Vec<String> = model.keys().cloned().collect();
    assert_eq!(keys, want, "keys at step {step}");
    for (k, v) in m.iter() {
        assert_eq!(
            model.get(k.as_ref()),
            Some(v),
            "iter value of {k} at step {step}"
        );
    }
    for (k, v) in model {
        assert_eq!(m.get(k), Some(v), "get {k} at step {step}");
        assert!(m.contains_key(k));
    }
    // Held entries and run elements partition the keys.
    let mut parts: Vec<String> = m.map_entries().map(|(k, _)| k.clone()).collect();
    for (name, first, values) in m.elem_runs() {
        for (i, v) in values.iter().enumerate() {
            let k = format!("{}[{}]", name, first + i as u64);
            assert_eq!(model.get(&k), Some(v));
            parts.push(k);
        }
    }
    assert_eq!(sorted(parts), want, "partition at step {step}");
    for name in names {
        let outer = base_of(name).unwrap_or(name);
        let under: Vec<String> = model
            .keys()
            .filter(|k| base_of(k) == Some(outer))
            .cloned()
            .collect();
        assert_eq!(
            sorted(m.elem_keys(outer)),
            under,
            "elem_keys({outer}) at step {step}"
        );
        assert_eq!(
            m.elem_gen_for_prefix(&format!("{outer}[")) == 0,
            under.is_empty(),
            "stamp of {outer} at step {step}"
        );
        for prefix in [
            format!("{name}["),
            format!("{name}[1]["),
            format!("{name}[2]."),
            format!("{name}[1"),
            format!("{name}[3]"),
            format!("{name}[0]x"),
        ] {
            let with: Vec<String> = model
                .keys()
                .filter(|k| k.starts_with(&prefix))
                .cloned()
                .collect();
            assert_eq!(
                m.any_key_with_prefix(&prefix),
                !with.is_empty(),
                "any_key_with_prefix({prefix}) at step {step}"
            );
            if prefix.ends_with('[') {
                assert_eq!(
                    sorted(m.keys_with_elem_prefix(&prefix)),
                    with,
                    "keys_with_elem_prefix({prefix}) at step {step}"
                );
                let borrowed = sorted(
                    m.elem_keys_with_prefix(&prefix)
                        .map(|k| k.into_owned())
                        .collect(),
                );
                assert_eq!(borrowed, with);
            }
        }
        // A prefix without a bracket scans every key.
        let bare = &name[..name.len().min(2)];
        if !bare.contains('[') {
            assert_eq!(
                m.any_key_with_prefix(bare),
                model.keys().any(|k| k.starts_with(bare)),
                "any_key_with_prefix({bare}) at step {step}"
            );
        }
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Random inserts, removals and edits over queue-shaped, row-shaped and
/// irregular keys: the store never differs from a plain map.
#[test]
fn runs_are_indistinguishable_from_map_entries() {
    let names = [
        "q", "12#m_q", "u.q", "g[0].q", "g[1].q", "aq[3]", "@d#7", "p::q",
    ];
    for seed in 1..=12u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut m = SignalMap::new();
        let mut model = Model::new();
        let mut stamps: BTreeMap<String, (u64, Vec<String>)> = BTreeMap::new();
        for step in 0..2500 {
            let name = names[rng.below(names.len() as u64) as usize];
            let i = rng.below(10);
            let key = match rng.below(24) {
                0 | 1 => format!("{name}.size"),
                2 => format!("{name}[{i}].f"),
                3 => format!("{name}[{i}][{}]", rng.below(3)),
                4 => format!("{name}[0{i}]"),
                5 => format!("{name}[\"k{i}\"]"),
                6 => format!("{name}[-{i}]"),
                7 => format!("{name}[{i}].size"),
                8 => name.to_string(),
                9 => format!("{name}[{}]", 1_000_000 + i),
                _ => format!("{name}[{i}]"),
            };
            let v = Value::from_u64(rng.next() & 0xffff, 16);
            match rng.below(10) {
                0..=5 => {
                    assert_eq!(m.insert(key.clone(), v.clone()), model.insert(key, v));
                }
                6..=8 => {
                    assert_eq!(m.remove(&key), model.remove(&key), "remove {key}");
                }
                _ => {
                    let got = m.get_mut(&key).map(|slot| *slot = v.clone()).is_some();
                    let want = model.get_mut(&key).map(|slot| *slot = v).is_some();
                    assert_eq!(got, want, "get_mut {key}");
                }
            }
            // A base's stamp moves whenever its key set does.
            for name in names {
                let outer = base_of(name).unwrap_or(name);
                let under: Vec<String> = model
                    .keys()
                    .filter(|k| base_of(k) == Some(outer))
                    .cloned()
                    .collect();
                let stamp = m.elem_gen_for_prefix(&format!("{outer}["));
                if let Some((old, keys)) = stamps.get(outer)
                    && *keys != under
                {
                    assert_ne!(*old, stamp, "stamp of {outer} unmoved at step {step}");
                }
                stamps.insert(outer.to_string(), (stamp, under));
            }
            if step % 7 == 0 || step > 2450 {
                check(&m, &model, &names, step);
            }
        }
        // Emptying the store leaves nothing behind.
        for k in model.keys().cloned().collect::<Vec<_>>() {
            assert!(m.remove(&k).is_some());
        }
        check(&m, &Model::new(), &names, usize::MAX);
        assert_eq!(m.elem_runs().count(), 0);
    }
}

/// The keys of collection `name` as the model holds them after its elements
/// `lo..size` shifted for a removal or an insertion at `idx`.
fn model_shift(model: &mut Model, name: &str, lo: u64, idx: u64, size: u64, open: bool) {
    let key = |i: u64| format!("{name}[{i}]");
    // Elements a shrink left behind are dropped.
    let stale: Vec<String> = model
        .keys()
        .filter(|k| {
            k.strip_prefix(name)
                .and_then(|r| r.strip_prefix('['))
                .and_then(|r| r.strip_suffix(']'))
                .and_then(|r| r.parse::<u64>().ok())
                .is_some_and(|i| i >= size)
        })
        .cloned()
        .collect();
    for k in stale {
        model.remove(&k);
    }
    if open {
        for i in (idx..size).rev() {
            let v = model[&key(i)].clone();
            model.insert(key(i + 1), v);
        }
    } else {
        for i in idx..size - 1 {
            let v = model[&key(i + 1)].clone();
            model.insert(key(i), v);
        }
        model.remove(&key(size - 1));
    }
    let _ = lo;
}

/// §7.10.2: removing or inserting an element moves the others as a block.
#[test]
fn run_shifts_match_element_by_element_moves() {
    for (name, lo) in [("q", 0u64), ("5#m_q", 0), ("g[2].u.q", 64), ("aq[1]", 0)] {
        let mut rng = Rng(0x1234_5678_9abc_def1 ^ lo);
        let mut m = SignalMap::new();
        let mut model = Model::new();
        let put = |m: &mut SignalMap, model: &mut Model, k: String, v: Value| {
            assert_eq!(m.insert(k.clone(), v.clone()), model.insert(k, v));
        };
        put(
            &mut m,
            &mut model,
            format!("{name}.size"),
            Value::from_u64(0, 32),
        );
        // A neighbour whose keys the index files under the same base.
        put(
            &mut m,
            &mut model,
            "g[2].u.other[3]".to_string(),
            Value::from_u64(9, 8),
        );
        let mut size = 0u64;
        // Nothing stored: no run covers anything.
        assert!(!m.run_covers(name, lo, lo + 1));
        for step in 0..600 {
            let v = Value::from_u64(rng.next() & 0xff_ffff, 24);
            let live = size.saturating_sub(lo);
            match rng.below(6) {
                // push_back
                0 | 1 => {
                    let at = size.max(lo);
                    put(&mut m, &mut model, format!("{name}[{at}]"), v);
                    size = at + 1;
                }
                // pop_back leaves the element behind.
                2 if live > 0 => size -= 1,
                // remove at a random index
                3 if live > 0 => {
                    let idx = lo + rng.below(live);
                    assert!(m.run_covers(name, lo, size), "covers at step {step}");
                    let want = model[&format!("{name}[{idx}]")].clone();
                    assert_eq!(m.run_remove_at(name, lo, idx, size), Some(want));
                    model_shift(&mut model, name, lo, idx, size, false);
                    size -= 1;
                }
                // insert at a random index
                4 | 5 if live > 0 => {
                    let idx = lo + rng.below(live);
                    assert!(m.run_covers(name, lo, size));
                    assert!(m.run_open_at(name, lo, idx, size));
                    model_shift(&mut model, name, lo, idx, size, true);
                    put(&mut m, &mut model, format!("{name}[{idx}]"), v);
                    size += 1;
                }
                _ => {}
            }
            check(&m, &model, &[name, "g[2].u.other"], step);
        }
        // A member of an element, a row below one, or an element held
        // outside the run: the run is no longer the whole collection.
        if size > lo + 1 {
            assert!(m.run_covers(name, lo, size));
            for extra in [
                format!("{name}[{lo}].f"),
                format!("{name}[{lo}][0]"),
                format!("{name}[{lo}].size"),
                format!("{name}[{}]", size + 5),
            ] {
                put(&mut m, &mut model, extra.clone(), Value::from_u64(1, 1));
                assert!(!m.run_covers(name, lo, size), "{extra} must not be covered");
                assert_eq!(m.remove(&extra), model.remove(&extra));
                assert!(
                    m.run_covers(name, lo, size),
                    "covered again without {extra}"
                );
            }
            assert!(!m.run_covers(name, lo + 1, size));
            assert!(!m.run_covers(name, lo, size + 100));
        }
    }
}

/// Elements stored before `name.size` move into the run when it appears,
/// and a store in any order ends as one run.
#[test]
fn elements_are_adopted_in_any_order() {
    let mut m = SignalMap::new();
    let mut model = Model::new();
    for i in [3u64, 1, 0, 2, 7] {
        let (k, v) = (format!("d[{i}]"), Value::from_u64(i, 8));
        m.insert(k.clone(), v.clone());
        model.insert(k, v);
    }
    assert_eq!(m.elem_runs().count(), 0);
    m.insert("d.size".to_string(), Value::from_u64(4, 32));
    model.insert("d.size".to_string(), Value::from_u64(4, 32));
    check(&m, &model, &["d"], 0);
    assert!(!m.run_covers("d", 0, 4), "d[7] is outside the run");
    assert_eq!(m.remove("d[7]"), model.remove("d[7]"));
    assert!(m.run_covers("d", 0, 4));
    // Descending and scattered stores.
    for order in [vec![5u64, 4, 3, 2, 1, 0], vec![2, 5, 0, 3, 1, 4]] {
        let mut m = SignalMap::new();
        m.insert("r.size".to_string(), Value::from_u64(6, 32));
        for i in &order {
            m.insert(format!("r[{i}]"), Value::from_u64(*i, 8));
        }
        assert!(m.run_covers("r", 0, 6), "order {order:?}");
        let (_, first, values) = m.elem_runs().next().unwrap();
        assert_eq!(first, 0);
        let got: Vec<u64> = values.iter().map(|v| v.to_u64().unwrap()).collect();
        assert_eq!(got, vec![0, 1, 2, 3, 4, 5]);
        // A hole splits the run; the store still answers every key.
        assert_eq!(m.remove("r[2]").and_then(|v| v.to_u64()), Some(2));
        assert!(!m.contains_key("r[2]"));
        for i in [0u64, 1, 3, 4, 5] {
            assert_eq!(m.get(&format!("r[{i}]")).and_then(|v| v.to_u64()), Some(i));
        }
        assert_eq!(m.len(), 6);
    }
}

/// A saved store reads back with its runs.
#[test]
fn runs_round_trip_through_serialization() {
    let mut m = SignalMap::new();
    m.insert("q.size".to_string(), Value::from_u64(3, 32));
    for i in 0..3u64 {
        m.insert(format!("q[{i}]"), Value::from_u64(10 + i, 8));
    }
    m.insert("q[9]".to_string(), Value::from_u64(99, 8));
    m.insert("aa[\"k\"]".to_string(), Value::from_u64(7, 8));
    let cfg = xezim_core::xez_bincode_config();
    let bytes = bincode::serde::encode_to_vec(&m, cfg).unwrap();
    let (back, _): (SignalMap, usize) = bincode::serde::decode_from_slice(&bytes, cfg).unwrap();
    let model: Model = m.iter().map(|(k, v)| (k.into_owned(), v.clone())).collect();
    check(&back, &model, &["q", "aa"], 0);
    assert!(back.run_covers("q", 0, 3) == m.run_covers("q", 0, 3));
    assert_eq!(back.elem_runs().count(), 1);
}
