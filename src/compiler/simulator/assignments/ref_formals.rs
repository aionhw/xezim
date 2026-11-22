//! Redirection of `ref` formals to their actuals.

use super::*;

impl Simulator {
    /// §13.5.2: an element, bit or part-select access on a `ref` formal
    /// (`a[i]`, `a[i][j]`, `a[7:0]`, `a[i].f`) goes THROUGH to the caller's
    /// actual — for a formal aliased to module-visible storage the storage
    /// name replaces the formal; for an aggregate bound by identity (no local
    /// copy exists) the actual's own hierarchical name is extended with the
    /// formal's selects. Whole-name reads and writes keep their existing
    /// routing (see `ref_alias_stack`); this covers the selected forms, which
    /// previously wrote a dead local copy while the matching read went to
    /// the storage — `a[1] = 9` inside the task read back x.
    pub(in crate::compiler::simulator) fn ref_formal_redirect_hier(
        &self,
        hier: &crate::ast::expr::HierarchicalIdentifier,
        selected: bool,
    ) -> Option<crate::ast::expr::HierarchicalIdentifier> {
        if hier.root.is_some() || hier.path.is_empty() {
            return None;
        }
        // A bare name is redirected only underneath an `Index`/`RangeSelect`
        // wrapper (`selected`); the whole-name forms keep their own routing.
        if !selected && hier.path.len() == 1 && hier.path[0].selects.is_empty() {
            return None;
        }
        let name = hier.path[0].name.name.as_str();
        // §13.5.2: an UNPACKED-struct formal binds member-wise into the frame
        // (its `<formal>.` marker) and copies back on return, as the
        // MemberAccess arm of `ref_formal_redirect_inner` already honours.
        // Redirecting a member ELEMENT (`r.arr[i]`, `r.mm[i][j]`) to the
        // actual bypassed that copy, and the stale frame value then
        // clobbered the write at return. Only a member whose storage is
        // per-element leaves (no frame key of its own) is exempt; a member
        // that is one frame leaf keeps the redirect. Probed only for a name
        // that would redirect.
        let member_wise_struct = |sim: &Self| {
            if !sim.any_struct_formal_markers
                || hier.path.len() < 2
                || hier.path.iter().any(|s| !s.selects.is_empty())
            {
                return false;
            }
            let Some(frame) = sim.local_stack.last() else {
                return false;
            };
            let path = hier
                .path
                .iter()
                .map(|s| s.name.name.as_str())
                .collect::<Vec<_>>()
                .join(".");
            frame.contains_key(format!("{}.", name).as_str()) && !frame.contains_key(path.as_str())
        };
        if let Some(storage) = self.ref_alias_stack.last().and_then(|m| m.get(name)) {
            // A formal bound to an actual of the SAME name (`run_checks(ref
            // u7_t [..] cnt)` called as `run_checks(cnt)`) rewrote the
            // identifier to itself, and the evaluator re-entered this
            // redirect on the rewritten node until the stack overflowed.
            // Plain resolution of the name already reaches that storage.
            if storage == name
                || member_wise_struct(self)
                || (hier.path.len() >= 2 && self.ref_elem_handle_formal(name))
            {
                return None;
            }
            // A pinned property (`<pin>.<prop>`): the formal's selects go on
            // the property segment.
            if let Some(mut pinned) = Self::ref_pin_hier(storage, hier.span) {
                // A member of a packed struct formal is sliced out of the
                // property's value by the packed-leaf paths (see
                // `ref_alias_base_value`); the pin has no declared type to
                // resolve the member through.
                if hier.path.len() >= 2
                    && hier.path[0].selects.is_empty()
                    && self
                        .packed_leaf_of_hier(&format!("{}.{}", name, hier.path[1].name.name))
                        .is_some()
                {
                    return None;
                }
                let mut rest = hier.path.clone();
                let first = rest.remove(0);
                pinned.path[1].selects = first.selects;
                pinned.path.extend(rest);
                return Some(pinned);
            }
            let mut path = hier.path.clone();
            path[0].name.name = storage.clone();
            return Some(crate::ast::expr::HierarchicalIdentifier {
                root: None,
                path,
                span: hier.span,
                cached_signal_id: crate::ast::cache::SignalCache::new(None),
                cached_resolved_name: crate::ast::cache::NameCache::new(),
                bound: crate::ast::binding::Bound::UNSET,
            });
        }
        // Identity-bound aggregate: the formal has no storage of its own.
        // Formals the copy-in machineries own (scalars with a local slot, 1-D
        // arrays, associative arrays, queues — copied back on return) are not
        // listed: a direct write to the actual would be undone by the stale
        // copy-back.
        if !self
            .ref_identity_stack
            .last()
            .is_some_and(|v| v.iter().any(|n| n == name))
            || member_wise_struct(self)
        {
            return None;
        }
        let actual = self.ref_binding_stack.last().and_then(|m| m.get(name))?;
        let ExprKind::Ident(ah) = &actual.kind else {
            return None;
        };
        if ah.path.is_empty() {
            return None;
        }
        // A self-mapped binding (`a` -> `a`) would rewrite forever.
        if ah.path.len() == 1 && ah.path[0].selects.is_empty() && ah.path[0].name.name == name {
            return None;
        }
        let mut path = ah.path.clone();
        let mut rest = hier.path.clone();
        let first = rest.remove(0);
        path.last_mut().unwrap().selects.extend(first.selects);
        path.extend(rest);
        Some(crate::ast::expr::HierarchicalIdentifier {
            root: ah.root.clone(),
            path,
            span: hier.span,
            cached_signal_id: crate::ast::cache::SignalCache::new(None),
            cached_resolved_name: crate::ast::cache::NameCache::new(),
            bound: crate::ast::binding::Bound::UNSET,
        })
    }

