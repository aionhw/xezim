//! IEEE 1800-2023 §4.6 / §10.4.2: guarded control preserves four-state fallback.
use super::*;
use crate::compiler::bytecode::{CompiledBlock, Insn, lower_two_state};

fn block(insns: Vec<TsInsn>) -> TwoStateBlock {
    TwoStateBlock {
        insns,
        num_regs: 8,
        reads_whole: Box::new([]),
        reads_slice: Box::new([]),
        has_ctrl: true,
        has_wide: false,
        wide_words: 0,
        has_wait: false,
        tsx: false,
        vm_to_ts: Box::new([]),
        wait_regs: Box::new([]),
        reads_wide: Box::new([]),
        writes: Box::new([]),
        writes_span: Box::new([]),
    }
}

fn bits(value: &Value) -> Vec<LogicBit> {
    (0..value.width as usize)
        .map(|i| value.get_bit(i))
        .collect()
}

fn final_values(signals: &[Value], writes: &[NbaFast]) -> Vec<Vec<LogicBit>> {
    let mut values = signals.to_vec();
    for entry in writes {
        if let Some(mask) = &entry.mask {
            for bit in 0..mask.width.min(values[entry.signal_id].width) as usize {
                if mask.get_bit(bit) == LogicBit::One {
                    values[entry.signal_id].set_bit(bit, entry.value.get_bit(bit));
                }
            }
        } else {
            values[entry.signal_id] = entry.value.clone();
        }
    }
    values.iter().map(bits).collect()
}

fn differential(insns: Vec<Insn>, signals: &[Value], expected_fast: bool) -> Vec<Vec<LogicBit>> {
    differential_options(insns, signals, expected_fast, false, false)
}

fn differential_options(
    insns: Vec<Insn>,
    signals: &[Value],
    expected_fast: bool,
    replication: bool,
    wide_range: bool,
) -> Vec<Vec<LogicBit>> {
    let widths: Vec<_> = signals.iter().map(|v| v.width).collect();
    let signed: Vec<_> = signals.iter().map(|v| v.is_signed).collect();
    let real = vec![false; signals.len()];
    let arrays = HashMap::default();
    let cb = CompiledBlock {
        instructions: insns,
        num_regs: 8,
        has_fallback: false,
        nba_dup_targets: true,
        foreach_slots: 0,
        static_locals: Box::new([]),
    };
    let ts = lower_two_state(&cb, &widths, &signed, &real, &arrays).expect("narrow control lowers");
    let program = WordProgram::lower_with_options(ts, replication, wide_range)
        .expect("requested guarded operations are admitted");
    let mut words = Vec::new();
    let mut output = vec![NbaFast {
        signal_id: 2,
        value: Value::from_u64(0xde, 32),
        block_index: 99,
        mask: None,
    }];
    let result = program.execute(signals, &signed, &mut words, &mut output, 7, true);
    assert_eq!(result, expected_fast);
    assert_eq!(output[0].value.to_u64(), Some(0xde));
    assert_eq!(output[0].block_index, 99);
    let mut regs = vec![Value::zero(1); 8];
    let generic = Simulator::exec_insns_isolated(
        &cb.instructions,
        signals,
        &signed,
        &NameMap::default(),
        &arrays,
        None,
        &mut regs,
        7,
        true,
    );
    if result {
        assert_eq!(
            final_values(signals, &output[1..]),
            final_values(signals, &generic)
        );
        final_values(signals, &output[1..])
    } else {
        assert_eq!(
            output.len(),
            1,
            "late fallback must discard only this block's writes"
        );
        final_values(signals, &generic)
    }
}

fn reset_enable() -> Vec<Insn> {
    vec![
        Insn::BranchIfSignalFalse(0, 4, u32::MAX),
        Insn::LoadConst(0, Box::new(Value::from_u64(0x11, 32))),
        Insn::NbaAssign(2, 0, 32),
        Insn::Jump(10),
        Insn::BranchIfSignalFalse(1, 10, u32::MAX),
        Insn::LoadConst(1, Box::new(Value::from_u64(0x22, 32))),
        Insn::NbaAssign(2, 1, 32),
        Insn::LoadSignal(1, 3),
        Insn::NbaAssign(2, 1, 32),
        Insn::NbaAssign(4, 1, 32),
    ]
}

#[test]
fn nested_reset_enable_matches_four_state_execution_and_last_nba_wins() {
    for reset in [0, 1] {
        for enable in [0, 1] {
            let signals = [
                Value::from_u64(reset, 1),
                Value::from_u64(enable, 1),
                Value::from_u64(0x84, 32),
                Value::from_u64(0x73, 32),
                Value::zero(32),
            ];
            let result = differential(reset_enable(), &signals, true);
            let expected = if reset != 0 {
                0x11
            } else if enable != 0 {
                0x73
            } else {
                0x84
            };
            assert_eq!(result[2], bits(&Value::from_u64(expected, 32)));
            assert_eq!(
                result[4],
                bits(&Value::from_u64(
                    if reset == 0 && enable != 0 { 0x73 } else { 0 },
                    32
                ))
            );
        }
    }
}

#[test]
fn skipped_unknown_reads_stay_fast_and_late_unknowns_roll_back_prior_nbas() {
    for unknown in [LogicBit::X, LogicBit::Z] {
        for (reset, enable, fast) in [(0, 0, true), (1, 1, true), (0, 1, false)] {
            let mut data = Value::zero(32);
            data.set_bit(17, unknown);
            let signals = [
                Value::from_u64(reset, 1),
                Value::from_u64(enable, 1),
                Value::from_u64(0x84, 32),
                data,
                Value::zero(32),
            ];
            differential(reset_enable(), &signals, fast);
        }
        for condition in [0, 1] {
            let mut signals = vec![
                Value::zero(1),
                Value::from_u64(1, 1),
                Value::from_u64(0x84, 32),
                Value::zero(32),
                Value::zero(32),
            ];
            signals[condition].set_bit(0, unknown);
            differential(reset_enable(), &signals, false);
        }
    }
}

