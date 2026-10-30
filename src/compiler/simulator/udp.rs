//! Simulator udp.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// `--primitive-verbose` terminal annotation: the net's current 4-state
    /// value at lowering time plus an IMPLICIT-NET marker. A UDP terminal
    /// landing on a §6.10 implicit 1-bit net is the classic signature of a
    /// dangling pin from a dropped/unresolved cell — flag it loudly.
    pub(super) fn udp_terminal_note(&self, sig_id: usize, bit: u32) -> String {
        let name = self.name_for_id(sig_id);
        let code = self
            .signal_table
            .get(sig_id)
            .map(|v| v.get_bit_code(bit as usize))
            .unwrap_or(2);
        let val = ['0', '1', 'x', 'z'][code as usize % 4];
        let implicit = if self.module.implicit_nets.contains(name) {
            " [IMPLICIT NET — undeclared identifier, check for a dropped/unresolved cell]"
        } else {
            ""
        };
        let width = self.signal_table.get(sig_id).map(|v| v.width).unwrap_or(1);
        if width > 1 {
            format!(" = {} (of {}-bit vector){}", val, width, implicit)
        } else {
            format!(" = {}{}", val, implicit)
        }
    }

    /// IEEE 1800-2017 §29: turn each flattened `UdpInstance` into a comb entry
    /// (`CombItem::Udp{idx}`) plus a `UdpRuntime` state slot. Terminal nets are
    /// resolved to single-bit refs; unresolved/multibit terminals emit a loud
    /// warning and the instance is dropped (output left undriven).
    pub(super) fn build_udp_entries(&mut self, entries: &mut Vec<CombEntry>) {
        let insts = std::mem::take(&mut self.module.udp_instances);
        let pv_total = insts.len();
        let mut pv_seq = 0usize;
        let mut pv_dropped = 0usize;
        let mut pv_const_terms = 0usize;
        let mut pv_implicit_terms = 0usize;
        let mut programs: HashMap<
            String,
            (
                Arc<[crate::ast::decl::UdpTableRow]>,
                Option<Arc<[u8]>>,
                usize,
            ),
        > = HashMap::default();
        let mut pv_lut = 0usize;
        let mut pv_event_filtered = 0usize;
        // Flattened module ports are represented as zero-delay copies. Follow
        // those aliases so UDPs clocked through separate wrapper-port nets can
        // share one upstream event dispatch, as net/functor simulators do.
        let mut event_aliases: HashMap<BitRef, BitRef> = HashMap::default();
        for entry in entries.iter() {
            match &entry.item {
                CombItem::FastDirectCopy { dst_id, src_id }
                    if self.signal_widths.get(*dst_id) == Some(&1)
                        && self.signal_widths.get(*src_id) == Some(&1) =>
                {
                    event_aliases.insert(
                        BitRef {
                            sig_id: *dst_id as u32,
                            bit: 0,
                        },
                        BitRef {
                            sig_id: *src_id as u32,
                            bit: 0,
                        },
                    );
                }
                CombItem::DirectCopy {
                    dst_id,
                    src_id,
                    width: 1,
                } => {
                    event_aliases.insert(
                        BitRef {
                            sig_id: *dst_id as u32,
                            bit: 0,
                        },
                        BitRef {
                            sig_id: *src_id as u32,
                            bit: 0,
                        },
                    );
                }
                CombItem::FusedGate {
                    op:
                        FusedGate::Buf1 {
                            dst,
                            src,
                            invert: false,
                        },
                } => {
                    event_aliases.insert(*dst, *src);
                }
                _ => {}
            }
        }
        let mut event_batch_index: HashMap<BitRef, usize> = HashMap::default();
        let mut event_batches: Vec<(BitRef, Vec<usize>)> = Vec::new();
        // `--primitive-verbose`: print every UDP instance with its source
        // location and each terminal's resolution (net + bit, constant, or
        // failure), so a dropped vendor cell can be diagnosed from the log
        // without the design sources at hand.
        let pv = xezim_core::primitive_verbose();
        for inst in insts {
            if inst.is_sequential {
                pv_seq += 1;
            }
            if pv {
                let loc = self
                    .span_file_line_in(inst.output.span, None)
                    .unwrap_or_else(|| format!("source offset {}", inst.output.span.start));
                eprintln!(
                    "[primitive-verbose] UDP '{}' instance '{}' ({}) — {} input terminal(s), \
                     {}, {} table row(s), init={}{}",
                    inst.udp_name,
                    inst.inst_path,
                    loc,
                    inst.inputs.len(),
                    if inst.is_sequential {
                        "sequential"
                    } else {
                        "combinational"
                    },
                    inst.rows.len(),
                    inst.init.unwrap_or('x'),
                    if inst.delay > 0 {
                        format!(", delay={} tick(s)", inst.delay)
                    } else {
                        String::new()
                    }
                );
            }
            // Resolve output + input terminals to single bit refs.
            let out_ref = match self.try_resolve_bit_ref(&inst.output, None) {
                Some(b) => {
                    if pv {
                        eprintln!(
                            "[primitive-verbose]   out  -> net '{}' bit {}{}",
                            self.name_for_id(b.sig_id as usize),
                            b.bit,
                            self.udp_terminal_note(b.sig_id as usize, b.bit)
                        );
                    }
                    b
                }
                None => {
                    eprintln!(
                        "\n========================================================================\n\
                         Warning: UDP OUTPUT UNRESOLVED — primitive '{}' instance '{}'\n\
                         the output terminal net could not be resolved to a single 1-bit net.\n\
                         Consequence: this instance is DROPPED; its output is left UNDRIVEN.\n\
                         ========================================================================\n",
                        inst.udp_name, inst.inst_path
                    );
                    pv_dropped += 1;
                    continue;
                }
            };
            let mut in_refs = Vec::with_capacity(inst.inputs.len());
            let mut bad: Option<(usize, String)> = None;
            for (ti, e) in inst.inputs.iter().enumerate() {
                if let Some(b) = self.try_resolve_bit_ref(e, None) {
                    if self
                        .module
                        .implicit_nets
                        .contains(self.name_for_id(b.sig_id as usize))
                    {
                        pv_implicit_terms += 1;
                    }
                    if pv {
                        eprintln!(
                            "[primitive-verbose]   in#{} -> net '{}' bit {}{}",
                            ti,
                            self.name_for_id(b.sig_id as usize),
                            b.bit,
                            self.udp_terminal_note(b.sig_id as usize, b.bit)
                        );
                    }
                    in_refs.push(b);
                    continue;
                }
                // A CONSTANT terminal (power/ground pins of power-aware
                // vendor cells are routinely tied to 1'b1/1'b0/'x/'z):
                // encode it as a sentinel BitRef — sig_id=u32::MAX, bit =
                // the 4-state code — read directly by eval_udp and skipped
                // by dependency registration (a constant never changes).
                let mut ce: &Expression = e;
                while let ExprKind::Paren(inner) = &ce.kind {
                    ce = inner;
                }
                if let ExprKind::Number(num) = &ce.kind {
                    let v = self.eval_number(num);
                    let code = v.get_bit_code(0) as u32;
                    if pv {
                        eprintln!(
                            "[primitive-verbose]   in#{} -> constant {}",
                            ti,
                            ['0', '1', 'x', 'z'][code as usize % 4]
                        );
                    }
                    pv_const_terms += 1;
                    in_refs.push(BitRef {
                        sig_id: u32::MAX,
                        bit: code,
                    });
                    continue;
                }
                let desc = self
                    .span_file_line_in(e.span, None)
                    .unwrap_or_else(|| format!("source offset {}", e.span.start));
                if pv {
                    let snippet = self
                        .span_source_snippet_in(e.span, None)
                        .unwrap_or_else(|| "<expression>".to_string());
                    eprintln!(
                        "[primitive-verbose]   in#{} -> FAILED: '{}' at {} is not a 1-bit net, \
                         constant bit-select, or literal",
                        ti, snippet, desc
                    );
                }
                bad = Some((ti, desc));
                break;
            }
            if let Some((ti, desc)) = bad {
                eprintln!(
                    "\n========================================================================\n\
                     Warning: UDP INPUT UNRESOLVED — primitive '{}' instance '{}'\n\
                     input terminal #{} ({}) could not be resolved to a single 1-bit net\n\
                     (supported: a 1-bit net, a constant bit-select of a packed vector,\n\
                     or a constant literal). Consequence: this instance is DROPPED; its\n\
                     output is left UNDRIVEN — downstream logic (e.g. a reset/clock\n\
                     chain through this cell) will never activate.\n\
                     ========================================================================\n",
                    inst.udp_name, inst.inst_path, ti, desc
                );
                pv_dropped += 1;
                continue;
            }

            let idx = self.udp_runtime.len();
            let init_state = match inst.init {
                Some('0') => 0u8,
                Some('1') => 1u8,
                _ => 2u8, // x (default start state)
            };
            let n = in_refs.len();
            let read_ids: Vec<usize> = in_refs
                .iter()
                .filter(|b| b.sig_id != u32::MAX)
                .map(|b| b.sig_id as usize)
                .collect();
            let write_ids: Vec<usize> = vec![out_ref.sig_id as usize];
            let (rows, transition_lut, lut_input_states) =
                if let Some(program) = programs.get(&inst.udp_name) {
                    program.clone()
                } else {
                    let rows: Arc<[crate::ast::decl::UdpTableRow]> =
                        Arc::from(inst.rows.into_boxed_slice());
                    let (transition_lut, lut_input_states) =
                        Self::compile_udp_transition_lut(&rows, n, inst.is_sequential)
                            .map_or((None, 0), |(lut, states)| (Some(Arc::from(lut)), states));
                    let program = (rows, transition_lut, lut_input_states);
                    programs.insert(inst.udp_name.clone(), program.clone());
                    program
                };
            if transition_lut.is_some() {
                pv_lut += 1;
            }
            let (event_input, event_transition_mask) = if inst.is_sequential {
                transition_lut
                    .as_ref()
                    .and_then(|lut| Self::compile_udp_event_filter(lut, lut_input_states, n))
                    .map_or((None, 0), |(input, mask)| (Some(input as u8), mask))
            } else {
                (None, 0)
            };
            if event_input.is_some() {
                pv_event_filtered += 1;
            }
            if !inst.is_sequential
                && inst.delay == 0
                && inst.delay_fall.unwrap_or(0) == 0
                && n <= 3
                && in_refs.iter().all(|input| input.sig_id != u32::MAX)
            {
                if let Some(lut) = transition_lut.as_ref() {
                    let mut inputs = [out_ref; 3];
                    inputs[..n].copy_from_slice(&in_refs);
                    let mut table = 0u64;
                    // For a combinational UDP, previous inputs and state do
                    // not participate in row matching. Extract one current-
                    // input plane from the general transition table.
                    let prev_x = lut_input_states.saturating_sub(1);
                    for cur_idx in 0..lut_input_states {
                        let idx = ((2 * lut_input_states + prev_x) * lut_input_states) + cur_idx;
                        table |= u64::from(lut[idx]) << (cur_idx * 2);
                    }
                    entries.push(CombEntry {
                        item: CombItem::FusedGate {
                            op: FusedGate::UdpLut3 {
                                dst: out_ref,
                                inputs,
                                input_count: n as u8,
                                table,
                            },
                        },
                        cold: Box::new(CombEntryCold {
                            scope_hint: None,
                            read_signal_ids: read_ids,
                            write_signal_ids: write_ids,
                            span: inst.span,
                            cov: None,
                        }),
                        has_unresolved_reads: false,
                        defer_at_time0: false,
                    });
                    continue;
                }
            }
            self.udp_runtime.push(UdpRuntime {
                udp_name: inst.udp_name.clone(),
                inst_path: inst.inst_path.clone(),
                out_ref,
                in_refs,
                is_sequential: inst.is_sequential,
                delay: inst.delay,
                delay_fall: inst.delay_fall.unwrap_or(inst.delay),
                rows,
                transition_lut,
                lut_input_states,
                input_state_idx: lut_input_states.saturating_sub(1),
                event_input,
                event_transition_mask,
                state: init_state,
                prev_inputs: vec![255u8; n],
                initialized: false,
                warned: false,
            });
            if let Some(event_input) = event_input {
                let mut event_ref = self.udp_runtime[idx].in_refs[event_input as usize];
                if event_ref.sig_id != u32::MAX {
                    for _ in 0..64 {
                        let Some(&upstream) = event_aliases.get(&event_ref) else {
                            break;
                        };
                        if upstream == event_ref {
                            break;
                        }
                        event_ref = upstream;
                    }
                    let batch_idx = *event_batch_index.entry(event_ref).or_insert_with(|| {
                        let batch_idx = event_batches.len();
                        event_batches.push((event_ref, Vec::new()));
                        batch_idx
                    });
                    event_batches[batch_idx].1.push(idx);
                    self.has_udp = true;
                    continue;
                }
            }
            entries.push(CombEntry {
                item: CombItem::Udp { idx },
                cold: Box::new(CombEntryCold {
                    scope_hint: None,
                    read_signal_ids: read_ids,
                    write_signal_ids: write_ids,
                    span: inst.span,
                    cov: None,
                }),
                has_unresolved_reads: false,
                defer_at_time0: false,
            });
            self.has_udp = true;
        }
        let pv_batched_instances: usize = event_batches.iter().map(|(_, batch)| batch.len()).sum();
        let pv_batches = event_batches.len();
        for (event_ref, indices) in event_batches {
            let write_signal_ids = indices
                .iter()
                .map(|&idx| self.udp_runtime[idx].out_ref.sig_id as usize)
                .collect();
            entries.push(CombEntry {
                item: CombItem::UdpBatch {
                    event_ref,
                    indices: indices.into_boxed_slice(),
                },
                cold: Box::new(CombEntryCold {
                    scope_hint: None,
                    read_signal_ids: vec![event_ref.sig_id as usize],
                    write_signal_ids,
                    span: crate::ast::Span::dummy(),
                    cov: None,
                }),
                has_unresolved_reads: false,
                defer_at_time0: false,
            });
        }
        if pv && pv_total > 0 {
            eprintln!(
                "[primitive-verbose] UDP summary: {} instance(s) — {} sequential, {} combinational, \
                 {} DROPPED (unresolved terminal), {} constant-tied terminal(s), \
                 {} terminal(s) on implicit nets{}",
                pv_total,
                pv_seq,
                pv_total - pv_seq,
                pv_dropped,
                pv_const_terms,
                pv_implicit_terms,
                if pv_implicit_terms > 0 {
                    " — implicit-net terminals usually mean a cell model was dropped or \
                     unresolved upstream; search the log for its instance"
                } else {
                    ""
                }
            );
        }
        if std::env::var("XEZIM_DEP_STATS").is_ok() && pv_total > 0 {
            eprintln!(
                "[UDP_STATS] instances={} transition_lut={} row_fallback={} event_filtered={} batched_instances={} batches={} shared_definitions={}",
                pv_total,
                pv_lut,
                pv_total.saturating_sub(pv_lut),
                pv_event_filtered,
                pv_batched_instances,
                pv_batches,
                programs.len()
            );
        }
        // A UDP output initializes to x (a reference simulator), not the undriven-wire z — a
        // sequential UDP to its `initial` start state (default x), a
        // combinational UDP to x until its first evaluation drives it. This is
        // visible when an instance `#delay` postpones the first real drive.
        let udp_inits: Vec<(usize, usize, u8)> = self
            .udp_runtime
            .iter()
            .map(|rt| {
                (
                    rt.out_ref.sig_id as usize,
                    rt.out_ref.bit as usize,
                    if rt.is_sequential { rt.state } else { 2 },
                )
            })
            .collect();
        for (id, bit, code) in udp_inits {
            if self.signal_table[id].set_bit_code(bit, code) {
                self.sync_mirror(id);
                self.table_modified = true;
            }
        }
    }

    /// Compile a small UDP definition into a dense ternary transition table.
    /// The table is shared by all flattened instances of that definition.
    pub(super) fn compile_udp_transition_lut(
        rows: &[crate::ast::decl::UdpTableRow],
        input_count: usize,
        is_sequential: bool,
    ) -> Option<(Box<[u8]>, usize)> {
        const MAX_LUT_ENTRIES: usize = 200_000;
        let input_states = 3usize.checked_pow(input_count as u32)?;
        let entries = input_states.checked_mul(input_states)?.checked_mul(3)?;
        if entries > MAX_LUT_ENTRIES {
            return None;
        }

        #[inline]
        fn decode_ternary(mut encoded: usize, values: &mut [u8]) {
            for value in values {
                *value = (encoded % 3) as u8;
                encoded /= 3;
            }
        }

        let mut lut = vec![2u8; entries];
        let mut cur = vec![0u8; input_count];
        let mut prev = vec![0u8; input_count];
        for state in 0..3u8 {
            for prev_idx in 0..input_states {
                decode_ternary(prev_idx, &mut prev);
                for cur_idx in 0..input_states {
                    decode_ternary(cur_idx, &mut cur);
                    let mut next = 2u8;
                    for row in rows {
                        if let Some(output) =
                            Self::udp_match_row(row, &cur, &prev, state, is_sequential)
                        {
                            next = output.unwrap_or(state);
                            break;
                        }
                    }
                    let idx = ((state as usize * input_states + prev_idx) * input_states) + cur_idx;
                    lut[idx] = next;
                }
            }
        }
        Some((lut.into_boxed_slice(), input_states))
    }

    /// Find a single event-bearing input for an edge-triggered sequential UDP.
    /// If changing any combination of the other inputs while this port is
    /// stable always holds state, those inputs are data terminals. The returned
    /// mask identifies event-port transitions that can possibly change state.
    pub(super) fn compile_udp_event_filter(
        lut: &[u8],
        input_states: usize,
        input_count: usize,
    ) -> Option<(usize, u16)> {
        for event_input in 0..input_count {
            let place = 3usize.pow(event_input as u32);
            let digit = |encoded: usize| (encoded / place) % 3;
            let mut passive_holds = true;
            for state in 0..3usize {
                for prev_idx in 0..input_states {
                    for cur_idx in 0..input_states {
                        if prev_idx != cur_idx
                            && digit(prev_idx) == digit(cur_idx)
                            && lut[(state * input_states + prev_idx) * input_states + cur_idx]
                                != state as u8
                        {
                            passive_holds = false;
                            break;
                        }
                    }
                    if !passive_holds {
                        break;
                    }
                }
                if !passive_holds {
                    break;
                }
            }
            if !passive_holds {
                continue;
            }

            let mut mask = 0u16;
            for state in 0..3usize {
                for cur_idx in 0..input_states {
                    let cur_digit = digit(cur_idx);
                    for prev_digit in 0..3usize {
                        if prev_digit == cur_digit {
                            continue;
                        }
                        let prev_idx = cur_idx - cur_digit * place + prev_digit * place;
                        let next = lut[(state * input_states + prev_idx) * input_states + cur_idx];
                        if next != state as u8 {
                            mask |= 1u16 << (prev_digit * 3 + cur_digit);
                        }
                    }
                }
            }
            if mask != 0 {
                return Some((event_input, mask));
            }
        }
        None
    }

    /// IEEE 1800-2017 §29: evaluate one UDP instance on an input change.
    /// Detects per-input edges vs the previous input vector, selects the first
    /// matching table row (edge rows require their input to have transitioned),
    /// and drives the output net (immediately, or after the instance delay).
    pub(super) fn eval_udp(&mut self, idx: usize) {
        // Read current input levels (z normalized to x).
        let n = self.udp_runtime[idx].in_refs.len();
        let mut cur = [2u8; 32];
        if n > 32 {
            // Absurdly wide UDP — not supported; warn once and bail.
            if !self.udp_runtime[idx].warned {
                self.udp_runtime[idx].warned = true;
                eprintln!(
                    "Warning: UDP '{}' instance '{}' has {} inputs (>32 unsupported); \
                    output left unchanged.",
                    self.udp_runtime[idx].udp_name, self.udp_runtime[idx].inst_path, n
                );
            }
            return;
        }
        if self.udp_runtime[idx].initialized {
            if let Some(event_input) = self.udp_runtime[idx].event_input {
                let event_input = event_input as usize;
                let input = self.udp_runtime[idx].in_refs[event_input];
                let code = if input.sig_id == u32::MAX {
                    input.bit as u8
                } else {
                    self.signal_table[input.sig_id as usize]
                        .get_bit_code(input.bit as usize)
                        .min(2)
                };
                let place = 3usize.pow(event_input as u32);
                let prev = (self.udp_runtime[idx].input_state_idx / place) % 3;
                let transition = prev * 3 + code as usize;
                if prev == code as usize {
                    // This entry was triggered by a data-terminal change.
                    // Such changes were proven state-holding by the compiled
                    // filter; refresh their cached trits without a LUT lookup.
                    let mut encoded = self.udp_runtime[idx].input_state_idx;
                    let mut data_place = 1usize;
                    for i in 0..n {
                        if i != event_input {
                            let input = self.udp_runtime[idx].in_refs[i];
                            let input_code = if input.sig_id == u32::MAX {
                                input.bit as u8
                            } else {
                                self.signal_table[input.sig_id as usize]
                                    .get_bit_code(input.bit as usize)
                                    .min(2)
                            };
                            let old = (encoded / data_place) % 3;
                            encoded = encoded - old * data_place + input_code as usize * data_place;
                        }
                        data_place *= 3;
                    }
                    self.udp_runtime[idx].input_state_idx = encoded;
                    return;
                }
                if prev != code as usize
                    && self.udp_runtime[idx].event_transition_mask & (1u16 << transition) == 0
                {
                    let rt = &mut self.udp_runtime[idx];
                    rt.input_state_idx = rt.input_state_idx - prev * place + code as usize * place;
                    return;
                }
                if prev != code as usize {
                    let mut cur_idx = 0usize;
                    let mut input_place = 1usize;
                    for input in &self.udp_runtime[idx].in_refs {
                        let input_code = if input.sig_id == u32::MAX {
                            input.bit as u8
                        } else {
                            self.signal_table[input.sig_id as usize]
                                .get_bit_code(input.bit as usize)
                                .min(2)
                        };
                        cur_idx += input_code as usize * input_place;
                        input_place *= 3;
                    }
                    let prev_idx = cur_idx - code as usize * place + prev * place;
                    let rt = &self.udp_runtime[idx];
                    let lut = rt.transition_lut.as_ref().expect("filtered UDP has LUT");
                    let lut_idx = ((rt.state as usize * rt.lut_input_states + prev_idx)
                        * rt.lut_input_states)
                        + cur_idx;
                    let new_code = lut[lut_idx];
                    {
                        let rt = &mut self.udp_runtime[idx];
                        rt.state = new_code;
                        rt.input_state_idx = cur_idx;
                    }
                    self.drive_udp_output(idx, new_code);
                    return;
                }
            }
        }
        let mut cur_idx = 0usize;
        let mut place = 1usize;
        for i in 0..n {
            let b = self.udp_runtime[idx].in_refs[i];
            let code = if b.sig_id == u32::MAX {
                // Constant terminal (see build_udp_entries): the code IS the value.
                b.bit as u8
            } else {
                self.signal_table[b.sig_id as usize].get_bit_code(b.bit as usize)
            };
            cur[i] = if code >= 2 { 2 } else { code }; // 0,1,x (z->x)
            cur_idx += cur[i] as usize * place;
            place *= 3;
        }

        let rt = &self.udp_runtime[idx];
        let first = !rt.initialized;
        let cur_state = rt.state;
        let has_lut = rt.transition_lut.is_some();

        // Previous input levels. Before the first evaluation each input's prior
        // value is x (its pre-time-0 state), so a t=0 x→value transition is a
        // real edge — matching a reference simulator.
        let mut prev = [2u8; 32];
        if !has_lut {
            for i in 0..n {
                prev[i] = if first { 2 } else { rt.prev_inputs[i] };
            }
        }

        // A UDP is event-driven: it evaluates only when an input actually
        // changes. A spurious re-trigger with no input change must NOT alter the
        // output (a reference simulator never re-evaluates without an event). The very first
        // evaluation always runs so combinational level rows establish t=0.
        let any_change = first
            || if has_lut {
                cur_idx != rt.input_state_idx
            } else {
                (0..n).any(|i| cur[i] != prev[i])
            };
        if !any_change {
            return;
        }

        let new_code = if let Some(lut) = &rt.transition_lut {
            let mut prev_idx = if first {
                rt.lut_input_states.saturating_sub(1)
            } else {
                rt.input_state_idx
            };
            if let Some(event_input) = rt.event_input {
                // Data-terminal changes were intentionally not dispatched.
                // Treat their previous values as already current so this LUT
                // lookup represents only the event-bearing input transition.
                let event_input = event_input as usize;
                let mut place = 1usize;
                for i in 0..n {
                    if i != event_input {
                        let old = (prev_idx / place) % 3;
                        prev_idx = prev_idx - old * place + cur[i] as usize * place;
                    }
                    place *= 3;
                }
            }
            let idx = ((cur_state as usize * rt.lut_input_states + prev_idx) * rt.lut_input_states)
                + cur_idx;
            lut[idx]
        } else {
            // Select the first matching row (top-down).
            let mut next: Option<u8> = None;
            let mut matched = false;
            for row in rt.rows.iter() {
                if let Some(out) =
                    Self::udp_match_row(row, &cur[..n], &prev[..n], cur_state, rt.is_sequential)
                {
                    next = out;
                    matched = true;
                    break;
                }
            }

            // On an input change with no matching row the output becomes x.
            if matched {
                next.unwrap_or(cur_state)
            } else {
                2
            }
        };

        // Commit state + previous-input vector.
        {
            let rt = &mut self.udp_runtime[idx];
            rt.state = new_code;
            rt.input_state_idx = cur_idx;
            for i in 0..n {
                rt.prev_inputs[i] = cur[i];
            }
            rt.initialized = true;
        }

        self.drive_udp_output(idx, new_code);
    }

    /// Commit a UDP result using delayed, sequential-NBA, or immediate
    /// combinational semantics as appropriate for the instance.
    #[inline(always)]
    pub(super) fn drive_udp_output(&mut self, idx: usize, new_code: u8) {
        let out_ref = self.udp_runtime[idx].out_ref;
        let (rise, fall) = (self.udp_runtime[idx].delay, self.udp_runtime[idx].delay_fall);
        let is_seq = self.udp_runtime[idx].is_sequential;
        let id = out_ref.sig_id as usize;
        let bit = out_ref.bit as usize;
        // §29.7 instance delay: applied to every transition, including the
        // initial one at t=0 (a reference simulator leaves the output at x until the delay
        // elapses — unlike SDF back-annotation, which does not act at t=0).
        if rise > 0 || fall > 0 {
            // §28.16: a change to 1 takes the rise delay, to 0 the fall
            // delay, and to x the smaller of the two.
            let delay = match new_code {
                1 => rise,
                0 => fall,
                _ => rise.min(fall),
            };
            // §29.7 instance delays are INERTIAL, and the compare must run
            // against the PENDING delayed value when one exists: gating on
            // the CURRENT value elided the cancelling transition of a pulse
            // narrower than the delay — the rise stayed scheduled, the fall
            // was dropped, and the output stuck high where the reference
            // swallows the glitch (issue #144). `schedule_delayed_slice`
            // composes into pending-or-current and cancels outright when the
            // revert leaves nothing to change.
            let base_code = self
                .delayed_updates
                .iter()
                .find(|(_, sid, _)| *sid == id)
                .map(|(_, _, v)| v.get_bit_code(bit))
                .unwrap_or_else(|| self.signal_table[id].get_bit_code(bit));
            if base_code != new_code {
                let mut sel = Value::new(1);
                sel.set_bit_code(0, new_code);
                self.schedule_delayed_slice(id, bit as u32, 1, &sel, delay);
            }
        } else if is_seq {
            // A SEQUENTIAL UDP is a flip-flop/latch: its output must update with
            // non-blocking semantics so that other sequential UDPs sampling the
            // SAME clock edge see this output's OLD value, not the new one.
            // Committing immediately (like a comb UDP) shifts a whole DFF chain
            // through in one edge — a shift register would collapse. Verified
            // against a commercial simulator and a reference simulator: a `q0->q1` chain must
            // shift one stage per clock. Route through the NBA queue like a flop.
            // §10.4.2 against the PENDING value: a second fire in the same
            // delta must update the queued value (a fire back to the current
            // code was dropped while the first fire's entry survived), and
            // two UDPs driving different BITS of one output word must merge
            // rather than the second clone losing the first's bit. Keeping
            // the entry indexed also lets the RTL NBA paths find it.
            if let Some(i) = self.nba_fast_index.get(id) {
                let tv = &mut self.nba_fast[i].value;
                tv.set_bit_code(bit, new_code);
            } else if self.signal_table[id].get_bit_code(bit) != new_code {
                let mut v = self.signal_table[id].clone();
                v.set_bit_code(bit, new_code);
                self.nba_fast_index.insert(id, self.nba_fast.len());
                self.nba_fast.push(NbaFast {
                    mask: None,
                    signal_id: id,
                    value: v,
                    block_index: 0,
                });
            }
        } else if self.signal_table[id].set_bit_code(bit, new_code) {
            self.sync_mirror(id);
            // Combinational UDP: propagate immediately, like a continuous assign.
            self.table_modified = true;
            self.after_signal_write(id);
            self.mark_dirty_id(id);
        }
    }

    /// Direct event dispatch for sequential UDPs sharing an upstream event
    /// terminal. The event value is read once; inactive edges only update each
    /// instance's cached event trit, while active edges read data terminals and
    /// perform one encoded LUT lookup.
    #[inline(always)]
    pub(super) fn eval_udp_batch(&mut self, event_ref: BitRef, indices: &[usize]) {
        let event_code = self.signal_table[event_ref.sig_id as usize]
            .get_bit_code(event_ref.bit as usize)
            .min(2);
        for &idx in indices {
            if !self.udp_runtime[idx].initialized {
                self.eval_udp(idx);
                continue;
            }
            let event_input = self.udp_runtime[idx]
                .event_input
                .expect("batched UDP has event input") as usize;
            let instance_event_ref = self.udp_runtime[idx].in_refs[event_input];
            let event_code = if !self.forced_signals.is_empty()
                && self
                    .forced_signals
                    .contains_key(&(instance_event_ref.sig_id as usize))
            {
                self.signal_table[instance_event_ref.sig_id as usize]
                    .get_bit_code(instance_event_ref.bit as usize)
                    .min(2)
            } else {
                event_code
            };
            let event_place = 3usize.pow(event_input as u32);
            let prev_event = (self.udp_runtime[idx].input_state_idx / event_place) % 3;
            if prev_event == event_code as usize {
                continue;
            }
            let transition = prev_event * 3 + event_code as usize;
            if self.udp_runtime[idx].event_transition_mask & (1u16 << transition) == 0 {
                let rt = &mut self.udp_runtime[idx];
                rt.input_state_idx = rt.input_state_idx - prev_event * event_place
                    + event_code as usize * event_place;
                continue;
            }

            let mut cur_idx = 0usize;
            let mut place = 1usize;
            for (input_idx, input) in self.udp_runtime[idx].in_refs.iter().enumerate() {
                let code = if input_idx == event_input {
                    event_code
                } else if input.sig_id == u32::MAX {
                    input.bit as u8
                } else {
                    self.signal_table[input.sig_id as usize]
                        .get_bit_code(input.bit as usize)
                        .min(2)
                };
                cur_idx += code as usize * place;
                place *= 3;
            }
            let prev_idx = cur_idx - event_code as usize * event_place + prev_event * event_place;
            let rt = &self.udp_runtime[idx];
            let lut = rt.transition_lut.as_ref().expect("batched UDP has LUT");
            let lut_idx = ((rt.state as usize * rt.lut_input_states + prev_idx)
                * rt.lut_input_states)
                + cur_idx;
            let new_code = lut[lut_idx];
            {
                let rt = &mut self.udp_runtime[idx];
                rt.state = new_code;
                rt.input_state_idx = cur_idx;
            }
            self.drive_udp_output(idx, new_code);
        }
    }

    /// Match one table row against the current/previous input levels and state.
    /// Returns `Some(Some(level))` for an explicit output, `Some(None)` for a
    /// `-` (no-change) output, or `None` if the row does not match.
    pub(super) fn udp_match_row(
        row: &crate::ast::decl::UdpTableRow,
        cur: &[u8],
        prev: &[u8],
        cur_state: u8,
        is_sequential: bool,
    ) -> Option<Option<u8>> {
        use crate::ast::decl::{UdpOut, UdpSym};
        if row.inputs.len() != cur.len() {
            return None;
        }
        // Find the (at most one) edge symbol position.
        let mut edge_pos: Option<usize> = None;
        for (i, sym) in row.inputs.iter().enumerate() {
            if matches!(sym, UdpSym::Edge { .. } | UdpSym::EdgeShort(_)) {
                if edge_pos.is_some() {
                    return None; // two edges in one row — illegal, reject
                }
                edge_pos = Some(i);
            }
        }
        // Level-match the non-edge inputs against their CURRENT value.
        for (i, sym) in row.inputs.iter().enumerate() {
            if Some(i) == edge_pos {
                continue;
            }
            if !Self::udp_level_match(sym, cur[i]) {
                return None;
            }
        }
        // Edge input: require that input to have transitioned and match.
        if let Some(e) = edge_pos {
            let p = prev[e];
            let c = cur[e];
            if p == c {
                return None; // no transition on the edge input
            }
            if !Self::udp_edge_match(&row.inputs[e], p, c) {
                return None;
            }
        }
        // Sequential current-state field.
        if is_sequential {
            if let Some(state_sym) = &row.state {
                if !Self::udp_level_match(state_sym, cur_state) {
                    return None;
                }
            }
        }
        // Matched — resolve the output.
        Some(match row.output {
            UdpOut::Level('0') => Some(0),
            UdpOut::Level('1') => Some(1),
            UdpOut::Level(_) => Some(2),
            UdpOut::NoChange => None,
        })
    }

    /// Level symbol match: `sym` against actual level `c` (0,1,2=x).
    pub(super) fn udp_level_match(sym: &crate::ast::decl::UdpSym, c: u8) -> bool {
        use crate::ast::decl::UdpSym;
        match sym {
            UdpSym::Level('0') => c == 0,
            UdpSym::Level('1') => c == 1,
            UdpSym::Level(_) => c == 2, // 'x'
            UdpSym::AnyQ => true,       // 0/1/x
            UdpSym::B => c == 0 || c == 1,
            // An edge symbol never matches as a level (handled separately).
            UdpSym::Edge { .. } | UdpSym::EdgeShort(_) => false,
        }
    }

    /// Edge symbol match: transition prev `p` -> cur `c` (levels 0,1,2=x).
    pub(super) fn udp_edge_match(sym: &crate::ast::decl::UdpSym, p: u8, c: u8) -> bool {
        use crate::ast::decl::UdpSym;
        let lc = |code: u8| -> char {
            match code {
                0 => '0',
                1 => '1',
                _ => 'x',
            }
        };
        let (pc, cc) = (lc(p), lc(c));
        let ml = |s: char, a: char| s == '?' || s == a;
        match sym {
            UdpSym::Edge { from, to } => ml(*from, pc) && ml(*to, cc),
            UdpSym::EdgeShort('r') => pc == '0' && cc == '1',
            UdpSym::EdgeShort('f') => pc == '1' && cc == '0',
            UdpSym::EdgeShort('p') => matches!((pc, cc), ('0', '1') | ('0', 'x') | ('x', '1')),
            UdpSym::EdgeShort('n') => matches!((pc, cc), ('1', '0') | ('1', 'x') | ('x', '0')),
            UdpSym::EdgeShort('*') => pc != cc, // any change
            _ => false,
        }
    }

    pub(super) fn cont_assign_lhs_is_noop(&self, lhs: &Expression) -> bool {
        match &lhs.kind {
            ExprKind::RangeSelect { expr, .. } => {
                let ExprKind::Index { expr: base, .. } = &expr.kind else {
                    return false;
                };
                let ExprKind::Ident(hier) = &base.kind else {
                    return false;
                };
                let name = self.resolve_hier_name(hier);
                !self.module.arrays.contains_key(&*name)
                    && !self.module.arrays_2d.contains_key(&*name)
                    && !self.module.arrays_nd.contains_key(&*name)
                    && !self.signal_name_to_id.contains_key(name.as_ref())
            }
            ExprKind::Index { expr, .. } => {
                let ExprKind::Index {
                    expr: inner_base, ..
                } = &expr.kind
                else {
                    return false;
                };
                let ExprKind::Ident(hier) = &inner_base.kind else {
                    return false;
                };
                let name = self.resolve_hier_name(hier);
                // A multi-D PACKED vector root (`logic [1:0][3:0][7:0] foo`)
                // is a real write target for `foo[i][j]` — only drop the
                // assign when the root resolves to nothing at all.
                !self.module.arrays.contains_key(&*name)
                    && !self.module.arrays_2d.contains_key(&*name)
                    && !self.module.arrays_nd.contains_key(&*name)
                    && !self.signal_name_to_id.contains_key(name.as_ref())
            }
            _ => false,
        }
    }
}
