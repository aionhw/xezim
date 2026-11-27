//! VPI value access (IEEE 1800 §38.16 and §38.35).
use super::*;

/// Read a signal value via VPI.
///
/// On any failure — bad handle, out-of-range signal, or a format xezim
/// cannot supply — sets `value_p->format` to `vpiSuppressVal` and writes
/// nothing else. IEEE 1800-2017 §38.16 makes that the caller's only way
/// to detect the failure, and `vpi_get_value` returns `void`, so there is
/// nowhere else to say so.
///
/// This used to handle exactly two formats and silently ignore the rest.
/// `vpiVectorVal` was among the ignored ones, which is the format
/// HDL backdoor read routines use: `vpi_get_value` returned success having
/// written nothing into the caller's buffer.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_get_value(handle: *mut libc::c_void, value_p: *mut s_vpi_value) {
    if value_p.is_null() {
        return;
    }
    let vp = unsafe { &mut *value_p };
    let Some(h) = (unsafe { vpi_deref(handle) }) else {
        vpi_error(vpi::ERROR, "vpi_get_value: null handle".into());
        vp.format = vpi::SUPPRESS_VAL;
        return;
    };
    // VPI object model (vpi_model.rs): expressions, constants, and the
    // objects that have no value.
    if matches!(h.kind, VpiKind::Obj | VpiKind::Scope) || h.type_code == vpi_model::c::NAMED_EVENT {
        let _ = try_active_sim("vpi_get_value", |sim| vpi_model::get_value(sim, h, vp));
        return;
    }
    // A constant argument carries its own value and needs no simulator.
    if h.kind == VpiKind::Constant {
        let ok = match &h.value {
            Some(v) => fill_vpi_value(v, 0, Some(h.type_code), vp),
            None => false,
        };
        if !ok {
            vpi_error(
                vpi::ERROR,
                format!(
                    "vpi_get_value: cannot supply format {} for a constant",
                    vp.format
                ),
            );
            vp.format = vpi::SUPPRESS_VAL;
        }
        return;
    }
    // Modules, memories, iterators and call handles have no readable value.
    if matches!(
        h.kind,
        VpiKind::Module
            | VpiKind::Iterator
            | VpiKind::Memory
            | VpiKind::SysTfCall
            | VpiKind::UserSystf
            | VpiKind::ModPath
            | VpiKind::Tchk
            | VpiKind::InterModPath
            | VpiKind::Callback
    ) {
        vpi_error(
            vpi::ERROR,
            "vpi_get_value: this object has no readable value".into(),
        );
        vp.format = vpi::SUPPRESS_VAL;
        return;
    }
    let sig_id = h.signal_id;

    let ok = try_active_sim("vpi_get_value", |sim| {
        let val = if h.kind == VpiKind::Slice {
            match vpi_slice_read(sim, h) {
                Some(v) => v,
                None => return false,
            }
        } else {
            match vpi_api::cell_value(sim, sig_id) {
                Some(v) => v,
                None => return false,
            }
        };
        let current_time = sim.time;
        // vpiStrengthVal reports the driving strength of a net.
        vpi_api::set_strength_hint(sim, sig_id, h.type_code, vp.format);
        let ok = fill_vpi_value(&val, current_time, Some(h.type_code), vp);
        vpi_api::clear_strength_hint();
        ok
    })
    .unwrap_or(false);

    if !ok {
        // vpiSuppressVal is the only failure channel this function has, and a
        // caller that never checks `format` needs vpi_chk_error to notice.
        vpi_error(
            vpi::ERROR,
            format!("vpi_get_value: cannot supply format {}", vp.format),
        );
        vp.format = vpi::SUPPRESS_VAL;
    }
}