#[test]
fn wide_signal_bit_conditions_ignore_unselected_unknowns_and_guard_selected_unknowns() {
    for bit in [64, 127] {
        for selected in [LogicBit::Zero, LogicBit::One, LogicBit::X, LogicBit::Z] {
            let mut source = Value::zero(128);
            source.set_bit(3, LogicBit::X);
            source.set_bit(bit, selected);
            let signals = [source, Value::zero(1), Value::from_u64(0x84, 32)];
            let insns = vec![
                Insn::BranchIfSignalFalse(0, 3, bit as u32),
                Insn::LoadConst(0, Box::new(Value::from_u64(0x42, 32))),
                Insn::NbaAssign(2, 0, 32),
            ];
            let result = differential(
                insns,
                &signals,
                matches!(selected, LogicBit::Zero | LogicBit::One),
            );
            assert_eq!(
                result[2],
                bits(&Value::from_u64(
                    if selected == LogicBit::One {
                        0x42
                    } else {
                        0x84
                    },
                    32
                ))
            );
        }
    }
}

#[test]
fn fused_branches_preserve_aliasing_scratch_writes_and_lazy_loads() {
    for value in [0, 1, 5] {
        let signals = [Value::from_u64(value, 32), Value::new(32), Value::zero(32)];
        for insns in [
            vec![
                TsInsn::Const { d: 0, v: value },
                TsInsn::EqCBrFalse {
                    d: 0,
                    s: 0,
                    k: 1,
                    t: 3,
                },
                TsInsn::Const { d: 1, v: 9 },
            ],
            vec![
                TsInsn::Const { d: 0, v: value },
                TsInsn::Const { d: 1, v: 1 },
                TsInsn::EqBrFalse {
                    d: 0,
                    a: 0,
                    b: 1,
                    t: 4,
                },
                TsInsn::Const { d: 2, v: 9 },
            ],
        ] {
            let marker = if insns.len() == 3 { 1 } else { 2 };
            let program = WordProgram::lower(block(insns)).unwrap();
            let mut regs = vec![0; 8];
            let mut out = Vec::new();
            assert!(program.execute(&signals, &[false; 3], &mut regs, &mut out, 0, false));
            assert_eq!(regs[0], u64::from(value == 1));
            assert_eq!(regs[marker], if value == 1 { 9 } else { 0 });
        }
        let program = WordProgram::lower(block(vec![
            TsInsn::LoadSigBrNz {
                dl: 0,
                sig: 0,
                t: 3,
            },
            TsInsn::Const { d: 1, v: 7 },
            TsInsn::Jmp { t: 4 },
            TsInsn::Const { d: 1, v: 9 },
        ]))
        .unwrap();
        let mut regs = vec![0; 8];
        let mut out = Vec::new();
        assert!(program.execute(&signals, &[false; 3], &mut regs, &mut out, 0, false));
        assert_eq!(regs[0], value);
        assert_eq!(regs[1], if value != 0 { 9 } else { 7 });
        let program = WordProgram::lower(block(vec![
            TsInsn::Const { d: 0, v: value },
            TsInsn::BrFalseLoadSig {
                s: 0,
                t: 3,
                dl: 1,
                sig: 1,
            },
            TsInsn::Const { d: 2, v: 9 },
        ]))
        .unwrap();
        assert_eq!(
            program.execute(&signals, &[false; 3], &mut regs, &mut out, 0, false),
            value == 0
        );
        for branch in [TsInsn::BrFalse { s: 0, t: 3 }, TsInsn::BrNz { s: 0, t: 3 }] {
            let takes = match branch {
                TsInsn::BrFalse { .. } => value == 0,
                _ => value != 0,
            };
            let program = WordProgram::lower(block(vec![
                TsInsn::Const { d: 0, v: value },
                branch,
                TsInsn::Const { d: 1, v: 9 },
            ]))
            .unwrap();
            regs.fill(0);
            assert!(program.execute(&signals, &[false; 3], &mut regs, &mut out, 0, false));
            assert_eq!(regs[1], if takes { 0 } else { 9 });
        }
    }
}

#[test]
fn unsupported_control_backedges_and_invalid_targets_remain_ineligible() {
    for insn in [
        TsInsn::Jmp { t: 0 },
        TsInsn::Jmp { t: 2 },
        TsInsn::JmpPoll { t: 0 },
        TsInsn::BrFalse { s: 0, t: 0 },
        TsInsn::BrNz { s: 0, t: 2 },
        TsInsn::BrSigFalse {
            sig: 0,
            bit: u32::MAX,
            t: 0,
        },
        TsInsn::LoadSigBrNz {
            dl: 0,
            sig: 0,
            t: 0,
        },
        TsInsn::BrFalseLoadSig {
            s: 0,
            t: 2,
            dl: 1,
            sig: 0,
        },
        TsInsn::EqBrFalse {
            d: 0,
            a: 0,
            b: 0,
            t: 0,
        },
        TsInsn::EqCBrFalse {
            d: 0,
            s: 0,
            k: 0,
            t: 2,
        },
        TsInsn::CovHit(0),
        TsInsn::LoopPoll,
    ] {
        assert!(WordProgram::lower(block(vec![insn])).is_none());
    }
    let mut wide = block(vec![]);
    wide.has_wide = true;
    assert!(WordProgram::lower(wide).is_none());
    let mut waiting = block(vec![]);
    waiting.has_wait = true;
    assert!(WordProgram::lower(waiting).is_none());
}

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

