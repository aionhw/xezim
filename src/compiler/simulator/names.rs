//! The flattened signal-name map (`name -> signal id`).
//!
//! A CPU design's signal table is mostly the ELEMENTS of fixed-size 1-D
//! arrays — register files, cache tag/data arrays, SRAM models: 1.51 M of
//! c906's 1.56 M signals, 2.0 M of c910's 2.4 M. Registering each element
//! as its own `"tb.x_soc...mem[123]"` string cost ~150-180 bytes per cell
//! (the `Arc<str>` with its header and size-class slack, the map entry and
//! the id-to-name pointer) — more than the cell's value and all its side
//! tables together.
//!
//! Element names are therefore VIRTUAL: a registered array's elements are
//! found arithmetically, `base[idx]` -> `first_id + (idx - lo)`, and are
//! never stored. Only the array base is kept. Every other name is an
//! ordinary map entry.
//!
//! `get` keeps the `HashMap` shape (`Option<&usize>`): a computed id is
//! handed out as a reference into `identity`, where `identity[i] == i`.

use std::sync::Arc;
use xezim_core::hasher::HashMap;

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct NameMap {
    map: HashMap<Arc<str>, usize>,
    /// Registered 1-D arrays whose element names are virtual:
    /// base -> (first_id, lo, hi).
    arrays: HashMap<Arc<str>, (usize, i64, i64)>,
    /// Virtual element count (for `len`).
    virtual_len: usize,
    /// `identity[i] == i` for every virtual element id.
    identity: Vec<usize>,
    /// first_id -> (count, base, lo), for id -> element name.
    spans: std::collections::BTreeMap<usize, (usize, Arc<str>, i64)>,
    /// Bit `len % 64` set for every virtual base's byte length: rejects most
    /// non-element `…]` names (queue slots, class properties) before the
    /// second hash lookup a real element needs.
    base_len_mask: u64,
    /// Input-port nets the elaborator left out (see
    /// `xezim_core::elaborate::set_port_elision`). A lookup that misses is
    /// checked against it, so a by-name reference the elision analysis did
    /// not foresee is reported instead of silently resolving to nothing.
    elided: Option<Arc<ElidedPorts>>,
    /// Bumped by every call that can change what a name maps to, so a
    /// caller can tell the map is untouched since it last looked.
    generation: u64,
    /// Interface port prefixes share the actual instance's signal IDs.
    interface_aliases: Option<Arc<(String, HashMap<String, String>)>>,
}

/// The elided port nets: `elided_port_hash` of each flat name, sorted.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ElidedPorts {
    hashes: Vec<u64>,
}

impl ElidedPorts {
    pub fn new(hashes: Vec<u64>) -> Self {
        ElidedPorts { hashes }
    }

    pub fn contains(&self, name: &str) -> bool {
        self.hashes
            .binary_search(&xezim_core::elaborate::elided_port_hash(name))
            .is_ok()
    }
}

/// Report a by-name lookup of an elided port net. Every one is a reference
/// the elision analysis should have kept the port for, so it is loud: a
/// one-time warning, or a panic under `XEZIM_ELIDE_STRICT=1` (the test and
/// validation mode).
#[cold]
#[inline(never)]
pub fn report_elided_lookup(name: &str) {
    if std::env::var("XEZIM_ELIDE_STRICT").ok().as_deref() == Some("1") {
        panic!("lookup of elided port net `{}`", name);
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        eprintln!(
            "xezim: warning: `{}` names an input-port net that was left out as unobserved; \
             results that depend on it may be wrong. Re-run with XEZIM_KEEP_PORTS=1 and \
             please report this.",
            name
        );
    });
}

/// `idx` must be written the way `format!("{}", i64)` writes it: the
/// element names this replaces were produced exactly that way, so e.g.
/// `mem[03]` or `mem[+3]` never named an element.
fn canonical_index(s: &str) -> Option<i64> {
    let digits = s.strip_prefix('-').unwrap_or(s);
    if digits.is_empty()
        || !digits.bytes().all(|b| b.is_ascii_digit())
        || (digits.len() > 1 && digits.starts_with('0'))
        || (digits == "0" && s.starts_with('-'))
    {
        return None;
    }
    s.parse().ok()
}

