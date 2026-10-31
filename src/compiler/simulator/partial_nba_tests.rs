//! IEEE 1800-2023 §10.4.2: selected NBA bits publish in source order.

use super::{LogicBit, NbaFast, Simulator, Value};

const WIDTHS: &[u32] = &[1, 63, 64, 65, 127, 128, 129, 257, 1024];

fn patterned(width: u32, phase: usize) -> Value {
    let mut value = Value::zero(width);
    for bit in 0..width as usize {
        value.set_bit(
            bit,
            match (bit + phase) % 4 {
                0 => LogicBit::Zero,
                1 => LogicBit::One,
                2 => LogicBit::X,
                _ => LogicBit::Z,
            },
        );
    }
    value
}

fn bits(value: &Value) -> Vec<LogicBit> {
    (0..value.width as usize)
        .map(|bit| value.get_bit(bit))
        .collect()
}

fn overwrite(expected: &mut [LogicBit], lo: u32, source: &Value) {
    for bit in 0..source.width as usize {
        if let Some(target) = expected.get_mut(lo as usize + bit) {
            *target = source.get_bit(bit);
        }
    }
}

fn simulator() -> Simulator {
    let mut source = "module top;".to_string();
    for width in WIDTHS {
        source.push_str(&format!("logic [{}:0] q{width}='0;", width - 1));
    }
    source.push_str("initial #1 $finish; endmodule");
    crate::simulate(&source, 2).expect("NBA fixture elaborates")
}

fn id(sim: &Simulator, width: u32) -> usize {
    let name = format!("q{width}");
    *sim.signal_name_to_id
        .get(name.as_str())
        .or_else(|| sim.signal_name_to_id.get(format!("top.{name}").as_str()))
        .expect("fixture signal exists")
}

fn publish(sim: &mut Simulator) {
    sim.nba_fast_index.clear();
    let entries = std::mem::take(&mut sim.nba_fast);
    for entry in entries {
        sim.apply_nba_entry(entry);
    }
}

#[test]
fn selected_ranges_union_across_word_boundaries_and_clip_to_width() {
    for &width in WIDTHS {
        let mut entry = NbaFast::partial(0, Value::zero(width), width, 0, 0);
        let mut selected = vec![false; width as usize];
        for (lo, len) in [
            (0, 1),
            (61, 7),
            (63, 64),
            (64, 65),
            (126, 5),
            (255, 65),
            (width.saturating_sub(3), 8),
            (width, 1),
            (u32::MAX - 2, 8),
            (width / 2, u32::MAX),
        ] {
            entry.include_range(lo, len);
            for (bit, expected) in selected.iter_mut().enumerate() {
                *expected |= lo <= bit as u32 && (bit as u32) < lo.saturating_add(len);
            }
            let mask = entry.mask.as_ref().expect("partial mask retained");
            assert_eq!(mask.width, width);
            for (bit, &expected) in selected.iter().enumerate() {
                assert_eq!(
                    mask.get_bit(bit),
                    if expected {
                        LogicBit::One
                    } else {
                        LogicBit::Zero
                    },
                    "width={width} bit={bit} after range {lo}+:{len}"
                );
            }
        }
        let mut whole = NbaFast {
            signal_id: 0,
            value: patterned(width, 1),
            mask: None,
            block_index: 0,
        };
        whole.include_range(0, width);
        assert!(
            whole.mask.is_none(),
            "a partial update cannot narrow a whole NBA"
        );
    }
}

#[test]
fn masked_publication_selects_only_known_ones_and_preserves_other_planes() {
    for &width in WIDTHS {
        for source_width in [0, 1, width / 2, width, width + 65] {
            for mask_width in [0, width / 2, width, width + 65] {
                let mut current = patterned(width, 0);
                let source = patterned(source_width, 1);
                let mask = patterned(mask_width, 2);
                let mut expected = bits(&current);
                for (bit, target) in expected.iter_mut().enumerate() {
                    if bit < mask.width as usize && mask.get_bit(bit) == LogicBit::One {
                        *target = source.get_bit(bit);
                    }
                }
                NbaFast::merge_masked(&mut current, &source, &mask);
                assert_eq!(
                    bits(&current),
                    expected,
                    "width={width} source_width={source_width} mask_width={mask_width}"
                );
                assert_eq!(current.width, width);
            }
        }
        for fill_bit in [LogicBit::One, LogicBit::X, LogicBit::Z] {
            let mut current = patterned(width, 0);
            let mut fill = Value::zero(1);
            fill.set_bit(0, fill_bit);
            fill.is_fill = true;
            let mask = patterned(width + 1, 1);
            let mut expected = bits(&current);
            for (bit, target) in expected.iter_mut().enumerate() {
                if mask.get_bit(bit) == LogicBit::One {
                    *target = fill.get_bit(bit);
                }
            }
            NbaFast::merge_masked(&mut current, &fill, &mask);
            assert_eq!(bits(&current), expected, "width={width} fill={fill_bit:?}");
        }
    }
}