#[test]
fn wide_slices_and_partial_nbas_match_independent_selected_bit_updates() {
    for (lo, width) in [(63, 1), (64, 1), (127, 1), (60, 8), (63, 64)] {
        for selected_unknown in [None, Some(LogicBit::X), Some(LogicBit::Z)] {
            let mut source = Value::zero(257);
            for bit in 0..257 {
                source.set_bit(
                    bit,
                    if bit % 3 == 1 {
                        LogicBit::One
                    } else {
                        LogicBit::Zero
                    },
                );
            }
            source.set_bit(200, LogicBit::X);
            source.set_bit(201, LogicBit::Z);
            if let Some(bit) = selected_unknown {
                source.set_bit(lo + width - 1, bit);
            }
            let target = patterned(129, 1);
            let signals = [
                source,
                Value::zero(1),
                target.clone(),
                Value::zero(1),
                Value::zero(32),
            ];
            let load = if width == 1 {
                Insn::LoadSignalBit(0, 0, lo as u32)
            } else {
                Insn::LoadSignalRange(0, 0, (lo + width - 1) as u32, lo as u32)
            };
            let insns = vec![
                Insn::LoadConst(1, Box::new(Value::from_u64(0x21, 32))),
                Insn::NbaAssign(4, 1, 32),
                load,
                Insn::NbaAssignRange(2, (64 + width - 1) as u32, 64, 0),
            ];
            let result = differential(insns, &signals, selected_unknown.is_none());
            let mut expected = bits(&target);
            for bit in 0..width {
                expected[64 + bit] = signals[0].get_bit(lo + bit);
            }
            assert_eq!(result[2], expected, "source {lo}+:{width}");
            assert_eq!(result[4], bits(&Value::from_u64(0x21, 32)));
        }
    }
}

#[test]
fn partial_masks_keep_equal_selected_bits_and_preserve_other_block_outputs_and_live_bits() {
    for width in [64, 65, 128, 129, 257] {
        let lo = if width == 64 { 60 } else { 61 };
        let count = (width - lo).min(8);
        let mask = u64::MAX >> (64 - count);
        let mut current = patterned(width, 0);
        let sampled = Value::from_u64(0x5a & mask, count);
        for bit in 0..count {
            current.set_bit((lo + bit) as usize, sampled.get_bit(bit as usize));
        }
        let signals = [Value::zero(1), Value::zero(1), current];
        let mut out = vec![NbaFast {
            signal_id: 2,
            value: patterned(width, 2),
            mask: None,
            block_index: 99,
        }];
        let original = bits(&out[0].value);
        let program = WordProgram::lower(block(vec![
            TsInsn::Const { d: 0, v: 0x5a },
            TsInsn::RangeStoreNbaW {
                sig: 2,
                hi: lo + count - 1,
                lo,
                s: 0,
                mask,
            },
        ]))
        .unwrap();
        assert!(program.execute(&signals, &[false; 3], &mut Vec::new(), &mut out, 7, false));
        assert_eq!(out.len(), 2);
        assert_eq!(bits(&out[0].value), original);
        assert_eq!(out[0].block_index, 99);
        let selection = out[1]
            .mask
            .as_ref()
            .expect("an equal sampled value still owns selected bits");
        for bit in 0..width {
            assert_eq!(
                selection.get_bit(bit as usize),
                if (lo..lo + count).contains(&bit) {
                    LogicBit::One
                } else {
                    LogicBit::Zero
                }
            );
        }
        let mut live = signals.to_vec();
        live[2] = patterned(width, 3);
        let mut expected = bits(&live[2]);
        for bit in 0..count {
            expected[(lo + bit) as usize] = sampled.get_bit(bit as usize);
        }
        assert_eq!(final_values(&live, &out[1..])[2], expected);
    }
}

#[test]
fn whole_and_overlapping_partial_updates_follow_source_order_within_each_block() {
    for whole_first in [false, true] {
        let whole = TsInsn::ConstStoreNba {
            sig: 2,
            v: 0x76543210abcdef98,
            w: 64,
        };
        let mut insns = vec![TsInsn::Const { d: 0, v: 0x5a }];
        if whole_first {
            insns.push(whole);
        }
        insns.push(TsInsn::RangeStoreNba {
            sig: 2,
            hi: 15,
            lo: 8,
            s: 0,
            mask: 0xff,
        });
        insns.push(TsInsn::RangeStoreNba {
            sig: 2,
            hi: 19,
            lo: 12,
            s: 0,
            mask: 0xff,
        });
        if !whole_first {
            insns.push(TsInsn::ConstStoreNba {
                sig: 2,
                v: 0x76543210abcdef98,
                w: 64,
            });
        }
        let program = WordProgram::lower(block(insns)).unwrap();
        let signals = [Value::zero(1), Value::zero(1), patterned(64, 0)];
        let mut out = vec![NbaFast {
            signal_id: 2,
            value: patterned(64, 3),
            mask: None,
            block_index: 99,
        }];
        let prefix = bits(&out[0].value);
        assert!(program.execute(&signals, &[false; 3], &mut Vec::new(), &mut out, 7, true));
        assert_eq!(out.len(), 2);
        assert_eq!(bits(&out[0].value), prefix);
        assert!(out[1].mask.is_none(), "whole write owns every bit");
        let mut expected = bits(&Value::from_u64(0x76543210abcdef98, 64));
        if whole_first {
            for lo in [8, 12] {
                for bit in 0..8 {
                    expected[lo + bit] = if 0x5a & (1 << bit) != 0 {
                        LogicBit::One
                    } else {
                        LogicBit::Zero
                    };
                }
            }
        }
        assert_eq!(final_values(&signals, &out[1..])[2], expected);
    }
}