impl NameMap {
    pub fn with_capacity(n: usize) -> Self {
        NameMap {
            map: HashMap::with_capacity_and_hasher(n, Default::default()),
            ..Default::default()
        }
    }

    pub fn set_interface_aliases(&mut self, top: String, aliases: HashMap<String, String>) {
        if !aliases.is_empty() {
            self.interface_aliases = Some(Arc::new((top, aliases)));
            self.generation += 1;
        }
    }

    pub fn canonical_interface_name<'a>(&self, name: &'a str) -> std::borrow::Cow<'a, str> {
        match &self.interface_aliases {
            Some(aliases) => {
                xezim_core::elaborate::interface_ports::resolve(name, &aliases.1, &aliases.0)
            }
            None => std::borrow::Cow::Borrowed(name),
        }
    }

    /// Register the elements of array `base` (ids `first..=first+(hi-lo)`)
    /// as virtual names.
    pub fn add_virtual_array(&mut self, base: Arc<str>, first: usize, lo: i64, hi: i64) {
        self.generation += 1;
        if hi < lo {
            return;
        }
        let n = (hi - lo + 1) as usize;
        let end = first + n;
        if self.identity.len() < end {
            let start = self.identity.len();
            self.identity.extend(start..end);
        }
        self.virtual_len += n;
        self.base_len_mask |= 1u64 << (base.len() % 64);
        self.spans.insert(first, (n, base.clone(), lo));
        self.arrays.insert(base, (first, lo, hi));
    }

    /// The virtual element name of signal `id`, if it is one.
    pub fn virtual_name_of(&self, id: usize) -> Option<String> {
        let (&first, (n, base, lo)) = self.spans.range(..=id).next_back()?;
        if id >= first + n {
            return None;
        }
        Some(format!("{}[{}]", base, lo + (id - first) as i64))
    }

    /// The id of a virtual element name, if `name` is one.
    #[inline]
    pub fn virtual_id(&self, name: &str) -> Option<usize> {
        if self.base_len_mask == 0 || name.as_bytes().last() != Some(&b']') {
            return None;
        }
        self.virtual_id_slow(name)
    }

    #[inline(never)]
    fn virtual_id_slow(&self, name: &str) -> Option<usize> {
        // Walk back over the subscript's digits (and sign) to its '[': any
        // other byte means no element name, found without a full scan.
        let b = name.as_bytes();
        let mut br = b.len() - 1;
        while br > 0 && (b[br - 1].is_ascii_digit() || b[br - 1] == b'-') {
            br -= 1;
        }
        if br == 0 || b[br - 1] != b'[' {
            return None;
        }
        let br = br - 1;
        if self.base_len_mask & (1u64 << (br % 64)) == 0 {
            return None;
        }
        let idx = canonical_index(&name[br + 1..name.len() - 1])?;
        let &(first, lo, hi) = self.arrays.get(&name[..br])?;
        if idx < lo || idx > hi {
            return None;
        }
        Some(first + (idx - lo) as usize)
    }

    /// The id of `name`: a stored name, else a virtual element name. The
    /// stored-name probe is the whole cost of every lookup that is not an
    /// element name, so it inlines like the plain map it replaces.
    #[inline(always)]
    pub fn get(&self, name: &str) -> Option<&usize> {
        match self.map.get(name) {
            Some(id) => Some(id),
            None => {
                let v = self
                    .virtual_id(name)
                    .map(|id| &self.identity[id])
                    .or_else(|| {
                        self.interface_aliases
                            .as_ref()
                            .and_then(|_| self.interface_id(name))
                    });
                if v.is_none() {
                    self.audit_miss(name);
                }
                v
            }
        }
    }

    #[inline(never)]
    fn interface_id(&self, name: &str) -> Option<&usize> {
        self.interface_aliases.as_ref()?;
        let canonical = self.canonical_interface_name(name);
        if canonical == name {
            return None;
        }
        self.map
            .get(canonical.as_ref())
            .or_else(|| self.virtual_id(&canonical).map(|id| &self.identity[id]))
    }

    #[inline(always)]
    pub fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut usize> {
        if !self.map.contains_key(name) {
            self.audit_miss(name);
        }
        self.generation += 1;
        self.map.get_mut(name)
    }

    /// Every failed lookup is checked against the elided port nets only in
    /// an `elide-audit` build: the check on this path cost 0.2% on a UVM
    /// run. Release builds check the resolution points that can plausibly
    /// reach a port net by name (see [`NameMap::check_miss`]).
    #[inline(always)]
    fn audit_miss(&self, _name: &str) {
        #[cfg(feature = "elide-audit")]
        self.check_miss(_name);
    }

    /// Report `name` if it is an elided port net; for a lookup that missed.
    #[inline(always)]
    pub fn check_miss(&self, name: &str) {
        if self.elided.is_some() {
            self.elided_miss(name);
        }
    }

    /// Install the elided port nets (see [`ElidedPorts`]).
    pub fn set_elided(&mut self, elided: Option<Arc<ElidedPorts>>) {
        self.elided = elided;
    }

    /// Whether `name` is an elided port net.
    pub fn is_elided(&self, name: &str) -> bool {
        self.elided.as_ref().is_some_and(|e| e.contains(name))
    }

    #[cold]
    #[inline(never)]
    fn elided_miss(&self, name: &str) {
        if self.is_elided(name) {
            report_elided_lookup(name);
        }
    }

    pub fn insert(&mut self, name: Arc<str>, id: usize) -> Option<usize> {
        self.generation += 1;
        self.map.insert(name, id)
    }

    /// Changes whenever a name may have been added or re-pointed.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn reserve(&mut self, n: usize) {
        self.map.reserve(n);
    }

    /// Every registered name, virtual elements included.
    pub fn len(&self) -> usize {
        self.map.len() + self.virtual_len
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The EXPLICIT entries only (no virtual element names).
    pub fn iter(&self) -> impl Iterator<Item = (&Arc<str>, &usize)> {
        self.map.iter()
    }

    /// The EXPLICIT names only (no virtual element names).
    pub fn keys(&self) -> impl Iterator<Item = &Arc<str>> {
        self.map.keys()
    }

    /// Arrays whose element names are virtual: (base, first_id, lo, hi).
    pub fn virtual_arrays(&self) -> impl Iterator<Item = (&Arc<str>, usize, i64, i64)> {
        self.arrays.iter().map(|(b, &(f, lo, hi))| (b, f, lo, hi))
    }

    /// Visit every registered name — explicit entries, then each virtual
    /// element (materialized in a scratch buffer) — until `f` returns false.
    pub fn for_each_name(&self, mut f: impl FnMut(&str, usize) -> bool) {
        for (k, &id) in self.map.iter() {
            if !f(k, id) {
                return;
            }
        }
        let mut buf = String::new();
        for (base, &(first, lo, hi)) in self.arrays.iter() {
            for idx in lo..=hi {
                use std::fmt::Write as _;
                buf.clear();
                buf.push_str(base);
                let _ = write!(buf, "[{}]", idx);
                if !f(&buf, first + (idx - lo) as usize) {
                    return;
                }
            }
        }
    }

    /// Every registered name starting with `prefix`, virtual elements
    /// included (element names are materialized only for arrays whose name
    /// can match).
    pub fn names_with_prefix(&self, prefix: &str) -> Vec<Arc<str>> {
        let mut out: Vec<Arc<str>> = self
            .map
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect();
        for (base, &(_, lo, hi)) in self.arrays.iter() {
            // `base[idx]` starts with `prefix` iff the prefix is a prefix of
            // the base, or reaches past it into the subscript.
            let reach = if base.starts_with(prefix) {
                true
            } else {
                prefix.starts_with(base.as_ref()) && prefix[base.len()..].starts_with('[')
            };
            if !reach {
                continue;
            }
            for idx in lo..=hi {
                let n = format!("{}[{}]", base, idx);
                if n.starts_with(prefix) {
                    out.push(Arc::from(n));
                }
            }
        }
        out
    }

    /// Whether any registered name ends with `suffix`.
    pub fn any_name_ends_with(&self, suffix: &str) -> bool {
        if self.map.keys().any(|k| k.ends_with(suffix)) {
            return true;
        }
        // A virtual element `base[idx]` ends with `suffix` when the suffix
        // is `...leaf[idx]` for an in-range idx and the base ends with the
        // part before the subscript.
        let Some(br) = suffix.rfind('[') else {
            return false;
        };
        if !suffix.ends_with(']') {
            return false;
        }
        let Some(idx) = canonical_index(&suffix[br + 1..suffix.len() - 1]) else {
            return false;
        };
        let head = &suffix[..br];
        self.arrays
            .iter()
            .any(|(base, &(_, lo, hi))| base.ends_with(head) && idx >= lo && idx <= hi)
    }

    /// The explicit map's heap estimate, for the memory census.
    pub fn explicit_len(&self) -> usize {
        self.map.len()
    }
}