#[test]
fn queued_disjoint_and_overlapping_slices_preserve_later_blocking_writes() {
    let mut sim = simulator();
    for &width in WIDTHS {
        let id = id(&sim, width);
        let initial = patterned(width, 0);
        let live = patterned(width, 2);
        sim.signal_table[id] = initial.clone();
        let lo = (width / 2).saturating_sub(1);
        let overlap = lo.saturating_add(2).min(width - 1);
        let writes = [
            (0, patterned(width.min(3), 1)),
            (lo, patterned((width - lo).min(7), 2)),
            (width - 1, patterned(1, 1)),
            (overlap, patterned((width - overlap).min(3), 3)),
        ];
        let mut expected = bits(&live);
        for (lo, part) in writes {
            overwrite(&mut expected, lo, &part);
            sim.queue_partial_nba(id, lo, part);
        }
        // Active-region writes happen after RHS sampling. They must remain
        // visible in every bit that none of the queued selections overwrote.
        sim.signal_table[id] = live;
        assert_eq!(sim.nba_fast.len(), 1);
        publish(&mut sim);
        assert_eq!(bits(&sim.signal_table[id]), expected, "width={width}");

        // An equal selected value is still an assignment: a later blocking
        // write to that selected bit must be overwritten in the NBA region.
        sim.signal_table[id] = Value::zero(width);
        sim.queue_partial_nba(id, width - 1, Value::zero(1));
        sim.signal_table[id].set_bit((width - 1) as usize, LogicBit::One);
        publish(&mut sim);
        assert_eq!(
            sim.signal_table[id].get_bit((width - 1) as usize),
            LogicBit::Zero
        );
    }
}

#[test]
fn whole_and_partial_assignments_keep_last_write_order() {
    let mut sim = simulator();
    for &width in WIDTHS {
        let id = id(&sim, width);
        let whole = patterned(width, 1);
        let part = patterned(width.min(65), 3);
        let lo = width - part.width;

        sim.signal_table[id] = patterned(width, 0);
        sim.queue_partial_nba(id, lo, part.clone());
        sim.ts_store_nba_val(id, whole.clone());
        assert!(sim.nba_fast[0].mask.is_none());
        sim.signal_table[id] = patterned(width, 2);
        publish(&mut sim);
        assert_eq!(
            bits(&sim.signal_table[id]),
            bits(&whole),
            "partial then whole, width={width}"
        );

        sim.signal_table[id] = patterned(width, 0);
        sim.ts_store_nba_val(id, whole.clone());
        sim.queue_partial_nba(id, lo, part.clone());
        assert!(sim.nba_fast[0].mask.is_none());
        let mut expected = bits(&whole);
        overwrite(&mut expected, lo, &part);
        sim.signal_table[id] = patterned(width, 2);
        publish(&mut sim);
        assert_eq!(
            bits(&sim.signal_table[id]),
            expected,
            "whole then partial, width={width}"
        );
    }
}

#[test]
fn masked_publication_keeps_forced_destinations_unchanged() {
    let mut sim = simulator();
    for &width in WIDTHS {
        let id = id(&sim, width);
        let forced = patterned(width, 2);
        sim.signal_table[id] = forced.clone();
        sim.forced_signals.insert(id, forced.clone());
        sim.queue_partial_nba(id, 0, patterned(width, 0));
        publish(&mut sim);
        assert_eq!(bits(&sim.signal_table[id]), bits(&forced), "width={width}");
        sim.forced_signals.remove(&id);
    }
}