#[test]
fn wide_fill_union_and_late_unknowns_preserve_masked_publication_and_rollback() {
    let signals = [Value::new(128), Value::zero(1), patterned(257, 0)];
    for fill in [0, 1] {
        let program = WordProgram::lower(block(vec![
            TsInsn::RangeFillNbaW {
                sig: 2,
                hi: 200,
                lo: 63,
                bit: fill,
            },
            TsInsn::Const { d: 0, v: 0x5a },
            TsInsn::RangeStoreNbaW {
                sig: 2,
                hi: 70,
                lo: 63,
                s: 0,
                mask: 0xff,
            },
        ]))
        .unwrap();
        let mut out = Vec::new();
        assert!(program.execute(&signals, &[false; 3], &mut Vec::new(), &mut out, 7, true));
        assert_eq!(out.len(), 1);
        let mut expected = bits(&signals[2]);
        for bit in 63..=200 {
            expected[bit] = if fill == 0 {
                LogicBit::Zero
            } else {
                LogicBit::One
            };
        }
        for bit in 0..8 {
            expected[63 + bit] = if 0x5a & (1 << bit) != 0 {
                LogicBit::One
            } else {
                LogicBit::Zero
            };
        }
        assert_eq!(final_values(&signals, &out)[2], expected);
        let program = WordProgram::lower(block(vec![
            TsInsn::RangeFillNbaW {
                sig: 2,
                hi: 200,
                lo: 63,
                bit: fill,
            },
            TsInsn::SigBitW {
                d: 0,
                sig: 0,
                bit: 64,
            },
        ]))
        .unwrap();
        let mut prefix = vec![NbaFast {
            signal_id: 2,
            value: patterned(257, 3),
            mask: None,
            block_index: 99,
        }];
        let before = bits(&prefix[0].value);
        assert!(!program.execute(&signals, &[false; 3], &mut Vec::new(), &mut prefix, 7, true));
        assert_eq!(prefix.len(), 1);
        assert_eq!(bits(&prefix[0].value), before);
    }
}

#[test]
fn unsupported_wide_shapes_reject_and_runtime_slice_bounds_fall_back_atomically() {
    for insn in [
        TsInsn::SigRangeW {
            d: 0,
            sig: 0,
            lo: 0,
            w: 65,
            mask: u64::MAX,
        },
        TsInsn::SigRangeW {
            d: 0,
            sig: 0,
            lo: 0,
            w: 0,
            mask: 0,
        },
        TsInsn::RangeStoreNbaW {
            sig: 2,
            hi: 64,
            lo: 0,
            s: 0,
            mask: u64::MAX,
        },
        TsInsn::RangeStoreNba {
            sig: 2,
            hi: 0,
            lo: 1,
            s: 0,
            mask: 1,
        },
        TsInsn::RangeFillNbaW {
            sig: 2,
            hi: 3,
            lo: 1,
            bit: 2,
        },
    ] {
        assert!(WordProgram::lower(block(vec![insn])).is_none());
    }
    let signals = [Value::zero(128), Value::zero(1), Value::zero(128)];
    for last in [
        TsInsn::SigRangeW {
            d: 0,
            sig: 0,
            lo: 120,
            w: 16,
            mask: 0xffff,
        },
        TsInsn::RangeStoreNbaW {
            sig: 2,
            hi: 128,
            lo: 127,
            s: 0,
            mask: 3,
        },
    ] {
        let program = WordProgram::lower(block(vec![
            TsInsn::Const { d: 0, v: 1 },
            TsInsn::RangeStoreNbaW {
                sig: 2,
                hi: 64,
                lo: 64,
                s: 0,
                mask: 1,
            },
            last,
        ]))
        .unwrap();
        let mut out = Vec::new();
        assert!(!program.execute(&signals, &[false; 3], &mut Vec::new(), &mut out, 7, false));
        assert!(out.is_empty());
    }
}

fn wide_block(insns: Vec<TsInsn>, words: u8) -> TwoStateBlock {
    let mut ts = block(insns);
    ts.has_wide = true;
    ts.wide_words = words;
    ts
}

fn known_wide(width: u32, phase: usize) -> Value {
    let mut value = Value::zero(width);
    for bit in 0..width as usize {
        if (bit + phase) % 5 == 1 || (bit + phase) % 5 == 3 {
            value.set_bit(bit, LogicBit::One);
        }
    }
    value
}

#[test]
fn wide_whole_copies_and_constants_match_every_bit_across_supported_widths() {
    for width in [65, 72, 96, 128, 129, 256, 512] {
        let source = known_wide(width, 0);
        let signals = [source.clone(), Value::zero(1), patterned(width, 1)];
        let copied = differential(
            vec![Insn::LoadSignal(0, 0), Insn::NbaAssign(2, 0, width)],
            &signals,
            true,
        );
        assert_eq!(copied[2], bits(&source), "copy width={width}");
        let constant = known_wide(width, 3);
        let copied = differential(
            vec![
                Insn::LoadConst(0, Box::new(constant.clone())),
                Insn::NbaAssign(2, 0, width),
            ],
            &signals,
            true,
        );
        assert_eq!(copied[2], bits(&constant), "constant width={width}");
    }
}