/// Signal id -> its registered name. Ids without a stored name — cells of
/// an array whose element names are virtual (see `NameMap`) or were never
/// registered at all (a bulk memory) — form GAPS: no per-cell entry, so the
/// stored names stay aligned with their ids at no per-cell cost.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct IdNames {
    names: Vec<Arc<str>>,
    /// (first id, len, names stored before it), ascending.
    gaps: Vec<(usize, usize, usize)>,
    /// Ids covered so far (names + gap cells).
    total: usize,
    /// Stored names plus VIRTUAL gap cells: the number of names the table
    /// held when every virtual element name was stored (see `legacy_len`).
    legacy: usize,
    /// One past the last id with a stored or virtual name (see
    /// `named_id_bound`).
    named_end: usize,
    /// Id ranges `[start, end)` with no name at all, stored or virtual (the
    /// cells of a bulk memory), ascending and merged.
    unnamed: Vec<(usize, usize)>,
}

impl IdNames {
    pub fn with_capacity(n: usize) -> Self {
        IdNames {
            names: Vec::with_capacity(n),
            ..Default::default()
        }
    }

    /// The next id's name.
    pub fn push(&mut self, name: Arc<str>) {
        self.names.push(name);
        self.total += 1;
        self.legacy += 1;
        self.named_end = self.total;
    }

