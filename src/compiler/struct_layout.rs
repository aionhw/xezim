//! Interned packed-struct / untagged-union layout descriptors and 2-state
//! masks.
//!
//! Elaboration stores `packed_struct_fields` as `SharedSliceMap<(String, u32, u32)>`.
//! At compile / simulation setup time we intern each distinct field slice into
//! one `Arc<StructLayout>` with an O(1) `HashMap<Arc<str>, FieldSlice>` lookup
//! and an optional precomputed 2-state bitmask (eliminating linear vector scans,
//! `bc_packed_fields` cloning, and per-access `HashSet<String>` AST walks).

use std::sync::Arc;
use xezim_core::Value;
use xezim_core::hasher::{HashMap, HashSet};
use xezim_core::value::LogicBit;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FieldSlice {
    pub offset: u32,
    pub width: u32,
    pub is_two_state: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct StructLayout {
    pub total_width: u32,
    pub fields: HashMap<Arc<str>, FieldSlice>,
    pub ordered: Box<[(Arc<str>, u32, u32)]>,
    pub is_mixed: bool,
    pub two_state_mask: Option<Value>,
}

impl StructLayout {
    pub(crate) fn build(
        raw: &[(String, u32, u32)],
        is_mixed: bool,
        two_state_leaves: Option<&HashSet<String>>,
    ) -> Self {
        let total_width = raw.iter().map(|(_, o, w)| o.saturating_add(*w)).max().unwrap_or(0);
        let mut fields: HashMap<Arc<str>, FieldSlice> =
            HashMap::with_capacity_and_hasher(raw.len(), Default::default());
        let mut ordered: Vec<(Arc<str>, u32, u32)> = Vec::with_capacity(raw.len());
        let mut mask = if total_width > 0 && two_state_leaves.is_some_and(|ts| !ts.is_empty()) {
            Some(Value::zero(total_width))
        } else {
            None
        };

        for (name, offset, width) in raw {
            let key: Arc<str> = Arc::from(name.as_str());
            let is_two_state = two_state_leaves.is_some_and(|ts| ts.contains(name.as_str()));
            if is_two_state
                && *width > 0
                && let Some(m) = mask.as_mut()
            {
                let mut ones = Value::zero(*width);
                if *width <= 64 {
                    for b in 0..(*width as usize) {
                        ones.set_bit(b, LogicBit::One);
                    }
                } else {
                    ones = Value::from_str_radix(&"1".repeat(*width as usize), 2, *width);
                }
                m.copy_bits_from(*offset as usize, &ones, 0, *width as usize);
            }
            fields.insert(
                Arc::clone(&key),
                FieldSlice {
                    offset: *offset,
                    width: *width,
                    is_two_state,
                },
            );
            ordered.push((key, *offset, *width));
        }

        if let Some(m) = &mask
            && m.is_nonzero() == Some(false)
        {
            mask = None;
        }

        Self {
            total_width,
            fields,
            ordered: ordered.into_boxed_slice(),
            is_mixed,
            two_state_mask: mask,
        }
    }

    #[inline]
    pub(crate) fn get(&self, member: &str) -> Option<&FieldSlice> {
        self.fields.get(member)
    }
}
