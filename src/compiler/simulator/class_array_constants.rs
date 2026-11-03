//! Class-scope constant arrays and nonparameterized static array initializers.

use super::*;

struct ClassArrayLayout {
    lo: i64,
    hi: i64,
    width: u32,
    signed: bool,
    two_state: bool,
    string: bool,
    real: bool,
    descending: bool,
    data_type: DataType,
}

impl ClassArrayLayout {
    fn default_value(&self) -> Value {
        let mut value = if self.string {
            Value::from_string("")
        } else if self.real {
            Value::from_f64(0.0)
        } else if self.two_state {
            Value::zero(self.width)
        } else {
            Value::new(self.width)
        };
        value.is_signed = self.signed;
        value
    }

    fn indices(&self) -> Vec<i64> {
        if self.descending {
            (self.lo..=self.hi).rev().collect()
        } else {
            (self.lo..=self.hi).collect()
        }
    }
}

impl Simulator {
    /// §8.25: retain a value-parameter pattern in an object's specialization
    /// signature, including literal sizes, signs, keys and replication.
    pub(super) fn class_array_arg_fragment(&self, expr: &Expression) -> Option<String> {
        match &expr.peel_typed_pattern().kind {
            ExprKind::AssignmentPattern(items) => {
                let parts: Option<Vec<String>> = items
                    .iter()
                    .map(|item| {
                        let value = self.class_array_arg_fragment(item.expr())?;
                        Some(match item {
                            AssignmentPatternItem::Ordered(Expression {
                                kind: ExprKind::Replication { count, exprs },
                                ..
                            }) => {
                                let parts: Option<Vec<_>> = exprs
                                    .iter()
                                    .map(|e| self.class_array_arg_fragment(e))
                                    .collect();
                                format!(
                                    "{}{{{}}}",
                                    self.class_array_arg_fragment(count)?,
                                    parts?.join(",")
                                )
                            }
                            AssignmentPatternItem::Ordered(_) => value,
                            AssignmentPatternItem::Default(_) => format!("default:{}", value),
                            AssignmentPatternItem::Named(name, _) => {
                                format!("{}:{}", name.name, value)
                            }
                            AssignmentPatternItem::Keyed(key, _) => {
                                format!("{}:{}", self.class_array_arg_fragment(key)?, value)
                            }
                            AssignmentPatternItem::Typed(dt, _) => format!(
                                "{}:{}",
                                super::elaborate::data_type_to_spec_fragment(dt)?,
                                value
                            ),
                        })
                    })
                    .collect();
                Some(format!("'{{{}}}", parts?.join(",")))
            }
            ExprKind::Number(NumberLiteral::Integer {
                size,
                signed,
                base,
                value,
                ..
            }) => {
                if size.is_none() && matches!(base, NumberBase::Decimal) {
                    return Some(value.clone());
                }
                let radix = match base {
                    NumberBase::Binary => 'b',
                    NumberBase::Octal => 'o',
                    NumberBase::Decimal => 'd',
                    NumberBase::Hex => 'h',
                };
                Some(format!(
                    "{}'{}{}{}",
                    size.map(|s| s.to_string()).unwrap_or_default(),
                    if *signed { "s" } else { "" },
                    radix,
                    value
                ))
            }
            ExprKind::Number(NumberLiteral::UnbasedUnsized(bit)) => Some(format!("'{}", bit)),
            ExprKind::Paren(inner) => Some(format!("({})", self.class_array_arg_fragment(inner)?)),
            ExprKind::Unary { op, operand } => {
                let op = match op {
                    UnaryOp::Plus => "+",
                    UnaryOp::Minus => "-",
                    UnaryOp::LogNot => "!",
                    UnaryOp::BitNot => "~",
                    UnaryOp::BitAnd => "&",
                    UnaryOp::BitNand => "~&",
                    UnaryOp::BitOr => "|",
                    UnaryOp::BitNor => "~|",
                    UnaryOp::BitXor => "^",
                    UnaryOp::BitXnor => "~^",
                    _ => return None,
                };
                Some(format!(
                    "({}{})",
                    op,
                    self.class_array_arg_fragment(operand)?
                ))
            }
            ExprKind::Binary { op, left, right } => {
                let op = match op {
                    BinaryOp::Add => "+",
                    BinaryOp::Sub => "-",
                    BinaryOp::Mul => "*",
                    BinaryOp::Div => "/",
                    BinaryOp::Mod => "%",
                    BinaryOp::Power => "**",
                    BinaryOp::Eq => "==",
                    BinaryOp::Neq => "!=",
                    BinaryOp::CaseEq => "===",
                    BinaryOp::CaseNeq => "!==",
                    BinaryOp::WildcardEq => "==?",
                    BinaryOp::WildcardNeq => "!=?",
                    BinaryOp::LogAnd => "&&",
                    BinaryOp::LogOr => "||",
                    BinaryOp::Lt => "<",
                    BinaryOp::Leq => "<=",
                    BinaryOp::Gt => ">",
                    BinaryOp::Geq => ">=",
                    BinaryOp::BitAnd => "&",
                    BinaryOp::BitOr => "|",
                    BinaryOp::BitXor => "^",
                    BinaryOp::BitXnor => "~^",
                    BinaryOp::ShiftLeft => "<<",
                    BinaryOp::ShiftRight => ">>",
                    BinaryOp::ArithShiftLeft => "<<<",
                    BinaryOp::ArithShiftRight => ">>>",
                    _ => return None,
                };
                Some(format!(
                    "({}{}{})",
                    self.class_array_arg_fragment(left)?,
                    op,
                    self.class_array_arg_fragment(right)?
                ))
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => Some(format!(
                "({}?{}:{})",
                self.class_array_arg_fragment(condition)?,
                self.class_array_arg_fragment(then_expr)?,
                self.class_array_arg_fragment(else_expr)?
            )),
            ExprKind::Concatenation(parts) => {
                let parts: Option<Vec<_>> = parts
                    .iter()
                    .map(|e| self.class_array_arg_fragment(e))
                    .collect();
                Some(format!("{{{}}}", parts?.join(",")))
            }
            ExprKind::Replication { count, exprs } => {
                let parts: Option<Vec<_>> = exprs
                    .iter()
                    .map(|e| self.class_array_arg_fragment(e))
                    .collect();
                Some(format!(
                    "{{{}{{{}}}}}",
                    self.class_array_arg_fragment(count)?,
                    parts?.join(",")
                ))
            }
            _ => self.expr_to_spec_fragment(expr),
        }
    }

    /// §8.25/§6.20.2 (issue #423): the element a `base[index]` read selects
    /// when `base` names a class-scope ARRAY constant — a header VALUE
    /// parameter declared with unpacked dimensions (`C#(..)::TBL[i]`), a
    /// class-body `localparam` array (`C#(..)::LP[i]`, bare `LP[i]` inside
    /// a static method of the specialization), or a STATIC array property
    /// (`pkg::Cls::ARR[i]`, bare `ARR[i]` in a static method). None when
    /// the base names nothing of the sort, so ordinary indexed reads pay
    /// only a flatten plus a prefix probe.
    pub(super) fn class_param_array_element(
        &mut self,
        base: &Expression,
        index: &Expression,
    ) -> Option<Value> {
        // Peel a `C#(args)` wrapper off the base first: the Index arm runs
        // before the base's own Specialization handling, which would
        // evaluate the base as a scalar and restore `current_spec`.
        let stripped;
        let (spec, base) = if Self::expr_has_specialization(base) {
            let saved = self.current_spec.clone();
            let sp = self.resolve_call_spec_params(Self::extract_call_spec(base), &saved);
            if let Some((ref b, ref s)) = sp {
                self.ensure_spec_statics(b, s);
            }
            stripped = Self::strip_spec_shape(base);
            (sp, &stripped)
        } else {
            (None, base)
        };
        let segs = Self::flatten_member_path(base)?;
        let n = segs.len();
        if n == 0 {
            return None;
        }
        let leaf = segs[n - 1].clone();
        // The class the prefix denotes — longest class part first, so a
        // nested `outer::inner::LP` prefers the scoped key while a
        // package-qualified `pkg::Cls::LP` falls through to `Cls`.
        let mut start: Option<(String, Option<String>)> = None;
        if n >= 2 {
            for k in 0..n - 1 {
                if let Some(r) = self.resolve_class_scope_prefix(&segs[k..n - 1].join("::")) {
                    start = Some(r);
                    break;
                }
            }
        }
        if start.is_none() && n == 1 {
            // Bare `LP[i]`: the enclosing specialization (a static method
            // of the specialization set it) or class context names the
            // scope. Locals and formals shadow class members; class members
            // in turn shadow a same-named module array or signal.
            if self.dyn_name_lookup(&leaf).is_some()
                || self.local_stack.last().is_some_and(|f| {
                    f.contains_key(&leaf) || f.contains_key(&format!("\u{1}array_formal:{leaf}"))
                })
            {
                return None;
            }
            if let Some((b, s)) = self.current_spec.clone() {
                start = Some((b, Some(s)));
            } else if let Some(cls) = self.class_context_stack.last().cloned().flatten() {
                start = Some((cls, None));
            }
        }
        let (start_class, resolved_sig) = start?;
        // §8.25: a typedef carries its specialization too. Keep the receiver
        // even for a default/nonparameterized derived class, so an inherited
        // constant follows its extends bindings rather than the base defaults.
        // A package qualification and its bare class spelling describe
        // one specialization. Normalize the extracted base before following
        // inheritance; serialized aliases do not share Arc identity.
        let spec = spec.map(|(base, sig)| {
            if start_class
                .rsplit_once("::")
                .is_some_and(|(_, leaf)| leaf == base)
            {
                (start_class.clone(), sig)
            } else {
                (base, sig)
            }
        });
        let spec = spec
            .or_else(|| resolved_sig.map(|s| (start_class.clone(), s)))
            .or_else(|| self.current_spec.clone().filter(|(b, _)| *b == start_class))
            .or_else(|| {
                Some((
                    start_class.clone(),
                    self.canonicalize_spec_sig(&start_class, ""),
                ))
            });
        // Walk the chain for the DECLARING class.
        let mut cur: Option<String> = Some(start_class);
        let mut guard = 0;
        while let Some(cn) = cur {
            guard += 1;
            if guard > 64 {
                return None;
            }
            // A class-scope ARRAY constant is an `array_properties` entry
            // that is either a const PROPERTY (the localparam-array
            // registration) or a header parameter (`param_order`) — an
            // ordinary instance array property is neither, and a static
            // fixed array property lives in `static_fixed_arrays` instead.
            let (param_arr, static_fixed, extends) = {
                let Some(cd) = self.module.classes.get(&cn) else {
                    return None;
                };
                (
                    cd.array_properties.contains_key(&leaf)
                        && (cd.param_order.iter().any(|p| *p == leaf)
                            || (cd.properties.get(&leaf).is_some_and(|s| s.is_const)
                                && !cd.property_order.contains(&leaf))),
                    cd.static_fixed_arrays.iter().any(|(nm, ..)| *nm == leaf),
                    cd.extends.clone(),
                )
            };
            if param_arr {
                let key = self.materialize_class_param_array(&cn, &leaf, spec.as_ref())?;
                let iv = self.eval_expr(index);
                let arr = self.class_param_arrays.get(&key)?;
                let (lo, hi) = (arr.lo, arr.hi);
                return match iv.to_i64().filter(|_| !iv.has_xz()) {
                    Some(i) if i >= lo && i <= hi => arr.elems.get((i - lo) as usize).cloned(),
                    _ => Some(arr.default.clone()),
                };
            }
            if static_fixed {
                // The 2-segment `Cls::S[i]` shape is read by the
                // static-fixed arm in the Index evaluator; this covers the
                // package-qualified and bare-in-method spellings, from the
                // same per-element cells.
                let key = format!("{}::{}", cn, leaf);
                let ew = self.module.arrays.get(&key).map(|t| t.2)?;
                let iv = self.eval_expr(index);
                return match iv.to_i64().filter(|_| !iv.has_xz()) {
                    Some(i) => Some(
                        self.get_signal_value_by_name(&format!("{}[{}]", key, i))
                            .unwrap_or_else(|| Value::new(ew.max(1))),
                    ),
                    None => Some(Value::new(ew.max(1))),
                };
            }
            // §8.14: a nearer field hides an inherited constant of the same
            // name. Leave that field to the normal instance read path.
            if self.module.classes.get(&cn).is_some_and(|cd| {
                cd.properties.contains_key(&leaf)
                    || cd.queue_properties.contains_key(&leaf)
                    || cd.assoc_properties.contains_key(&leaf)
            }) {
                return None;
            }
            cur = extends;
        }
        None
    }

    /// §6.20.2 / §8.25: resolve bounds and element types in the declaring
    /// specialization, rather than retaining its elaboration-time defaults.
    fn class_array_layout(&mut self, class: &str, leaf: &str) -> Option<ClassArrayLayout> {
        use crate::ast::types::UnpackedDimension as UD;
        let cd = self.module.classes.get(class)?.clone();
        let (mut lo, mut hi, fallback_width) =
            cd.array_properties.get(leaf).copied().or_else(|| {
                cd.static_fixed_arrays
                    .iter()
                    .find(|(n, ..)| n == leaf)
                    .map(|&(_, l, h, w)| (l, h, w))
            })?;
        let mut params = self.module.parameters.clone();
        for (name, _) in &cd.param_defaults {
            if !cd.array_properties.contains_key(name)
                && let Some(v) = self.resolve_value_param_in_current(name)
            {
                params.insert(name.clone(), v);
            }
        }
        let mut descending = cd
            .array_prop_decl_dims
            .get(leaf)
            .and_then(|d| d.first())
            .is_some_and(|&(l, r)| l > r);
        if let Some(dim) = cd.param_sized_props.get(leaf).and_then(|d| d.first()) {
            let bounds = match dim {
                UD::Range { left, right, .. } => {
                    let l = self.eval_expr(left);
                    let r = self.eval_expr(right);
                    if l.has_xz() || r.has_xz() {
                        None
                    } else {
                        l.to_i64().zip(r.to_i64())
                    }
                }
                UD::Expression { expr, .. } => {
                    let n = self.eval_expr(expr);
                    n.to_i64()
                        .filter(|&v| v > 0 && !n.has_xz())
                        .map(|n| (0, n - 1))
                }
                // The parser also represents the constant size [N] as a
                // type-keyed dimension. It is still a fixed constant array.
                UD::Associative {
                    data_type: Some(dt),
                    ..
                } => {
                    if let DataType::TypeReference { name, .. } = dt.as_ref() {
                        let scoped = name.qualified();
                        let value = if name.has_scope() {
                            params.get(&scoped).cloned()
                        } else {
                            self.resolve_value_param_in_current(&name.name.name)
                                .or_else(|| params.get(&name.name.name).cloned())
                        };
                        value
                            .filter(|v| !v.has_xz())
                            .and_then(|v| v.to_i64())
                            .filter(|&n| n > 0)
                            .map(|n| (0, n - 1))
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some((l, r)) = bounds {
                lo = l.min(r);
                hi = l.max(r);
                descending = l > r;
            }
        }
        let dt = self.class_prop_decl_type(&cd, leaf)?.clone();
        let dt = self
            .bind_type_params_in(&dt, &|n| self.resolve_type_param_binding(n))
            .unwrap_or(dt);
        let resolved = self.resolve_dt(&dt);
        let width = super::elaborate::resolve_type_width(
            &resolved,
            Some(&params),
            Some(&self.module.typedefs),
        );
        Some(ClassArrayLayout {
            lo,
            hi,
            width: if width > 0 {
                width
            } else {
                fallback_width.max(1)
            },
            signed: self.type_is_signed_concrete(&dt),
            two_state: super::elaborate::is_type_two_state_resolved(
                &dt,
                &self.module.typedef_types,
            ),
            string: matches!(
                resolved,
                DataType::Simple {
                    kind: crate::ast::types::SimpleType::String,
                    ..
                }
            ),
            real: matches!(resolved, DataType::Real { .. }),
            descending,
            data_type: resolved,
        })
    }

    /// §10.9.1 / §10.8: pattern elements are assigned in the context of the
    /// declared element type, including its width, signing and state domain.
    fn class_array_element_value(&mut self, expr: &Expression, layout: &ClassArrayLayout) -> Value {
        if layout.string {
            return self.eval_expr(expr);
        }
        if layout.real {
            return Value::from_f64(self.eval_expr(expr).to_f64());
        }
        let value = self.eval_expr_ctx(expr, layout.width);
        let mut value = self.fit_pattern_item_value(value, layout.width);
        if layout.two_state {
            value = value.to_two_state();
        }
        value.is_signed = layout.signed;
        value
    }

    pub(super) fn materialize_class_param_array(
        &mut self,
        class: &str,
        leaf: &str,
        spec: Option<&(String, String)>,
    ) -> Option<String> {
        let signature = match spec {
            Some((base, sig)) => {
                let sig = self.canonicalize_spec_sig(base, sig);
                if base == class {
                    sig
                } else {
                    self.ancestor_spec(base, &sig, class)?
                }
            }
            None => self.canonicalize_spec_sig(class, ""),
        };
        let key = format!("{}#{}::{}", class, signature, leaf);
        if self.class_param_arrays.contains_key(&key) {
            return Some(key);
        }
        let cd = self.module.classes.get(class)?.clone();
        let explicit = cd
            .param_order
            .iter()
            .position(|p| p == leaf)
            .and_then(|p| Self::spec_arg_at(&signature, p))
            .filter(|f| f.trim() != leaf)
            .and_then(Self::parse_spec_arg_expr);
        let init = explicit.or_else(|| cd.property_inits.get(leaf).cloned())?;
        let saved_spec = self.current_spec.replace((class.to_string(), signature));
        self.class_context_stack.push(Some(class.to_string()));
        self.this_stack.push(None);
        // §6.20.2: a constant is evaluated in its declaration's scope. The
        // caller's locals/formals must neither change nor poison this cache.
        let frame = self.take_pooled_frame();
        self.push_local_frame(frame);
        let saved_dyn = std::mem::take(&mut self.local_dyn);
        let saved_formal = std::mem::take(&mut self.formal_dyn);
        let built = self.build_class_param_array(class, leaf, &init, &key);
        self.local_dyn = saved_dyn;
        self.formal_dyn = saved_formal;
        self.pop_local_frame();
        self.this_stack.pop();
        self.class_context_stack.pop();
        self.current_spec = saved_spec;
        built.map(|_| key)
    }

    fn build_class_param_array(
        &mut self,
        class: &str,
        leaf: &str,
        init: &Expression,
        key: &str,
    ) -> Option<()> {
        let layout = self.class_array_layout(class, leaf)?;
        let ExprKind::AssignmentPattern(items) = &init.peel_typed_pattern().kind else {
            return None;
        };
        let count = usize::try_from(layout.hi.checked_sub(layout.lo)?.checked_add(1)?).ok()?;
        let default = layout.default_value();
        // Seed before evaluating element expressions so a malformed cyclic
        // declaration cannot recursively materialize itself without a bound.
        self.class_param_arrays.insert(
            key.to_string(),
            ClassParamArray {
                lo: layout.lo,
                hi: layout.hi,
                descending: layout.descending,
                default: default.clone(),
                elems: vec![default; count],
            },
        );
        for (index, value) in self.class_array_pattern_values(items, &layout) {
            self.class_param_arrays.get_mut(key)?.elems[(index - layout.lo) as usize] = value;
        }
        Some(())
    }

    /// §8.25: object-qualified and bare method reads of a parameter array
    /// use the same specialization's immutable values as class-qualified reads.
    pub(super) fn initialize_class_constant_array(
        &mut self,
        handle: usize,
        class: &str,
        leaf: &str,
    ) -> bool {
        let Some(cd) = self.module.classes.get(class) else {
            return false;
        };
        if !cd.array_properties.contains_key(leaf)
            || !(cd.param_order.iter().any(|name| name == leaf)
                || (cd.properties.get(leaf).is_some_and(|s| s.is_const)
                    && !cd.property_order.iter().any(|p| p == leaf)))
            || self
                .class_prop_struct(handle, leaf)
                .is_some_and(|s| Self::spreads_member_wise(&s))
        {
            return false;
        }
        let Some(inst) = self.heap.get(handle).and_then(|i| i.as_ref()) else {
            return false;
        };
        let spec = inst.spec.clone().unwrap_or_else(|| {
            (
                inst.class_name.clone(),
                self.canonicalize_spec_sig(&inst.class_name, ""),
            )
        });
        let Some(key) = self.materialize_class_param_array(class, leaf, Some(&spec)) else {
            return false;
        };
        let Some(arr) = self.class_param_arrays.get(&key) else {
            return false;
        };
        let (lo, hi, width, descending, elems) = (
            arr.lo,
            arr.hi,
            arr.default.width,
            arr.descending,
            arr.elems.clone(),
        );
        let scoped = format!("{}#{}", handle, leaf);
        self.module.arrays.insert(scoped.clone(), (lo, hi, width));
        if descending {
            self.module.descending_arrays.insert(scoped.clone());
        } else {
            self.module.descending_arrays.remove(&scoped);
        }
        self.widths.insert(scoped.clone(), width);
        for (offset, value) in elems.into_iter().enumerate() {
            let name = format!("{}[{}]", scoped, lo + offset as i64);
            self.widths.insert(name.clone(), width);
            self.signals.insert(name, value);
        }
        true
    }

    /// §8.9 / §10.9.1: fixed static arrays share their initialized cells;
    /// patterns use the same keys, directions and conversions as constants.
    pub(super) fn initialize_class_fixed_array(
        &mut self,
        class: &str,
        leaf: &str,
        init: &Expression,
    ) {
        let ExprKind::AssignmentPattern(items) = &init.peel_typed_pattern().kind else {
            return;
        };
        let signature = self.canonicalize_spec_sig(class, "");
        let saved_spec = self.current_spec.replace((class.to_string(), signature));
        self.class_context_stack.push(Some(class.to_string()));
        self.this_stack.push(None);
        if let Some(layout) = self.class_array_layout(class, leaf) {
            for (index, value) in self.class_array_pattern_values(items, &layout) {
                self.signals
                    .insert(format!("{}::{}[{}]", class, leaf, index), value);
            }
        }
        self.this_stack.pop();
        self.class_context_stack.pop();
        self.current_spec = saved_spec;
    }

    fn class_array_pattern_values(
        &mut self,
        items: &[AssignmentPatternItem],
        layout: &ClassArrayLayout,
    ) -> Vec<(i64, Value)> {
        let indices = layout.indices();
        // §10.9.1: an explicit index wins over a type key, which wins over
        // default. Reuse the common positional/key/default/replication binder.
        let typed = items.iter().find_map(|item| match item {
            AssignmentPatternItem::Typed(dt, value)
                if self.pattern_type_matches(&layout.data_type, dt) =>
            {
                Some(value)
            }
            _ => None,
        });
        let mut adjusted;
        let items = if let Some(value) = typed {
            adjusted = items
                .iter()
                .filter(|item| !matches!(item, AssignmentPatternItem::Default(_)))
                .cloned()
                .collect::<Vec<_>>();
            adjusted.push(AssignmentPatternItem::Default(value.clone()));
            adjusted.as_slice()
        } else {
            items
        };
        let elements = self.pattern_elems(items, &indices);
        indices
            .into_iter()
            .zip(elements)
            .filter_map(|(index, expr)| {
                expr.map(|expr| (index, self.class_array_element_value(expr, layout)))
            })
            .collect()
    }

    /// Parse a specialization-argument fragment into an `Expression` by
    /// wrapping it in a scratch `localparam` declaration (the same trick
    /// `param_default_fragment` uses). Needed for assignment-pattern
    /// arguments (`#(4, '{9, 8, 7, 6})`) whose element expressions the
    /// scalar fragment evaluator cannot spell.
    fn parse_spec_arg_expr(text: &str) -> Option<Expression> {
        use crate::ast::decl::{ModuleItem, ParameterKind};
        let src = format!(
            "module __xezim_spec_arg; localparam int __xezim_v [1] = {};\nendmodule\n",
            text
        );
        let tokens = crate::lexer::Lexer::new(&src).tokenize();
        let mut parser = sv_parser::parse::Parser::new(tokens);
        let parsed = parser.parse_source_text();
        if parser
            .diagnostics()
            .iter()
            .any(|d| d.severity == sv_parser::diagnostics::Severity::Error)
        {
            return None;
        }
        let crate::ast::Description::Module(m) = parsed.descriptions.into_iter().next()? else {
            return None;
        };
        m.items.into_iter().find_map(|it| match it {
            ModuleItem::LocalparamDeclaration(pd) => match pd.kind {
                ParameterKind::Data { assignments, .. } => {
                    assignments.into_iter().next().and_then(|a| a.init)
                }
                _ => None,
            },
            _ => None,
        })
    }
}