    /// The next `n` ids have no stored name; `virtual_names` when they are
    /// the elements of a virtual-name array (see `NameMap`).
    pub fn push_gap(&mut self, n: usize, virtual_names: bool) {
        if n == 0 {
            return;
        }
        if virtual_names {
            self.legacy += n;
        }
        match self.gaps.last_mut() {
            Some(g) if g.0 + g.1 == self.total => g.1 += n,
            _ => self.gaps.push((self.total, n, self.names.len())),
        }
        let start = self.total;
        self.total += n;
        if virtual_names {
            self.named_end = self.total;
        } else {
            match self.unnamed.last_mut() {
                Some(r) if r.1 == start => r.1 = self.total,
                _ => self.unnamed.push((start, self.total)),
            }
        }
    }

    /// Does `id` have no name at all, stored or virtual — a bulk memory's
    /// cell, which no name-keyed dependency can reach?
    #[inline]
    pub fn is_unnamed(&self, id: usize) -> bool {
        if self.unnamed.is_empty() {
            return false;
        }
        let k = self.unnamed.partition_point(|r| r.0 <= id);
        k > 0 && id < self.unnamed[k - 1].1
    }

    pub fn reserve(&mut self, n: usize) {
        self.names.reserve(n);
    }

    /// Ids covered (named and gap).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.total
    }

    /// Registered names, virtual elements included — the length the table
    /// had when element names were stored (a count, not an id bound).
    pub fn legacy_len(&self) -> usize {
        self.legacy
    }

    /// Bound on every id a NAME can resolve to: one past the last id with a
    /// stored or virtual name. Only the cells of unnamed bulk memories lie
    /// beyond it — and, when such a memory sorts before a named array, in
    /// the middle of it too, which is why `legacy_len` (a count) is no id
    /// bound.
    pub fn named_id_bound(&self) -> usize {
        self.named_end
    }

    /// The stored name of `id`; None for a gap id or an id past the end.
    #[inline]
    pub fn get(&self, id: usize) -> Option<&Arc<str>> {
        if id >= self.total {
            return None;
        }
        if self.gaps.is_empty() {
            return self.names.get(id);
        }
        // Last gap starting at or before `id`.
        let k = self.gaps.partition_point(|g| g.0 <= id);
        if k == 0 {
            return self.names.get(id);
        }
        let (start, len, before) = self.gaps[k - 1];
        if id < start + len {
            return None;
        }
        self.names.get(before + (id - start - len))
    }

    /// (id, name) for every id with a stored name, ascending.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &Arc<str>)> {
        let mut gi = 0usize;
        let mut id = 0usize;
        let gaps = &self.gaps;
        self.names.iter().map(move |n| {
            while gi < gaps.len() && gaps[gi].0 == id {
                id += gaps[gi].1;
                gi += 1;
            }
            let r = (id, n);
            id += 1;
            r
        })
    }

    /// Heap bytes of the stored name pointers (census).
    pub fn ptr_capacity(&self) -> usize {
        self.names.capacity()
    }
}