    /// `ref_formal_redirect_hier` applied through `Index`/`RangeSelect`
    /// wrappers down to the root identifier of an lvalue or operand.
    pub(in crate::compiler::simulator) fn ref_formal_redirect_expr(
        &self,
        e: &Expression,
    ) -> Option<Expression> {
        if !self.ref_redirect_possible() {
            return None;
        }
        self.ref_formal_redirect_inner(e, false)
    }

    /// IEEE 1800-2023 §13.5.2: a `ref` formal names the actual's storage,
    /// so an event control on it (`@(posedge sig)`, `@(sig)`, `@(b[2])`)
    /// waits for a change of the ACTUAL. The formal itself is a frame value
    /// with no signal behind it: the wait resolved to nothing, fell through
    /// to the NBA-region fallback and resumed in the same time step. Each
    /// term naming a formal aliased to module-visible storage (or an
    /// identity-bound aggregate) is rewritten onto the actual. `None` when
    /// no term changes.
    pub(in crate::compiler::simulator) fn ref_event_control(
        &self,
        event: &EventControl,
    ) -> Option<EventControl> {
        let redirect = |e: &Expression| -> Option<Expression> {
            let mut inner = e;
            while let ExprKind::Paren(x) = &inner.kind {
                inner = x;
            }
            self.ref_formal_redirect_inner(inner, true)
        };
        match event {
            EventControl::HierIdentifier(e) => redirect(e).map(EventControl::HierIdentifier),
            EventControl::Identifier(id) => {
                let e = Expression::new(
                    ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                        root: None,
                        path: vec![crate::ast::expr::HierPathSegment {
                            name: id.clone(),
                            selects: Vec::new(),
                        }],
                        span: id.span,
                        cached_signal_id: crate::ast::cache::SignalCache::new(None),
                        cached_resolved_name: crate::ast::cache::NameCache::new(),
                        bound: crate::ast::binding::Bound::UNSET,
                    }),
                    id.span,
                );
                redirect(&e).map(EventControl::HierIdentifier)
            }
            EventControl::EventExpr(terms) => {
                let mut out: Option<Vec<crate::ast::stmt::EventExpr>> = None;
                for (k, t) in terms.iter().enumerate() {
                    if let Some(ne) = redirect(&t.expr) {
                        let v = out.get_or_insert_with(|| terms.clone());
                        v[k].expr = ne;
                    }
                }
                out.map(EventControl::EventExpr)
            }
            EventControl::Star | EventControl::ParenStar => None,
        }
    }

    /// Cheap reject for the hot paths: only a call with an aliased or an
    /// identity-bound `ref` formal can redirect anything.
    #[inline(always)]
    pub(in crate::compiler::simulator) fn ref_redirect_possible(&self) -> bool {
        self.ref_redirect_hot
    }

    pub(in crate::compiler::simulator) fn refresh_ref_redirect_hot(&mut self) {
        self.ref_redirect_hot = self.ref_alias_stack.last().is_some_and(|m| !m.is_empty())
            || self
                .ref_identity_stack
                .last()
                .is_some_and(|v| !v.is_empty());
    }

    pub(in crate::compiler::simulator) fn ref_formal_redirect_inner(
        &self,
        e: &Expression,
        selected: bool,
    ) -> Option<Expression> {
        match &e.kind {
            ExprKind::Ident(h) => self
                .ref_formal_redirect_hier(h, selected)
                .map(|nh| Expression::new(ExprKind::Ident(nh), e.span)),
            ExprKind::Index { expr, index } => {
                self.ref_formal_redirect_inner(expr, true).map(|ne| {
                    Expression::new(
                        ExprKind::Index {
                            expr: Box::new(ne),
                            index: index.clone(),
                        },
                        e.span,
                    )
                })
            }
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => self.ref_formal_redirect_inner(expr, true).map(|ne| {
                Expression::new(
                    ExprKind::RangeSelect {
                        expr: Box::new(ne),
                        kind: kind.clone(),
                        left: left.clone(),
                        right: right.clone(),
                    },
                    e.span,
                )
            }),
            // §13.5.2: `r.f = v` on a `ref` formal — the MemberAccess shape
            // (how a member write parses inside a subroutine body) missed
            // the redirect, so the write fell to the member-write
            // fall-throughs, which read the formal's integral value back as
            // a heap handle (id-collision-in-heap) or vanished on a phantom
            // name. Redirect the RECEIVER to the caller's actual and rebuild
            // the access. An UNPACKED-struct formal is exempt: it binds
            // member-wise (frame keys `<formal>.<member>`) and the return
            // copy-back delivers the frame copy to the caller's actual —
            // redirecting here would bypass that copy and let the stale
            // call-time value clobber the write at return.
            ExprKind::MemberAccess { expr, member } => {
                if let ExprKind::Ident(rh) = &expr.kind {
                    if rh.path.len() == 1 && rh.path[0].selects.is_empty() {
                        let key = format!("{}.{}", rh.path[0].name.name, member.name);
                        // A member with per-element (or nested) leaves has
                        // no key of its own; the formal's `<formal>.` marker
                        // still says it is bound member-wise.
                        if self.local_stack.last().is_some_and(|f| {
                            f.contains_key(&key)
                                || (self.any_struct_formal_markers
                                    && f.contains_key(
                                        format!("{}.", rh.path[0].name.name).as_str(),
                                    ))
                        }) {
                            return None;
                        }
                    }
                }
                // A handle formal aliased to an element is read from its
                // refreshed frame slot (see `ref_elem_handle_formal`).
                if Self::expr_root_first_seg(expr).is_some_and(|r| self.ref_elem_handle_formal(r)) {
                    return None;
                }
                self.ref_formal_redirect_inner(expr, true).map(|ne| {
                    Expression::new(
                        ExprKind::MemberAccess {
                            expr: Box::new(ne),
                            member: member.clone(),
                        },
                        e.span,
                    )
                })
            }
            _ => None,
        }
    }
}