#[test]
fn wide_partial_copies_overlap_in_order_and_mix_with_whole_writes() {
    let source = known_wide(96, 1);
    let whole = known_wide(256, 4);
    let signals = [source.clone(), Value::zero(1), patterned(256, 0)];
    for position in [0, 1, 2] {
        let mut insns = Vec::new();
        if position == 1 {
            insns.extend([
                Insn::LoadConst(1, Box::new(whole.clone())),
                Insn::NbaAssign(2, 1, 256),
            ]);
        }
        insns.extend([
            Insn::LoadSignal(0, 0),
            Insn::NbaAssignRange(2, 158, 63, 0),
            Insn::NbaAssignRange(2, 222, 127, 0),
        ]);
        if position == 2 {
            insns.extend([
                Insn::LoadConst(1, Box::new(whole.clone())),
                Insn::NbaAssign(2, 1, 256),
            ]);
        }
        let result = differential(insns, &signals, true);
        let mut expected = if position == 0 {
            bits(&signals[2])
        } else {
            bits(&whole)
        };
        if position != 2 {
            for lo in [63, 127] {
                for bit in 0..96 {
                    expected[lo + bit] = source.get_bit(bit);
                }
            }
        }
        assert_eq!(result[2], expected, "whole position={position}");
    }
}

#[test]
fn disabled_wide_unknowns_stay_lazy_and_executed_unknowns_roll_back_partial_outputs() {
    for source in [Value::new(129), Value::all_z(129)] {
        for enabled in [false, true] {
            let signals = [
                source.clone(),
                Value::from_u64(u64::from(enabled), 1),
                patterned(257, 0),
                Value::zero(129),
                Value::zero(32),
            ];
            let constant = known_wide(129, 1);
            let result = differential(
                vec![
                    Insn::LoadConst(0, Box::new(Value::from_u64(7, 32))),
                    Insn::NbaAssign(4, 0, 32),
                    Insn::BranchIfSignalFalse(1, 8, u32::MAX),
                    Insn::LoadConst(1, Box::new(constant.clone())),
                    Insn::NbaAssignRange(2, 192, 64, 1),
                    Insn::LoadSignal(2, 0),
                    Insn::NbaAssignRange(2, 255, 127, 2),
                    Insn::NbaAssign(3, 2, 129),
                ],
                &signals,
                !enabled,
            );
            let mut expected = bits(&signals[2]);
            if enabled {
                for bit in 0..129 {
                    expected[64 + bit] = constant.get_bit(bit);
                }
                for bit in 0..129 {
                    expected[127 + bit] = source.get_bit(bit);
                }
            }
            assert_eq!(result[2], expected);
            assert_eq!(result[4], bits(&Value::from_u64(7, 32)));
        }
    }
}

#[test]
fn wide_masks_and_bank_moves_truncate_and_zero_extend_without_touching_narrow_regs() {
    for width in [65u16, 72, 96, 128, 129, 256, 512] {
        let program = WordProgram::lower(wide_block(
            vec![
                TsInsn::Const { d: 1, v: 0x1234 },
                TsInsn::WConst {
                    d: 0,
                    v: vec![u64::MAX; 8].into_boxed_slice(),
                },
                TsInsn::WMask { d: 0, w: width },
                TsInsn::WStoreNba {
                    sig: 2,
                    s: 0,
                    w: 512,
                },
                TsInsn::NFromW { r: 0, mask: 0xff },
                TsInsn::WFromN { r: 0 },
                TsInsn::WStoreNba {
                    sig: 3,
                    s: 0,
                    w: 512,
                },
            ],
            8,
        ))
        .unwrap();
        let signals = [
            Value::zero(1),
            Value::zero(1),
            Value::zero(512),
            Value::zero(512),
        ];
        let mut regs = Vec::new();
        let mut out = Vec::new();
        assert!(program.execute(
            &signals,
            &[false, false, true, false],
            &mut regs,
            &mut out,
            7,
            true
        ));
        assert_eq!(regs[1], 0x1234);
        assert!(out[0].value.is_signed);
        assert!(!out[1].value.is_signed);
        let result = final_values(&signals, &out);
        for bit in 0..512 {
            assert_eq!(
                result[2][bit],
                if bit < width as usize {
                    LogicBit::One
                } else {
                    LogicBit::Zero
                }
            );
            assert_eq!(
                result[3][bit],
                if bit < 8 {
                    LogicBit::One
                } else {
                    LogicBit::Zero
                }
            );
        }
    }
}

#[test]
fn retained_wide_bank_capacity_survives_plan_width_changes_and_runtime_bounds() {
    let mut regs = Vec::new();
    for capacity in [8u8, 1, 4, 2, 7, 3, 6, 5] {
        let width = u32::from(capacity) * 64;
        let mut ts = wide_block(
            vec![
                TsInsn::Const { d: 0, v: 0x78 },
                TsInsn::WConst {
                    d: 1,
                    v: vec![0xfeed].into_boxed_slice(),
                },
                TsInsn::WStoreNba {
                    sig: 2,
                    s: 1,
                    w: width,
                },
            ],
            capacity,
        );
        ts.num_regs = 2;
        let program = WordProgram::lower(ts).unwrap();
        let signals = [Value::zero(1), Value::zero(1), Value::zero(width)];
        let mut out = Vec::new();
        assert!(program.execute(&signals, &[false; 3], &mut regs, &mut out, 7, false));
        assert_eq!(regs[0], 0x78);
        assert!(regs.len() >= 2 * (1 + capacity as usize));
        assert_eq!(bits(&out[0].value), bits(&Value::from_u64(0xfeed, width)));
    }
    let signals = [Value::zero(129), Value::zero(1), Value::zero(128)];
    for final_op in [
        TsInsn::WLoadSig { d: 0, sig: 0 },
        TsInsn::WStoreNba {
            sig: 2,
            s: 0,
            w: 65,
        },
        TsInsn::WRangeStoreNba {
            sig: 2,
            lo: u32::MAX,
            s: 0,
            w: 65,
        },
    ] {
        let program = WordProgram::lower(wide_block(
            vec![
                TsInsn::ConstStoreNba {
                    sig: 2,
                    v: 3,
                    w: 128,
                },
                final_op,
            ],
            2,
        ))
        .unwrap();
        let mut out = Vec::new();
        assert!(!program.execute(&signals, &[false; 3], &mut regs, &mut out, 7, true));
        assert!(out.is_empty());
    }
}