/// Render `val` into `vp` using `vp.format`. Returns false when the
/// format cannot be supplied; the caller then sets `vpiSuppressVal`.
///
/// Shared by `vpi_get_value` and the value-change dispatcher, so a
/// callback sees its trigger object's value in exactly the format a
/// direct read would have produced.
pub(super) fn fill_vpi_value(
    val: &Value,
    _current_time: u64,
    obj_type_code: Option<libc::c_int>,
    vp: &mut s_vpi_value,
) -> bool {
    {
        // vpiObjTypeVal: the simulator picks the object's natural format
        // and reports which one it chose in `format` (§38.15).
        let mut format = vp.format;
        if format == vpi::OBJ_TYPE_VAL {
            format = vpi_api::obj_type_format(val, obj_type_code);
            vp.format = format;
        }
        // §38.15: a real object is converted to an integer, rounded as
        // §6.12.2 specifies, for every format but vpiRealVal and vpiStringVal.
        let rounded;
        let val = if val.is_real && !vpi_api::format_keeps_real(format) {
            rounded = vpi_api::real_to_integer(val);
            &rounded
        } else {
            val
        };

        match format {
            vpi::INT_VAL => {
                // PLI_INT32: the low 32 bits, X/Z read as 0, a signed value
                // sign-extended. Callers that need the full width must use
                // vpiVectorVal; vpiSize tells them whether they do.
                let n = if val.is_signed {
                    val.to_i64().unwrap_or(0) as u64
                } else {
                    val.to_u64().unwrap_or(0)
                };
                vp.value.integer = (n & 0xFFFF_FFFF) as u32 as libc::c_int;
            }
            vpi::REAL_VAL => vp.value.real = val.to_f64(),
            vpi::SCALAR_VAL => {
                vp.value.scalar = match val.get_bit_code(0) {
                    0 => vpi::SCALAR_0,
                    1 => vpi::SCALAR_1,
                    2 => vpi::SCALAR_X,
                    _ => vpi::SCALAR_Z,
                };
            }
            vpi::VECTOR_VAL => {
                let ptr = VPI_VEC_SCRATCH.with(|cell| {
                    let mut buf = cell.borrow_mut();
                    value_to_vecval(val, &mut buf);
                    buf.as_mut_ptr()
                });
                vp.value.vector = ptr;
            }
            vpi::BIN_STR_VAL => vp.value.str = vpi_str_scratch(&vpi_radix_string(val, 1)),
            vpi::OCT_STR_VAL => vp.value.str = vpi_str_scratch(&vpi_api::radix_string_xz(val, 3)),
            vpi::HEX_STR_VAL => vp.value.str = vpi_str_scratch(&vpi_api::radix_string_xz(val, 4)),
            vpi::DEC_STR_VAL => vp.value.str = vpi_str_scratch(&val.to_dec_string()),
            vpi::STRING_VAL if val.is_real => {
                vp.value.str = vpi_str_scratch(&vpi_api::real_string(val.to_f64()))
            }
            vpi::STRING_VAL => vp.value.str = vpi_str_scratch(&val.to_sv_string()),
            vpi::TIME_VAL => {
                let t = val.to_u64().unwrap_or(0);
                let ptr = VPI_TIME_SCRATCH.with(|cell| {
                    let mut slot = cell.borrow_mut();
                    *slot = s_vpi_time {
                        type_: vpi::SIM_TIME,
                        high: (t >> 32) as u32,
                        low: (t & 0xFFFF_FFFF) as u32,
                        // §37 vpiTimeVal: `.real` must describe the SAME value
                        // as high/low (the value being read, `t`), not the sim
                        // clock. Using `current_time` made the record
                        // self-inconsistent (a `time` var holding 42 read at
                        // tick 5 reported real=5).
                        real: t as f64,
                    };
                    cell.as_ptr()
                });
                vp.value.time = ptr;
            }
            vpi::SUPPRESS_VAL => {} // caller explicitly wants no value
            // vpiStrengthVal, the short/long/shortreal formats and the raw
            // formats; false for anything unknown.
            other => return vpi_api::fill_extra_format(val, obj_type_code, other, vp),
        }
        true
    }
}