/// Sort `names` into byte-lexicographic order, the order `slice::sort` on
/// strings gives (equal names end up adjacent, in no particular order).
///
/// A flattened design's names share long prefixes (`tb.dut.u_bank[0].bank.
/// u_mat[3]...`, 65 bytes on average on a DRAM array), so a comparison sort
/// spends its time comparing the same prefixes again and again, each time
/// chasing two pointers to scattered heap strings. This sort compares eight
/// bytes at a time held inline next to the index: one pass orders the whole
/// set by bytes `[0, 8)`, and only a run of names that tie on them is ordered
/// by the next eight, so each name's bytes are read about once.
pub fn sort_lexicographic<S: AsRef<str>>(names: &mut Vec<S>) {
    fn chunk(s: &[u8], d: usize) -> u64 {
        let mut buf = [0u8; 8];
        if d < s.len() {
            let n = (s.len() - d).min(8);
            buf[..n].copy_from_slice(&s[d..d + n]);
        }
        u64::from_be_bytes(buf)
    }
    // Every item of `items` shares its first `d` bytes with the others.
    fn sort_from(items: &mut [(u64, u32)], bytes: &[&[u8]], d: usize) {
        let suffix = |i: u32| bytes[i as usize].get(d..).unwrap_or(&[]);
        if items.len() <= 32 {
            items.sort_unstable_by(|a, b| suffix(a.1).cmp(suffix(b.1)));
            return;
        }
        for it in items.iter_mut() {
            it.0 = chunk(bytes[it.1 as usize], d);
        }
        items.sort_unstable_by_key(|it| it.0);
        let mut i = 0;
        while i < items.len() {
            let k = items[i].0;
            let mut j = i + 1;
            while j < items.len() && items[j].0 == k {
                j += 1;
            }
            if j - i > 1 {
                if k & 0xff != 0 {
                    // Every name of the run is longer than `d + 8` bytes
                    // and they all agree on `[d, d + 8)`.
                    sort_from(&mut items[i..j], bytes, d + 8);
                } else {
                    // Some name of the run ends inside this chunk (or
                    // holds a NUL byte there): compare exactly.
                    items[i..j].sort_unstable_by(|a, b| suffix(a.1).cmp(suffix(b.1)));
                }
            }
            i = j;
        }
    }
    if names.len() < 2 {
        return;
    }
    assert!(names.len() <= u32::MAX as usize);
    let mut items: Vec<(u64, u32)> = (0..names.len() as u32).map(|i| (0, i)).collect();
    {
        let bytes: Vec<&[u8]> = names.iter().map(|n| n.as_ref().as_bytes()).collect();
        sort_from(&mut items, &bytes, 0);
    }
    let mut slots: Vec<Option<S>> = std::mem::take(names).into_iter().map(Some).collect();
    names.extend(
        items
            .into_iter()
            .map(|(_, i)| slots[i as usize].take().unwrap()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexicographic_sort_matches_slice_sort() {
        // Hierarchical names with long shared prefixes, bytes that sort
        // around '.', prefixes of each other, duplicates, NULs and the empty
        // name.
        let mut input: Vec<String> = Vec::new();
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |m: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % m
        };
        for _ in 0..5000 {
            let mut n = String::from("tb.dut.u_bank");
            n.push_str(&format!("[{}].bank.u_mat[{}]", next(4), next(8)));
            match next(5) {
                0 => {}
                1 => n.push_str(&format!(".mat.u_row[{}].q", next(130))),
                2 => n.push_str(&format!("_{}", next(10))),
                3 => n.push_str(&format!(".mat.u_row[{}]", next(130))),
                _ => n.push_str(&format!(".x{}\0y", next(3))),
            }
            input.push(n);
        }
        for extra in [
            "", "a", "a.b", "a_b", "a.b.c", "a\0", "a\0b", "tb", "tb.", "zz",
        ] {
            input.push(extra.to_string());
            input.push(extra.to_string());
        }
        let mut want = input.clone();
        want.sort();
        let mut got = input.clone();
        sort_lexicographic(&mut got);
        assert_eq!(got, want);
        let mut borrowed: Vec<std::borrow::Cow<'_, str>> = input
            .iter()
            .map(|s| std::borrow::Cow::Borrowed(s.as_str()))
            .collect();
        sort_lexicographic(&mut borrowed);
        assert!(
            borrowed
                .iter()
                .map(|c| c.as_ref())
                .eq(want.iter().map(|s| s.as_str()))
        );
    }

    #[test]
    fn virtual_element_names_resolve_like_stored_ones() {
        let mut m = NameMap::default();
        m.insert(Arc::from("top.a"), 0);
        m.add_virtual_array(Arc::from("top.mem"), 1, -2, 1);
        assert_eq!(m.get("top.a"), Some(&0));
        assert_eq!(m.get("top.mem[-2]"), Some(&1));
        assert_eq!(m.get("top.mem[1]"), Some(&4));
        assert_eq!(m.get("top.mem[2]"), None, "out of range");
        assert_eq!(m.get("top.mem[01]"), None, "not a canonical index");
        assert_eq!(m.get("top.mem[+1]"), None);
        assert_eq!(m.get("top.mem[-0]"), None);
        assert_eq!(m.get("top.mem"), None, "the base is not a signal");
        assert_eq!(m.len(), 5);
        // An explicit entry wins over the virtual one.
        m.insert(Arc::from("top.mem[0]"), 9);
        assert_eq!(m.get("top.mem[0]"), Some(&9));
        assert_eq!(m.virtual_name_of(2).as_deref(), Some("top.mem[-1]"));
        assert_eq!(m.virtual_name_of(0), None);
        assert_eq!(m.virtual_name_of(5), None);
        assert!(m.any_name_ends_with(".mem[1]"));
        assert!(!m.any_name_ends_with(".mem[5]"));
        let mut p = m.names_with_prefix("top.mem[");
        p.sort();
        assert_eq!(p.len(), 5);
        let mut all = 0;
        m.for_each_name(|_, _| {
            all += 1;
            true
        });
        assert_eq!(all, 6);
    }

    #[test]
    fn id_names_keep_ids_aligned_across_gaps() {
        let mut n = IdNames::default();
        n.push(Arc::from("a"));
        n.push_gap(3, true);
        n.push(Arc::from("b"));
        n.push_gap(2, false);
        n.push_gap(1, true);
        n.push(Arc::from("c"));
        assert_eq!(n.len(), 9);
        assert_eq!(n.legacy_len(), 3 + 3 + 1);
        assert_eq!(n.named_id_bound(), 9, "c follows the bulk gap");
        assert_eq!(n.get(0).map(|s| &**s), Some("a"));
        assert!(n.get(1).is_none() && n.get(3).is_none());
        assert_eq!(n.get(4).map(|s| &**s), Some("b"));
        assert!(n.get(5).is_none() && n.get(7).is_none());
        assert_eq!(n.get(8).map(|s| &**s), Some("c"));
        assert!(n.get(9).is_none());
        let ids: Vec<(usize, &str)> = n.iter().map(|(i, s)| (i, &**s)).collect();
        assert_eq!(ids, vec![(0, "a"), (4, "b"), (8, "c")]);
        // A trailing bulk memory holds no named id.
        n.push_gap(4, false);
        assert_eq!((n.len(), n.named_id_bound()), (13, 9));
        let unnamed: Vec<usize> = (0..14).filter(|&i| n.is_unnamed(i)).collect();
        assert_eq!(
            unnamed,
            [5, 6, 9, 10, 11, 12],
            "bulk gaps only, not virtual ones"
        );
    }
}