#[test]
fn malformed_wide_capacity_arithmetic_and_unknown_constants_remain_ineligible() {
    for capacity in [0, 9] {
        assert!(
            WordProgram::lower(wide_block(
                vec![TsInsn::WLoadSig { d: 0, sig: 0 }],
                capacity
            ))
            .is_none()
        );
    }
    for insn in [
        TsInsn::WConst {
            d: 0,
            v: Box::new([]),
        },
        TsInsn::WConst {
            d: 0,
            v: vec![0; 3].into_boxed_slice(),
        },
        TsInsn::WLoadSig { d: 8, sig: 0 },
        TsInsn::WStoreNba {
            sig: 2,
            s: 0,
            w: 129,
        },
        TsInsn::WRangeStoreNba {
            sig: 2,
            lo: 0,
            s: 0,
            w: 0,
        },
        TsInsn::WMask { d: 0, w: 129 },
        TsInsn::WXor { d: 0, a: 0, b: 1 },
    ] {
        assert!(WordProgram::lower(wide_block(vec![insn], 2)).is_none());
    }
    assert!(WordProgram::lower(block(vec![TsInsn::WLoadSig { d: 0, sig: 0 }])).is_none());
    let mut too_many = wide_block(vec![TsInsn::WLoadSig { d: 0, sig: 0 }], 2);
    too_many.num_regs = u32::MAX;
    assert!(WordProgram::lower(too_many).is_none());
    for unknown in [Value::new(129), Value::all_z(129)] {
        let cb = CompiledBlock {
            instructions: vec![
                Insn::LoadConst(0, Box::new(unknown)),
                Insn::NbaAssign(2, 0, 129),
            ],
            num_regs: 8,
            has_fallback: false,
            nba_dup_targets: false,
            foreach_slots: 0,
            static_locals: Box::new([]),
        };
        assert!(
            lower_two_state(
                &cb,
                &[129, 1, 129],
                &[false; 3],
                &[false; 3],
                &HashMap::default()
            )
            .is_none()
        );
    }
}

fn replicated_bits(part: u64, width: u8, count: u16) -> Vec<LogicBit> {
    (0..usize::from(width) * usize::from(count))
        .map(|bit| {
            if (part >> (bit % usize::from(width))) & 1 == 0 {
                LogicBit::Zero
            } else {
                LogicBit::One
            }
        })
        .collect()
}

#[test]
fn scalar_replication_masks_raw_parts_handles_aliases_and_preserves_fused_scratch() {
    for (width, count) in [
        (1, 1),
        (1, 63),
        (1, 64),
        (3, 21),
        (7, 9),
        (8, 8),
        (16, 4),
        (31, 2),
        (32, 2),
        (63, 1),
        (64, 1),
    ] {
        let total = u32::from(width) * u32::from(count);
        for alias in [false, true] {
            let source = 0xfedc_ba98_7654_321b;
            let dest = if alias { 0 } else { 1 };
            let program = WordProgram::lower_with_options(
                block(vec![
                    TsInsn::Const { d: 0, v: source },
                    TsInsn::Repl {
                        d: dest,
                        s: 0,
                        w: width,
                        count,
                    },
                    TsInsn::StoreNba {
                        sig: 2,
                        s: dest,
                        w: total,
                        mask: u64::MAX >> (64 - total),
                    },
                ]),
                true,
                false,
            )
            .unwrap();
            let signals = [Value::zero(1), Value::zero(1), Value::zero(total)];
            let mut output = Vec::new();
            assert!(program.execute(
                &signals,
                &[false, false, true],
                &mut Vec::new(),
                &mut output,
                7,
                true
            ));
            assert_eq!(
                bits(&output[0].value),
                replicated_bits(source, width, u16::from(count))
            );
            assert!(output[0].value.is_signed);
            let mut input = Value::from_u64(source, u32::from(width));
            input.is_signed = true;
            let signals = [
                input,
                Value::zero(1),
                Value::zero(total),
                Value::zero(u32::from(width)),
            ];
            let mut insns = vec![
                TsInsn::LoadSigRepl {
                    dl: 0,
                    sig: 0,
                    d: dest,
                    w: width,
                    count,
                },
                TsInsn::StoreNba {
                    sig: 2,
                    s: dest,
                    w: total,
                    mask: u64::MAX >> (64 - total),
                },
            ];
            if !alias {
                insns.push(TsInsn::StoreNba {
                    sig: 3,
                    s: 0,
                    w: u32::from(width),
                    mask: u64::MAX >> (64 - u32::from(width)),
                });
            }
            let program = WordProgram::lower_with_options(block(insns), true, false).unwrap();
            let mut output = Vec::new();
            assert!(program.execute(
                &signals,
                &[true, false, false, false],
                &mut Vec::new(),
                &mut output,
                7,
                true
            ));
            assert_eq!(
                bits(&output[0].value),
                replicated_bits(source, width, u16::from(count))
            );
            if !alias {
                assert_eq!(bits(&output[1].value), bits(&signals[0]));
            }
        }
    }
}