/// Write a signal value via VPI (supports force/release).
#[unsafe(no_mangle)]
pub extern "C" fn vpi_put_value(
    handle: *mut libc::c_void,
    value_p: *mut s_vpi_value,
    _time_p: *mut s_vpi_time,
    flags: libc::c_int,
) -> *mut libc::c_void {
    if handle.is_null() {
        return std::ptr::null_mut();
    }

    // Depositing into the call handle sets a system function's return value
    // (IEEE 1800-2017 §38.33). Handled before the simulator lookup because it
    // touches only the call frame.
    {
        let Some(h) = (unsafe { vpi_deref(handle) }) else {
            return std::ptr::null_mut();
        };
        if h.kind == VpiKind::SysTfCall {
            if value_p.is_null() {
                vpi_error(
                    vpi::ERROR,
                    "vpi_put_value: null value on a $systf call".into(),
                );
                return std::ptr::null_mut();
            }
            let w = h.width.max(1);
            let vp = unsafe { &*value_p };
            let v = match vp.format {
                vpi::INT_VAL => Value::from_u64(unsafe { vp.value.integer } as i64 as u64, w),
                vpi::REAL_VAL => Value::from_f64(unsafe { vp.value.real }),
                vpi::VECTOR_VAL => {
                    let ptr = unsafe { vp.value.vector };
                    if ptr.is_null() {
                        vpi_error(
                            vpi::ERROR,
                            "vpi_put_value: null vector on a $systf call".into(),
                        );
                        return std::ptr::null_mut();
                    }
                    let words = (w as usize).div_ceil(32);
                    vecval_to_value(unsafe { std::slice::from_raw_parts(ptr, words) }, w, false)
                }
                other => {
                    vpi_error(
                        vpi::ERROR,
                        format!(
                            "vpi_put_value: format {} unsupported on a $systf call",
                            other
                        ),
                    );
                    return std::ptr::null_mut();
                }
            };
            let frame = h.frame;
            VPI_SYSTF_STACK.with(|st| {
                if let Some(f) = st.borrow_mut().get_mut(frame) {
                    if f.tf_type == vpi::SYS_FUNC {
                        f.ret = Some(v);
                    } else {
                        vpi_error(
                            vpi::ERROR,
                            "vpi_put_value: a $systf TASK has no return value".into(),
                        );
                    }
                }
            });
            return std::ptr::null_mut();
        }
        if h.kind == VpiKind::Constant {
            vpi_error(
                vpi::ERROR,
                "vpi_put_value: a vpiConstant argument is read-only".into(),
            );
            return std::ptr::null_mut();
        }
        // VPI object model (vpi_model.rs): objects with no writable value.
        if vpi_model::put_value_rejected(h) {
            return std::ptr::null_mut();
        }
    }

    try_active_sim("vpi_put_value", |sim| {
        let h = unsafe { &*(handle as *const VpiHandle) };
        if matches!(
            h.kind,
            VpiKind::Module | VpiKind::Iterator | VpiKind::Memory | VpiKind::Callback
        ) {
            vpi_error(
                vpi::ERROR,
                "vpi_put_value: this object has no value; ignored".into(),
            );
            return std::ptr::null_mut();
        }
        // §38.36.2: nothing may be written from a cbReadOnlySynch routine.
        if sim.vpi_in_read_only() {
            vpi_error(
                vpi::ERROR,
                "vpi_put_value: writes are not allowed in the read-only synch region; ignored"
                    .into(),
            );
            return std::ptr::null_mut();
        }
        let sig_id = h.signal_id;
        if matches!(
            h.kind,
            VpiKind::UserSystf | VpiKind::ModPath | VpiKind::Tchk | VpiKind::InterModPath
        ) {
            vpi_error(
                vpi::ERROR,
                "vpi_put_value: this object has no value; ignored".into(),
            );
            return std::ptr::null_mut();
        }

        if flags == vpi::RELEASE_FLAG && is_packed_id(sig_id) {
            // Nothing can force a packed-arena word, so there is nothing to release.
            return std::ptr::null_mut();
        }
        if flags == vpi::RELEASE_FLAG {
            // §38.36.1 cbRelease, once the net has been re-driven.
            if sim.vpi_cb_mask != 0 {
                sim.vpi_queue_release(vpi_cb::CB_RELEASE, Some(sig_id));
            }
            // Release: remove from forced_signals and trigger settle
            sim.forced_signals.remove(&sig_id);
            // Re-evaluate the continuous drivers of the released net.
            sim.redrive_released_signal(sig_id);
            // Release invalidates the event-edge assumption that a flop's
            // output still reflects its last sampled inputs.
            sim.edge_block_snap_valid
                .iter_mut()
                .for_each(|v| *v = false);
            sim.edge_block_armed
                .iter_mut()
                .for_each(|v| *v |= EDGE_ARMED);
            sim.dirty_any = true;
            return std::ptr::null_mut();
        }

        if value_p.is_null() {
            vpi_error(
                vpi::ERROR,
                "vpi_put_value: null value_p with a write flag; ignored".into(),
            );
            return std::ptr::null_mut();
        }
        let vp = unsafe { &*value_p };
        // §38.34 disallows vpiStringVal on real variables. Use the same
        // storage check as fit_put_value, including for real-typed nets.
        if vp.format == vpi::STRING_VAL
            && h.kind != VpiKind::Slice
            && vpi_api::cell_real(sim, sig_id)
        {
            vpi_error(
                vpi::ERROR,
                "vpi_put_value: vpiStringVal is illegal on a real target".into(),
            );
            return std::ptr::null_mut();
        }

        // A slice decodes at its own width, then splices back into its parent.
        let w = if h.kind == VpiKind::Slice {
            h.width
        } else {
            vpi_api::cell_width(sim, sig_id).unwrap_or(32)
        };
        let is_signed = vpi_api::cell_signed(sim, sig_id);

        // A format we cannot decode writes NOTHING. The old code fell
        // through to "read the current value and write it back", which
        // looks like a successful no-op and hides the unsupported format.
        let value = match vp.format {
            vpi::INT_VAL => {
                // PLI_INT32 is signed; widen through i64 so a negative
                // integer sign-extends into a wider signal.
                let ival = unsafe { vp.value.integer } as i64;
                let mut v = Value::from_u64(ival as u64, w);
                v.is_signed = is_signed;
                v
            }
            vpi::REAL_VAL => Value::from_f64(unsafe { vp.value.real }),
            vpi::SCALAR_VAL => {
                let mut v = Value::zero(w);
                let code = match unsafe { vp.value.scalar } {
                    // vpiL / vpiH are the weak 0 / 1 levels.
                    vpi::SCALAR_0 | vpi_api::vc::SCALAR_L => 0,
                    vpi::SCALAR_1 | vpi_api::vc::SCALAR_H => 1,
                    vpi::SCALAR_X => 2,
                    vpi::SCALAR_Z => 3,
                    other => {
                        vpi_error(
                            vpi::ERROR,
                            format!("vpi_put_value: unsupported vpiScalarVal code {}", other),
                        );
                        return std::ptr::null_mut();
                    }
                };
                v.set_bit_code(0, code);
                v.is_signed = is_signed;
                v
            }
            vpi::VECTOR_VAL => {
                // Every word of the signal's width, and both aval and
                // bval, so X and Z survive a deposit. The old code read
                // only `vec[0].aval` for a signal of 64 bits or fewer —
                // dropping the upper word of anything wider than 32 —
                // masked X/Z away as `aval & !bval`, and capped the wide
                // path at four words (128 bits).
                let vec_ptr = unsafe { vp.value.vector };
                if vec_ptr.is_null() {
                    vpi_error(
                        vpi::ERROR,
                        "vpi_put_value: vpiVectorVal with a null vector; ignored".into(),
                    );
                    return std::ptr::null_mut();
                }
                let num_words = (w as usize).div_ceil(32);
                let vec = unsafe { std::slice::from_raw_parts(vec_ptr, num_words) };
                vecval_to_value(vec, w, is_signed)
            }
            // §38.16 string payloads. cocotb writes 4-state values this way
            // (a `LogicArray` carrying x/z has no lossless integer form), so
            // without these every cocotb drive of a clock or reset was
            // rejected here and silently never reached the signal.
            vpi::BIN_STR_VAL
            | vpi::OCT_STR_VAL
            | vpi::HEX_STR_VAL
            | vpi::DEC_STR_VAL
            | vpi::STRING_VAL => {
                let sp = unsafe { vp.value.str };
                if sp.is_null() {
                    vpi_error(
                        vpi::ERROR,
                        format!(
                            "vpi_put_value: null string for format {}; ignored",
                            vp.format
                        ),
                    );
                    return std::ptr::null_mut();
                }
                let text = match unsafe { std::ffi::CStr::from_ptr(sp) }.to_str() {
                    Ok(t) => t.trim(),
                    Err(_) => {
                        vpi_error(
                            vpi::ERROR,
                            "vpi_put_value: string payload is not valid UTF-8; ignored".into(),
                        );
                        return std::ptr::null_mut();
                    }
                };
                let mut v = match vp.format {
                    vpi::BIN_STR_VAL => vpi_value_from_radix_str(text, 1, w),
                    vpi::OCT_STR_VAL => vpi_value_from_radix_str(text, 3, w),
                    vpi::HEX_STR_VAL => vpi_value_from_radix_str(text, 4, w),
                    vpi::DEC_STR_VAL => match text.parse::<i64>() {
                        Ok(n) => Value::from_u64(n as u64, w),
                        Err(_) => {
                            vpi_error(
                                vpi::ERROR,
                                format!("vpi_put_value: '{}' is not a decimal; ignored", text),
                            );
                            return std::ptr::null_mut();
                        }
                    },
                    // vpiStringVal: the payload is CHARACTERS, packed
                    // right-aligned one byte per 8 bits.
                    _ => {
                        let mut sv = Value::zero(w);
                        for (i, byte) in text.as_bytes().iter().rev().enumerate() {
                            for k in 0..8u32 {
                                let bit = i as u32 * 8 + k;
                                if bit >= w {
                                    break;
                                }
                                sv.set_bit_code(bit as usize, ((byte >> k) & 1) as u8);
                            }
                        }
                        sv
                    }
                };
                v.is_signed = is_signed;
                v
            }
            // vpiTimeVal, vpiStrengthVal, the short/long/shortreal formats and
            // the raw formats.
            other => match vpi_api::decode_extra_put(vp, other, w, is_signed) {
                Ok(v) => v,
                Err(msg) => {
                    vpi_error(
                        vpi::ERROR,
                        format!("vpi_put_value: {} (nothing written)", msg),
                    );
                    return std::ptr::null_mut();
                }
            },
        };
        // §6.12.2: an integral value put to a real object converts to real, a
        // real one put to an integral object rounds to an integer.
        let value = vpi_api::fit_put_value(sim, h, value, w, is_signed);

        // Splice a slice back into its parent before it hits the signal table.
        let value = if h.kind == VpiKind::Slice {
            match vpi_slice_write(sim, h, &value) {
                Some(v) => v,
                None => return std::ptr::null_mut(),
            }
        } else {
            value
        };

        // A word of a packed-arena memory has no signal-table slot.
        if is_packed_id(sig_id) {
            if flags == vpi::FORCE_FLAG {
                vpi_error(
                    vpi::ERROR,
                    "vpi_put_value: an element of a packed-arena memory cannot be forced".into(),
                );
            } else {
                sim.cell_write(sig_id, &value);
            }
            return std::ptr::null_mut();
        }

        // For force: write value first, THEN add to forced_signals
        if flags == vpi::FORCE_FLAG {
            // Write directly to signal_table (bypass write_sig! which would skip)
            if sig_id < sim.signal_table.len() {
                sim.signal_table[sig_id] = value.clone();
                sim.sync_mirror(sig_id);
                if sig_id < sim.signal_has_xz.len() {
                    sim.signal_has_xz[sig_id] = if value.raw_bits().1 != 0 { 1u8 } else { 0u8 };
                }
                if sig_id < sim.signal_inline_bits.len() {
                    let (v, x) = value.raw_bits();
                    sim.signal_inline_bits[sig_id] = [v, x];
                }
                sim.after_signal_write(sig_id);
            }
            sim.ctl_note_external_write(sig_id);
            // Now mark as forced (after the write succeeded)
            sim.forced_signals.insert(sig_id, value);
            // §38.36.1 cbForce.
            if sim.vpi_cb_mask != 0 {
                sim.vpi_notify_override(vpi_cb::CB_FORCE, Some(sig_id));
            }
        } else {
            // Normal write - write_sig! will skip if signal is forced
            write_sig!(sim, sig_id, value);
            sim.ctl_note_external_write(sig_id);
            // A VPI deposit is an EXTERNAL write: no HDL driver stands behind
            // it, so nothing else marks the signal dirty or re-snapshots it for
            // edge detection. Without this a cocotb-driven clock landed in the
            // signal table and propagated nowhere — `always @(posedge clk)` and
            // even `always @(clk)` never fired, so the DUT sat at x while the
            // Python side saw its own writes read back correctly.
            sim.after_signal_write(sig_id);
            if sig_id < sim.dirty_signals.len() && !sim.dirty_signals[sig_id] {
                sim.dirty_signals[sig_id] = true;
                sim.dirty_list.push(sig_id);
            }
            sim.dirty_any = true;
        }

        std::ptr::null_mut()
    })
    .unwrap_or(std::ptr::null_mut())
}
