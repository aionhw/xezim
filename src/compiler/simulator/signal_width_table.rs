//! Widths for name-keyed signal storage. A fixed class member's declared
//! width covers every indexed cell; only individual changes need storage.

use super::{HashMap, split_elem_key};
use std::borrow::Cow;

#[derive(Default)]
pub struct SignalWidths {
    named: HashMap<String, u32>,
    ranges: HashMap<String, WidthRange>,
}

struct WidthRange {
    first: u64,
    last: u64,
    width: u32,
    // None represents a removed key. A mutable lookup only materializes
    // the selected width, so it cannot alter any sibling's default.
    changes: HashMap<u64, Option<u32>>,
}

impl WidthRange {
    fn new(first: u64, last: u64, width: u32) -> Self {
        Self {
            first,
            last,
            width,
            changes: HashMap::default(),
        }
    }

    fn contains(&self, i: u64) -> bool {
        (self.first..=self.last).contains(&i)
    }

    fn get(&self, i: u64) -> Option<&u32> {
        match self.changes.get(&i) {
            Some(explicit) => explicit.as_ref(),
            None => Some(&self.width),
        }
    }
}

impl SignalWidths {
    fn range(&self, key: &str) -> Option<(&WidthRange, u64)> {
        if self.ranges.is_empty() {
            return None;
        }
        let (name, i) = split_elem_key(key)?;
        let range = self.ranges.get(name)?;
        range.contains(i).then_some((range, i))
    }

    fn range_mut(&mut self, key: &str) -> Option<(&mut WidthRange, u64)> {
        if self.ranges.is_empty() {
            return None;
        }
        let (name, i) = split_elem_key(key)?;
        let range = self.ranges.get_mut(name)?;
        range.contains(i).then_some((range, i))
    }

    pub fn get(&self, key: &str) -> Option<&u32> {
        match self.range(key) {
            Some((r, i)) => r.get(i),
            None => self.named.get(key),
        }
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut u32> {
        if !self.ranges.is_empty()
            && let Some((name, i)) = split_elem_key(key)
            && let Some(r) = self.ranges.get_mut(name)
            && r.contains(i)
        {
            return r.changes.entry(i).or_insert(Some(r.width)).as_mut();
        }
        self.named.get_mut(key)
    }

    pub fn insert(&mut self, key: String, width: u32) -> Option<u32> {
        if let Some((r, i)) = self.range_mut(&key) {
            let prev = r.get(i).copied();
            if width == r.width {
                r.changes.remove(&i);
            } else {
                r.changes.insert(i, Some(width));
            }
            return prev;
        }
        self.named.insert(key, width)
    }

    pub fn remove(&mut self, key: &str) -> Option<u32> {
        if let Some((r, i)) = self.range_mut(key) {
            let prev = r.get(i).copied();
            r.changes.insert(i, None);
            return prev;
        }
        self.named.remove(key)
    }

    pub fn len(&self) -> usize {
        self.named.len()
            + self
                .ranges
                .values()
                .map(|r| {
                    (r.last - r.first + 1) as usize
                        - r.changes.values().filter(|w| w.is_none()).count()
                })
                .sum::<usize>()
    }

    pub(crate) fn storage_counts(&self) -> (usize, usize) {
        (self.named.len(), self.ranges.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = (Cow<'_, str>, &u32)> {
        self.named
            .iter()
            .map(|(name, w)| (Cow::Borrowed(name.as_str()), w))
            .chain(self.ranges.iter().flat_map(|(name, r)| {
                (r.first..=r.last)
                    .filter_map(move |i| Some((Cow::Owned(format!("{name}[{i}]")), r.get(i)?)))
            }))
    }

    pub fn keys(&self) -> impl Iterator<Item = Cow<'_, str>> {
        self.iter().map(|(key, _)| key)
    }

    pub fn values(&self) -> impl Iterator<Item = &u32> {
        self.named.values().chain(
            self.ranges
                .values()
                .flat_map(|r| (r.first..=r.last).filter_map(move |i| r.get(i))),
        )
    }

    /// Assign the declared width to each key in [first, last]. Construction
    /// and specialization both use this operation; old cells outside the
    /// bounds retain their widths, as for individual map inserts.
    pub(super) fn declare_fixed(&mut self, name: &str, first: u64, last: u64, width: u32) {
        if let Some(old) = self.ranges.get(name)
            && (first > old.first || last < old.last)
        {
            for i in first..=last {
                self.insert(format!("{name}[{i}]"), width);
            }
            return;
        }
        // Drop any named cells the new range replaces. For a small range,
        // inspect only its keys; for a large range, inspect the smaller map.
        if self.named.len() as u64 > last - first + 1 {
            for i in first..=last {
                self.named.remove(&format!("{name}[{i}]"));
            }
        } else {
            self.named.retain(|key, _| {
                !matches!(split_elem_key(key),
                Some((base, i)) if name == base && (first..=last).contains(&i))
            });
        }
        self.ranges
            .insert(name.to_owned(), WidthRange::new(first, last, width));
    }

    pub(super) fn specialize_fixed(&mut self, name: &str, lo: i64, hi: i64, width: u32) -> bool {
        if lo < 0 || lo > hi || hi >= 1_000_000_000 || !self.ranges.contains_key(name) {
            return false;
        }
        self.declare_fixed(name, lo as u64, hi as u64, width);
        true
    }

    /// Class GC tests ownership by the prefix preceding the first '#'.
    /// Every indexed name in a range belongs to the same owner.
    pub(super) fn remove_dead_owners(&mut self, dead: impl Fn(&str) -> bool) {
        self.named.retain(|name, _| !dead(name));
        self.ranges.retain(|name, _| !dead(name));
    }
}

impl<K: AsRef<str> + ?Sized> std::ops::Index<&K> for SignalWidths {
    type Output = u32;

    fn index(&self, key: &K) -> &Self::Output {
        self.get(key.as_ref()).expect("no entry found for key")
    }
}

impl<'a> IntoIterator for &'a SignalWidths {
    type Item = (Cow<'a, str>, &'a u32);
    type IntoIter = Box<dyn Iterator<Item = Self::Item> + 'a>;

    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

// Checkpoints retain the original map-of-string-to-u32 wire format. The
// restored design's fixed array bounds recover the compact representation.
impl serde::Serialize for SignalWidths {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.len()))?;
        for (k, v) in self.iter() {
            map.serialize_entry(k.as_ref(), v)?;
        }
        map.end()
    }
}