#[test]
fn bounded_wide_replication_matches_independent_bits_and_generic_execution() {
    let mut scratch = vec![u64::MAX; 8 * 9];
    for (width, count) in [
        (1, 65),
        (8, 9),
        (3, 32),
        (1, 127),
        (64, 2),
        (3, 43),
        (1, 257),
        (7, 73),
        (64, 8),
        (63, 8),
        (31, 13),
    ] {
        let total = u32::from(width) * u32::from(count);
        for part in [0, 1, 0xfedc_ba98_7654_321b, u64::MAX] {
            for alias in [false, true] {
                let dest = if alias { 0 } else { 1 };
                let program = WordProgram::lower_with_options(
                    wide_block(
                        vec![
                            TsInsn::Const { d: 0, v: part },
                            TsInsn::WRepl {
                                d: dest,
                                s: 0,
                                w: width,
                                count,
                            },
                            TsInsn::WStoreNba {
                                sig: 2,
                                s: dest,
                                w: total,
                            },
                        ],
                        8,
                    ),
                    true,
                    false,
                )
                .unwrap();
                let signals = [Value::zero(1), Value::zero(1), Value::zero(total)];
                let mut output = Vec::new();
                assert!(program.execute(&signals, &[false; 3], &mut scratch, &mut output, 7, true));
                assert_eq!(bits(&output[0].value), replicated_bits(part, width, count));
                let offset = 8 + usize::from(dest) * 8;
                let used = total.div_ceil(64) as usize;
                assert!(
                    scratch[offset + used..offset + 8]
                        .iter()
                        .all(|&word| word == 0)
                );
                if total % 64 != 0 {
                    assert_eq!(scratch[offset + used - 1] >> (total % 64), 0);
                }
                assert_eq!(
                    scratch[0], part,
                    "wide writes preserve narrow source even when aliased"
                );
            }
            let signals = [
                Value::from_u64(part, u32::from(width)),
                Value::zero(1),
                Value::zero(total),
            ];
            let observed = differential_options(
                vec![
                    Insn::LoadSignal(0, 0),
                    Insn::Replicate(0, 0, u32::from(count)),
                    Insn::NbaAssign(2, 0, total),
                ],
                &signals,
                true,
                true,
                false,
            );
            assert_eq!(observed[2], replicated_bits(part, width, count));
        }
    }
}

#[test]
fn wide_signal_ranges_match_selected_bits_and_ignore_unknowns_outside_selection() {
    for (lo, width) in [
        (0, 65),
        (1, 127),
        (63, 128),
        (64, 129),
        (127, 257),
        (129, 511),
        (63, 512),
    ] {
        for unknown in [None, Some(LogicBit::X), Some(LogicBit::Z)] {
            let mut source = known_wide(1024, 2);
            source.is_signed = true;
            source.set_bit(1022, LogicBit::X);
            source.set_bit(1023, LogicBit::Z);
            if let Some(bit) = unknown {
                source.set_bit((lo + width - 1) as usize, bit);
            }
            let target = patterned(1024, 3);
            let signals = [source, Value::zero(1), target.clone(), Value::zero(32)];
            let observed = differential_options(
                vec![
                    Insn::LoadConst(1, Box::new(Value::from_u64(19, 32))),
                    Insn::NbaAssign(3, 1, 32),
                    Insn::LoadSignalRange(0, 0, lo + width - 1, lo),
                    Insn::NbaAssignRange(2, width + 62, 63, 0),
                ],
                &signals,
                unknown.is_none(),
                false,
                true,
            );
            let mut expected = bits(&target);
            for bit in 0..width as usize {
                expected[63 + bit] = signals[0].get_bit(lo as usize + bit);
            }
            assert_eq!(observed[2], expected, "slice {lo}+:{width}");
            assert_eq!(observed[3], bits(&Value::from_u64(19, 32)));
        }
    }
}

#[test]
fn skipped_replication_and_wide_slice_loads_are_lazy_and_late_unknowns_roll_back_partials() {
    for wide in [false, true] {
        for unknown in [LogicBit::X, LogicBit::Z] {
            for execute in [false, true] {
                let target_width = if wide { 257 } else { 63 };
                let mut source = known_wide(if wide { 257 } else { 3 }, 1);
                source.set_bit(if wide { 127 } else { 1 }, unknown);
                let signals = [
                    source,
                    Value::from_u64(u64::from(execute), 1),
                    patterned(target_width, 2),
                ];
                let load = if wide {
                    TsInsn::WSigRange {
                        d: 2,
                        sig: 0,
                        lo: 63,
                        w: 129,
                    }
                } else {
                    TsInsn::LoadSigRepl {
                        dl: 0,
                        sig: 0,
                        d: 2,
                        w: 3,
                        count: 21,
                    }
                };
                let store = if wide {
                    TsInsn::WRangeStoreNba {
                        sig: 2,
                        lo: 64,
                        s: 2,
                        w: 129,
                    }
                } else {
                    TsInsn::StoreNba {
                        sig: 2,
                        s: 2,
                        w: 63,
                        mask: u64::MAX >> 1,
                    }
                };
                let insns = vec![
                    TsInsn::Const { d: 0, v: 5 },
                    TsInsn::RangeStoreNba {
                        sig: 2,
                        hi: 4,
                        lo: 2,
                        s: 0,
                        mask: 7,
                    },
                    TsInsn::LoadSig { d: 1, sig: 1 },
                    TsInsn::BrFalse { s: 1, t: 6 },
                    load,
                    store,
                ];
                let ts = if wide {
                    wide_block(insns, 3)
                } else {
                    block(insns)
                };
                let program = WordProgram::lower_with_options(ts, true, true).unwrap();
                let prefix = patterned(target_width, 4);
                let mut output = vec![NbaFast {
                    signal_id: 2,
                    value: prefix.clone(),
                    block_index: 99,
                    mask: None,
                }];
                assert_eq!(
                    program.execute(&signals, &[false; 3], &mut Vec::new(), &mut output, 7, true),
                    !execute
                );
                assert_eq!(bits(&output[0].value), bits(&prefix));
                assert_eq!(output[0].block_index, 99);
                assert!(output[0].mask.is_none());
                if execute {
                    assert_eq!(output.len(), 1);
                } else {
                    let mut expected = bits(&prefix);
                    for bit in 0..3 {
                        expected[2 + bit] = if (5 >> bit) & 1 == 0 {
                            LogicBit::Zero
                        } else {
                            LogicBit::One
                        };
                    }
                    assert_eq!(final_values(&signals, &output)[2], expected);
                }
            }
        }
    }
}

#[test]
fn replication_and_wide_range_options_are_separate_and_reject_invalid_shapes() {
    for insn in [
        TsInsn::Repl {
            d: 0,
            s: 0,
            w: 1,
            count: 64,
        },
        TsInsn::LoadSigRepl {
            dl: 0,
            sig: 0,
            d: 1,
            w: 3,
            count: 21,
        },
        TsInsn::WRepl {
            d: 0,
            s: 0,
            w: 1,
            count: 129,
        },
    ] {
        assert!(WordProgram::lower_with_options(wide_block(vec![insn], 8), false, true).is_none());
    }
    assert!(
        WordProgram::lower_with_options(
            wide_block(
                vec![TsInsn::WSigRange {
                    d: 0,
                    sig: 0,
                    lo: 0,
                    w: 129
                }],
                8
            ),
            true,
            false
        )
        .is_none()
    );
    for insn in [
        TsInsn::Repl {
            d: 0,
            s: 0,
            w: 0,
            count: 3,
        },
        TsInsn::Repl {
            d: 0,
            s: 0,
            w: 65,
            count: 1,
        },
        TsInsn::Repl {
            d: 0,
            s: 0,
            w: 3,
            count: 0,
        },
        TsInsn::Repl {
            d: 0,
            s: 0,
            w: 33,
            count: 2,
        },
        TsInsn::LoadSigRepl {
            dl: 0,
            sig: 0,
            d: 1,
            w: 64,
            count: 2,
        },
        TsInsn::WRepl {
            d: 0,
            s: 0,
            w: 0,
            count: 128,
        },
        TsInsn::WRepl {
            d: 0,
            s: 0,
            w: 65,
            count: 2,
        },
        TsInsn::WRepl {
            d: 0,
            s: 0,
            w: 64,
            count: 9,
        },
        TsInsn::WRepl {
            d: 0,
            s: 0,
            w: 1,
            count: 0,
        },
        TsInsn::WRepl {
            d: 0,
            s: 0,
            w: 1,
            count: 64,
        },
        TsInsn::WSigRange {
            d: 0,
            sig: 0,
            lo: 0,
            w: 0,
        },
        TsInsn::WSigRange {
            d: 0,
            sig: 0,
            lo: 0,
            w: 513,
        },
        TsInsn::WSigRange {
            d: 0,
            sig: 0,
            lo: u16::MAX,
            w: 129,
        },
    ] {
        assert!(WordProgram::lower_with_options(wide_block(vec![insn], 8), true, true).is_none());
    }
    assert!(
        WordProgram::lower_with_options(
            wide_block(
                vec![TsInsn::WRepl {
                    d: 0,
                    s: 0,
                    w: 1,
                    count: 129
                }],
                2
            ),
            true,
            true
        )
        .is_none()
    );
    let program = WordProgram::lower_with_options(
        wide_block(
            vec![TsInsn::WSigRange {
                d: 0,
                sig: 0,
                lo: 64,
                w: 129,
            }],
            3,
        ),
        false,
        true,
    )
    .unwrap();
    let mut output = vec![NbaFast {
        signal_id: 0,
        value: Value::from_u64(9, 8),
        block_index: 99,
        mask: None,
    }];
    assert!(!program.execute(
        &[Value::zero(192)],
        &[false],
        &mut Vec::new(),
        &mut output,
        7,
        true
    ));
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].value.to_u64(), Some(9));
}

#[test]
fn replicated_and_selected_wide_partials_overlap_in_source_order() {
    let part = 5;
    let source = known_wide(257, 1);
    let target = patterned(257, 4);
    let signals = [
        Value::from_u64(part, 3),
        Value::zero(1),
        target.clone(),
        source,
    ];
    let observed = differential_options(
        vec![
            Insn::LoadSignal(0, 0),
            Insn::Replicate(0, 0, 43),
            Insn::NbaAssignRange(2, 192, 64, 0),
            Insn::LoadSignalRange(1, 3, 191, 63),
            Insn::NbaAssignRange(2, 189, 61, 1),
        ],
        &signals,
        true,
        true,
        true,
    );
    let mut expected = bits(&target);
    let repeated = replicated_bits(part, 3, 43);
    expected[64..193].copy_from_slice(&repeated);
    for bit in 0..129 {
        expected[61 + bit] = signals[3].get_bit(63 + bit);
    }
    assert_eq!(observed[2], expected);
}