impl<'de> serde::Deserialize<'de> for SignalWidths {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self {
            named: HashMap::deserialize(d)?,
            ranges: HashMap::default(),
        })
    }
}

impl super::Simulator {
    pub(super) fn restore_class_width_ranges(&mut self) {
        for (name, first, values) in self.signals.elem_runs() {
            let Some(&(_, _, width)) = self.module.arrays.get(name) else {
                continue;
            };
            if Self::class_storage_owner(name).is_none() {
                continue;
            }
            let last = first + values.len() as u64 - 1;
            let mut range = WidthRange::new(first, last, width);
            for i in first..=last {
                let stored = self.widths.named.remove(&format!("{name}[{i}]"));
                if stored != Some(width) {
                    range.changes.insert(i, stored);
                }
            }
            self.widths.ranges.insert(name.to_owned(), range);
        }
        if !self.widths.ranges.is_empty() {
            self.widths.named.shrink_to_fit();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redeclared_widths_overwrite_only_their_own_bounds() {
        let mut w = SignalWidths::default();
        w.insert("1#d[1]".into(), 5);
        w.insert("1#d[4]".into(), 99);
        w.insert("12#d[4]".into(), 1);
        w.declare_fixed("1#d", 3, 5, 8);
        assert_eq!(w.storage_counts(), (2, 1));
        w.insert("1#d[5]".into(), 9);
        w.insert("1#d[6]".into(), 111);
        assert!(w.specialize_fixed("1#d", 4, 7, 32));
        assert_eq!(w["1#d[3]"], 8);
        for i in 4..=7 {
            assert_eq!(w[&format!("1#d[{i}]")], 32);
        }
        w.remove("1#d[4]");
        assert!(w.specialize_fixed("1#d", 2, 7, 16));
        assert_eq!(w.storage_counts(), (2, 1));
        assert_eq!(w.len(), 8);
        assert_eq!(w.iter().count(), 8);
        assert_eq!(w["1#d[1]"], 5);
        assert_eq!(w["1#d[4]"], 16);
        assert_eq!(w["12#d[4]"], 1);
    }

    #[test]
    fn fixed_widths_keep_independent_edits_and_legacy_map_encoding() {
        let mut table = SignalWidths::default();
        table.declare_fixed("17#data", 4, 9, 8);
        table.declare_fixed("18#data", 4, 9, 16);
        table.insert("other".into(), 1);
        assert_eq!(table.get("17#data[03]"), None);
        assert_eq!(table.get("17#data[3]"), None);
        assert_eq!(table.get("17#data[10]"), None);
        assert_eq!(table.insert("17#data[5]".into(), 32), Some(8));
        *table.get_mut("17#data[6]").unwrap() = 64;
        assert_eq!(table.remove("17#data[7]"), Some(8));
        assert_eq!(table.get_mut("17#data[7]"), None);
        assert_eq!(table.insert("17#data[7]".into(), 8), None);
        assert_eq!(table.remove("17#data[8]"), Some(8));
        assert_eq!(table.len(), 12);
        assert_eq!(table.keys().count(), table.len());
        assert_eq!(table.values().count(), table.len());
        assert_eq!(table["17#data[5]"], 32);
        assert_eq!((&table).into_iter().count(), table.len());

        let cfg = xezim_core::xez_bincode_config();
        let bytes = bincode::serde::encode_to_vec(&table, cfg).unwrap();
        let (legacy, _): (HashMap<String, u32>, _) =
            bincode::serde::decode_from_slice(&bytes, cfg).unwrap();
        assert_eq!(legacy.len(), 12);
        assert!(!legacy.contains_key("17#data[8]"));
        assert_eq!(legacy["17#data[5]"], 32);
        assert_eq!(legacy["17#data[6]"], 64);
        assert_eq!(legacy["17#data[7]"], 8);
        let bytes = bincode::serde::encode_to_vec(&legacy, cfg).unwrap();
        let (restored, _): (SignalWidths, _) =
            bincode::serde::decode_from_slice(&bytes, cfg).unwrap();
        for (k, w) in &legacy {
            assert_eq!(restored.get(k), Some(w));
        }

        table.remove_dead_owners(|name| name.starts_with("17#"));
        assert_eq!(table.storage_counts(), (1, 1));
        assert_eq!(table.len(), 7);
        assert_eq!(table.get("17#data[5]"), None);
        assert_eq!(table.get("18#data[5]"), Some(&16));
    }
}
