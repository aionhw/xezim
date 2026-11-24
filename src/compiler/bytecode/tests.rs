//! Regression tests for bytecode compilation and lowering.
use super::*;

/// IEEE 1800-2023 §13.4.2: a bare callee with an instance-qualified twin
/// cannot borrow the top declaration's external names during inlining.
#[test]
fn function_scope_index_matches_suffix_guard_and_resets_with_table() {
    let (_, module) = xezim_core::parse_and_elaborate_multi(
        &["module top; function int calc(); calc = 1; endfunction endmodule".into()],
        Some("top"),
        &[],
        &[],
        &[],
    )
    .expect("elaborate function template");
    let fd = module.functions.get("calc").expect("calc declaration");
    let functions: HashMap<_, _> = [
        "calc",
        "u.calc",
        "u.nested.calc",
        "u.pkg::calc",
        "u.calc_suffix",
        "u.calc2",
    ]
    .into_iter()
    .map(|name| (name.to_string(), fd.clone()))
    .collect();
    let empty = HashMap::default();
    let index = FunctionScopeIndex::new(&functions);
    let empty_index = FunctionScopeIndex::new(&empty);
    assert_eq!(index.qualified_leaf_count(), 4);
    let signals = NameMap::default();
    let arrays = HashMap::default();
    let widths = HashMap::default();
    let mut compiler = BytecodeCompiler::new(&signals, &[], &[], &arrays, &widths);
    assert!(!compiler.callee_ext_reads_ok("calc"));
    assert!(compiler.callee_ext_reads_ok("u.calc"));
    compiler.set_functions(&functions);
    let names = ["", "calc", "calc2", "calc_suffix", "pkg::calc", "other", "u.calc"];
    let expected: Vec<_> = names
        .iter()
        .map(|name| compiler.callee_ext_reads_ok(name))
        .collect();
    assert_eq!(expected, [true, false, false, false, false, true, true]);
    compiler.set_function_scope_index(&index);
    for (name, expected) in names.into_iter().zip(expected) {
        assert_eq!(compiler.callee_ext_reads_ok(name), expected, "{name}");
    }
    compiler.set_functions(&empty);
    assert!(compiler.function_scope_index.is_none());
    assert!(compiler.callee_ext_reads_ok("calc"));
    compiler.set_function_scope_index(&index);
    assert!(!compiler.callee_ext_reads_ok("calc"));
    compiler.set_function_scope_index(&empty_index);
    assert!(compiler.callee_ext_reads_ok("calc"));
}

#[test]
fn function_scope_index_preserves_method_fact_cache() {
    let (_, module) = xezim_core::parse_and_elaborate_multi(
        &["module top; function int calc(); calc = 1; endfunction endmodule".into()],
        Some("top"),
        &[],
        &[],
        &[],
    )
    .expect("elaborate function template");
    let fd = module.functions.get("calc").expect("calc declaration");
    let functions: HashMap<_, _> = [("calc".into(), fd.clone()), ("u.calc".into(), fd.clone())]
        .into_iter()
        .collect();
    let index = FunctionScopeIndex::new(&functions);
    let signals = NameMap::default();
    let arrays = HashMap::default();
    let widths = HashMap::default();
    let mut compiler = BytecodeCompiler::new(&signals, &[], &[], &arrays, &widths);
    let admission = std::rc::Rc::new(MethodAdmission::default());
    compiler.method_admission = Some(admission.clone());
    compiler.set_function_scope_index(&index);
    assert!(!compiler.callee_ext_reads_ok("calc"));
    assert!(compiler.callee_ext_reads_ok("other"));
    assert!(compiler.callee_ext_reads_ok("u.calc"));
    let facts = admission.fn_facts.ext_reads_ok.borrow();
    assert_eq!(facts.get("calc"), Some(&false));
    assert_eq!(facts.get("other"), Some(&true));
    assert!(!facts.contains_key("u.calc"));
}

#[test]
fn concrete_typedef_kinds_keep_scopes_and_runtime_type_parameters() {
    use crate::ast::types::{DataType as D, IntegerAtomType as A, TypeName};
    let span = crate::ast::Span::dummy();
    let alias = |name: &str| D::TypeReference {
        name: TypeName {
            scopes: vec![],
            name: crate::ast::Identifier {
                name: name.into(),
                span,
            },
            span,
            bound: Default::default(),
        },
        dimensions: vec![],
        type_args: vec![],
        span,
    };
    let atom = |kind| D::IntegerAtom {
        kind,
        signing: None,
        span,
    };
    let signals = NameMap::default();
    let arrays = HashMap::default();
    let widths = HashMap::default();
    let types: HashMap<_, _> = [
        ("word".into(), atom(A::Int)),
        ("inst.word".into(), atom(A::Byte)),
        ("inst.chain".into(), alias("word")),
        ("T".into(), atom(A::Int)),
    ]
    .into_iter()
    .collect();
    let mut compiler = BytecodeCompiler::new(&signals, &[], &[], &arrays, &widths);
    compiler.set_typedef_types(&types);
    assert_eq!(
        compiler.local_kind_of(&alias("chain"), Some("inst.f")),
        Some(LocalKind::Int {
            width: 8,
            signed: true,
            two_state: true
        })
    );
    assert!(
        !compiler
            .type_default_value_in(&alias("chain"), 8, Some("inst.f"))
            .has_xz()
    );
    assert!(
        compiler
            .type_default_value_in(&alias("chain"), 8, Some("inst.f"))
            .is_signed
    );
    compiler.method_class = Some("C".into());
    compiler.set_local_typedef_types(
        [
            ("C::word".into(), atom(A::LongInt)),
            ("word".into(), atom(A::LongInt)),
        ]
        .into_iter()
        .collect(),
    );
    assert_eq!(compiler.decl_width(&alias("word")), 64);
    assert_eq!(compiler.decl_width_in(&alias("word"), Some("f")), 32);
    assert_eq!(compiler.decl_width_in(&alias("word"), Some("inst.f")), 8);
    compiler.set_unresolved_type_names(["T".into()].into_iter().collect());
    assert_eq!(compiler.local_kind_of(&alias("T"), None), None);
    assert_eq!(
        compiler.local_kind_of(&alias("T"), Some("inst.f")),
        Some(LocalKind::Int {
            width: 32,
            signed: true,
            two_state: true
        })
    );
    assert_eq!(compiler.local_kind_of(&alias("unknown_class"), None), None);
    compiler.local_typedef_types.insert(
        "C::dynamic".into(),
        D::IntegerVector {
            kind: crate::ast::types::IntegerVectorType::Logic,
            signing: None,
            dimensions: vec![crate::ast::types::PackedDimension::Range {
                left: Box::new(ident_expr("W")),
                right: Box::new(Expression::new(
                    ExprKind::Number(NumberLiteral::UnbasedUnsized('0')),
                    span,
                )),
                span,
            }],
            span,
        },
    );
    assert!(compiler.class_alias_dynamic_width(&alias("dynamic")));
    assert!(!compiler.class_alias_dynamic_width(&alias("word")));
}

#[test]
fn unsupported_selected_frame_target_never_resolves_module_homonym() {
    let mut signals = NameMap::default();
    signals.insert(Arc::from("k"), 0);
    let arrays = HashMap::default();
    let widths = HashMap::default();
    let mut compiler = BytecodeCompiler::new(&signals, &[false], &[8], &arrays, &widths);
    let reg = compiler.alloc_reg();
    let value = compiler.alloc_reg();
    compiler.local_var_regs.insert("k".into(), (reg, 8));
    let lhs = indexed_expr("k", '0');
    // No LocalKind: an unsupported declaration must decline, not store
    // to module k. The same holds for a deferred write to this local.
    assert!(!compiler.compile_blocking_target(&lhs, value, 1));
    assert!(!compiler.compile_nba_target(&lhs, value, 1));
    assert!(compiler.insns.is_empty());
    compiler.local_kinds.insert(
        reg,
        LocalKind::Int {
            width: 8,
            signed: false,
            two_state: false,
        },
    );
    assert!(compiler.compile_blocking_target(&lhs, value, 1));
    assert!(!compiler.insns.iter().any(|i| matches!(
        i,
        Insn::BlockingAssign(..)
            | Insn::BlockingAssignBitDyn(..)
            | Insn::BlockingAssignRange(..)
            | Insn::BlockingAssignRangeDyn(..)
    )));
}

#[test]
fn constant_branch_fusion_preserves_kinds_and_constants() {
    for kind in [BinOpConstKind::Eq, BinOpConstKind::CaseEq] {
        for k in [
            Value::from_u64(3, 8),
            Value::all_x(8),
            Value::all_z(8),
            Value::from_u128(1 << 80, 96),
        ] {
            let mut insns = vec![
                Insn::BinOpConst(2, 0, Box::new(k.clone()), kind),
                Insn::Nop,
                Insn::BranchIfFalse(2, 4),
                Insn::Move(5, 0),
            ];
            BytecodeCompiler::fuse_cmp_branch_move_resize(&mut insns);
            let Insn::CmpBranchConst(fused_kind, src, fused_k, tmp, target) = &insns[0] else {
                panic!("constant equality was not fused: {:?}", insns[0]);
            };
            assert_eq!(*fused_kind, kind);
            assert_eq!((*src, *tmp, *target), (0, 2, 4));
            assert_eq!(**fused_k, k);
            assert!(matches!(insns[2], Insn::Nop));
        }
    }
}

#[test]
fn constant_branch_fusion_preserves_live_results_and_branch_entries() {
    let compare =
        || Insn::BinOpConst(2, 0, Box::new(Value::from_u64(3, 8)), BinOpConstKind::Eq);
    let streams = [
        // The true path replaces the result; the false path still reads it.
        vec![
            compare(),
            Insn::BranchIfFalse(2, 4),
            Insn::LoadConst(2, Box::new(Value::zero(8))),
            Insn::Jump(5),
            Insn::Move(5, 2),
        ],
        // A backward edge reaches a read before the comparison redefines it.
        vec![Insn::Move(5, 2), compare(), Insn::BranchIfFalse(2, 0)],
        // Another branch enters the pair at its second instruction.
        vec![Insn::Jump(2), compare(), Insn::BranchIfFalse(2, 3)],
        // Opaque escapes may observe a register-backed local.
        vec![
            compare(),
            Insn::BranchIfFalse(2, 3),
            carrying_fallback(Some(2)),
        ],
    ];
    for mut insns in streams {
        BytecodeCompiler::fuse_cmp_branch_move_resize(&mut insns);
        assert!(!insns.iter().any(|i| matches!(i, Insn::CmpBranchConst(..))));
    }
}

#[test]
fn constant_branch_targets_and_read_operands_survive_rewrites() {
    let mut branch = Insn::CmpBranchConst(
        BinOpConstKind::CaseEq,
        1,
        Box::new(Value::from_u64(3, 8)),
        2,
        4,
    );
    assert!(branch.offset_regs_and_targets(5, 7));
    assert!(matches!(branch, Insn::CmpBranchConst(_, 6, _, 7, 11)));
    assert!(BytecodeCompiler::insn_reads_reg(&branch, 6));
    assert!(!BytecodeCompiler::insn_reads_reg(&branch, 7));
    assert_eq!(BytecodeCompiler::dest_reg(&branch), None);
    assert!(BytecodeCompiler::replace_read_reg(&mut branch, 6, 9));
    assert!(matches!(branch, Insn::CmpBranchConst(_, 9, _, 7, 11)));

    let mut insns = vec![
        Insn::CmpBranchConst(BinOpConstKind::Eq, 0, Box::new(Value::zero(8)), 1, 4),
        Insn::Nop,
        Insn::Move(3, 0),
        Insn::Nop,
    ];
    assert!(BytecodeCompiler::branch_target_map(&insns)[4]);
    BytecodeCompiler::compact_nops(&mut insns);
    assert!(matches!(insns[0], Insn::CmpBranchConst(_, _, _, _, 2)));
    let mut live = RegLiveness::new(insns.len());
    assert!(live.read_before_write(&insns, &[0], 0));
    let back = [Insn::CmpBranchConst(
        BinOpConstKind::Eq,
        0,
        Box::new(Value::zero(8)),
        1,
        0,
    )];
    assert!(BytecodeCompiler::has_backward_branch(&back));
}

#[test]
fn constant_branch_lowering_preserves_acceleration_and_case_equality_guard() {
    for kind in [BinOpConstKind::Eq, BinOpConstKind::CaseEq] {
        let cb = CompiledBlock {
            instructions: vec![
                Insn::LoadSignal(0, 0),
                Insn::CmpBranchConst(kind, 0, Box::new(Value::from_u64(3, 8)), 1, 3),
                Insn::NbaAssign(1, 0, 8),
            ],
            num_regs: 2,
            has_fallback: false,
            nba_dup_targets: false,
            foreach_slots: 0,
            static_locals: Box::new([]),
        };
        let ts = lower_two_state(
            &cb,
            &[8, 8],
            &[false, false],
            &[false, false],
            &HashMap::default(),
        )
        .expect("constant comparison remains two-state lowerable");
        assert!(ts.has_ctrl);
        assert!(matches!(
            ts.insns[1],
            TsInsn::EqCBrFalse {
                d: 1,
                s: 0,
                k: 3,
                t: 3
            }
        ));
        assert_eq!(ts.insns.len(), 3);
        assert_eq!(ts.tsx, kind == BinOpConstKind::Eq);
        // The existing signed widening, X/Z, fill, and wide-constant
        // admission rules must also govern the fused form.
        for k in [
            Value::all_x(8),
            Value::all_z(8),
            Value::from_u128(1 << 80, 96),
        ] {
            let mut cb = cb.clone();
            cb.instructions[1] = Insn::CmpBranchConst(kind, 0, Box::new(k), 1, 3);
            assert!(
                lower_two_state(
                    &cb,
                    &[8, 8],
                    &[false, false],
                    &[false, false],
                    &HashMap::default()
                )
                .is_none()
            );
        }
        let mut cb = cb.clone();
        let mut k = Value::from_u64(3, 16);
        k.is_signed = true;
        cb.instructions[1] = Insn::CmpBranchConst(kind, 0, Box::new(k), 1, 3);
        assert!(
            lower_two_state(
                &cb,
                &[8, 8],
                &[true, false],
                &[false, false],
                &HashMap::default()
            )
            .is_none()
        );
    }
}

#[test]
fn constant_two_state_branch_fusion_preserves_registers_and_entries() {
    for dest in [0, 1] {
        let mut stream = vec![
            TsInsn::EqC {
                d: dest,
                s: 0,
                k: u64::MAX,
            },
            TsInsn::BrFalse { s: dest, t: 3 },
            TsInsn::Const { d: 2, v: 19 },
        ];
        fuse_ts_pairs(&mut stream);
        assert!(matches!(stream[0], TsInsn::EqCBrFalse {
            d, s: 0, k: u64::MAX, t: 2
        } if d == dest));
        assert_eq!(stream.len(), 2);
        assert!(tsx_insn_ok(&stream[0]));
    }
    for condition in [0, 1] {
        let mut stream = vec![
            TsInsn::Jmp { t: 2 },
            TsInsn::EqC { d: 1, s: 0, k: 3 },
            TsInsn::BrFalse { s: condition, t: 3 },
        ];
        fuse_ts_pairs(&mut stream);
        assert_eq!(stream.len(), 3, "never erase an incoming branch entry");
    }
    let mut mismatched = vec![
        TsInsn::EqC { d: 1, s: 0, k: 3 },
        TsInsn::BrFalse { s: 0, t: 2 },
    ];
    fuse_ts_pairs(&mut mismatched);
    assert_eq!(mismatched.len(), 2, "branch must consume the comparison");
}

#[test]
fn constant_two_state_branch_fusion_relocates_tables_backedges_and_exit() {
    let mut stream = vec![
        TsInsn::CaseJmp {
            s: 0,
            cj: Box::new(CaseJumpData {
                table: vec![1, 3, 5],
                default: 6,
            }),
        },
        TsInsn::EqC { d: 1, s: 0, k: 3 },
        TsInsn::BrFalse { s: 1, t: 6 },
        TsInsn::Const { d: 2, v: 19 },
        TsInsn::Jmp { t: 1 },
        TsInsn::Const { d: 2, v: 23 },
    ];
    fuse_ts_pairs(&mut stream);
    assert_eq!(stream.len(), 5);
    let TsInsn::CaseJmp { cj, .. } = &stream[0] else {
        panic!("case dispatch")
    };
    assert_eq!(cj.table, [1, 2, 4]);
    assert_eq!(cj.default, 5);
    assert!(matches!(stream[1], TsInsn::EqCBrFalse { t: 5, .. }));
    assert!(matches!(stream[3], TsInsn::Jmp { t: 1 }));
}

/// §12.5: relocating a case preserves holes, duplicate targets and block exits.
#[test]
fn case_jump_relocation_adjusts_selector_and_all_targets() {
    let original = Insn::CaseJump(
        2,
        Box::new(CaseJumpData {
            table: vec![1, 5, 1],
            default: 5,
        }),
    );
    let mut relocated = original.clone();
    assert!(relocated.offset_regs_and_targets(7, 11));
    let Insn::CaseJump(selector, jump) = relocated else {
        panic!("case jump")
    };
    assert_eq!(selector, 9);
    assert_eq!(jump.table, vec![12, 16, 12]);
    assert_eq!(jump.default, 16);
    let Insn::CaseJump(selector, jump) = original else {
        panic!("original case jump")
    };
    assert_eq!(selector, 2);
    assert_eq!(jump.table, vec![1, 5, 1]);
    assert_eq!(jump.default, 5);
}

/// §12.5: each case arm's temporary definition has its own lifetime.
#[test]
fn comparison_fusion_accepts_recycled_temporaries() {
    let mut insns = vec![
        Insn::CaseEq(3, 0, 1),
        Insn::BranchIfFalse(3, 4),
        Insn::Move(5, 0),
        Insn::Jump(7),
        Insn::CaseEq(3, 0, 2),
        Insn::BranchIfFalse(3, 7),
        Insn::Move(5, 1),
    ];
    BytecodeCompiler::fuse_cmp_branch_move_resize(&mut insns);
    assert!(matches!(
        insns[0],
        Insn::CmpBranch(CmpKind::CaseEq, 0, 1, 3, 4)
    ));
    assert!(matches!(
        insns[4],
        Insn::CmpBranch(CmpKind::CaseEq, 0, 2, 3, 7)
    ));
    assert!(matches!(insns[1], Insn::Nop));
    assert!(matches!(insns[5], Insn::Nop));
}

/// §12.4: an untaken arm can bypass a definition and keep the old value live.
#[test]
fn comparison_fusion_keeps_a_value_read_on_the_other_branch() {
    let mut insns = vec![
        Insn::CaseEq(3, 0, 1),
        Insn::BranchIfFalse(3, 4),
        Insn::LoadConst(3, Box::new(Value::from_u64(0, 1))),
        Insn::Jump(5),
        Insn::Move(5, 3),
    ];
    BytecodeCompiler::fuse_cmp_branch_move_resize(&mut insns);
    assert!(matches!(insns[0], Insn::CaseEq(3, 0, 1)));
    assert!(matches!(insns[1], Insn::BranchIfFalse(3, 4)));
}

/// §12.5: loads in later case arms replace the earlier constant definition.
#[test]
fn constant_fusion_accepts_recycled_temporaries() {
    let mut insns = vec![
        Insn::LoadConst(2, Box::new(Value::from_u64(1, 8))),
        Insn::CaseEq(3, 0, 2),
        Insn::BranchIfFalse(3, 4),
        Insn::Jump(7),
        Insn::LoadConst(2, Box::new(Value::from_u64(2, 8))),
        Insn::CaseEq(3, 0, 2),
        Insn::Move(5, 3),
    ];
    BytecodeCompiler::fuse_binop_const(&mut insns);
    assert!(matches!(insns[0], Insn::Nop));
    assert!(matches!(insns[4], Insn::Nop));
    assert!(matches!(
        insns[1],
        Insn::BinOpConst(3, 0, _, BinOpConstKind::CaseEq)
    ));
    assert!(matches!(
        insns[5],
        Insn::BinOpConst(3, 0, _, BinOpConstKind::CaseEq)
    ));
}

/// §12.4: a branch can bypass the next textual constant definition.
#[test]
fn constant_fusion_keeps_a_value_read_on_the_other_branch() {
    let mut insns = vec![
        Insn::LoadConst(2, Box::new(Value::from_u64(1, 8))),
        Insn::CaseEq(3, 0, 2),
        Insn::BranchIfFalse(3, 5),
        Insn::LoadConst(2, Box::new(Value::from_u64(2, 8))),
        Insn::Jump(6),
        Insn::Move(5, 2),
    ];
    BytecodeCompiler::fuse_binop_const(&mut insns);
    assert!(matches!(insns[0], Insn::LoadConst(2, _)));
    assert!(matches!(insns[1], Insn::CaseEq(3, 0, 2)));
}

/// §12.7: liveness follows loop back-edges and reads before in-place writes.
#[test]
fn definition_liveness_handles_loops_in_place_reads_and_fallbacks() {
    let streams = [
        vec![Insn::Move(5, 2), Insn::Jump(0)],
        vec![Insn::Add(2, 2, 1)],
        vec![carrying_fallback(Some(2))],
    ];
    for insns in streams {
        let mut live = RegLiveness::new(insns.len());
        assert!(live.read_before_write(&insns, &[insns.len() - 1], 2));
    }
    let insns = vec![Insn::Jump(0)];
    let mut live = RegLiveness::new(insns.len());
    assert!(!live.read_before_write(&insns, &[0], 2));
    // Epoch reuse must not carry an earlier query's visited nodes forward.
    assert!(!live.read_before_write(&insns, &[0], 3));
    live.epoch = u32::MAX;
    assert!(!live.read_before_write(&insns, &[0], 2));
}

/// §12.5: every dispatch successor must kill the definition before reading it.
#[test]
fn definition_liveness_checks_all_dispatch_successors() {
    let mut insns = vec![
        Insn::CaseJump(
            0,
            Box::new(CaseJumpData {
                table: vec![1, 3],
                default: 5,
            }),
        ),
        Insn::LoadConst(2, Box::new(Value::from_u64(0, 8))),
        Insn::Jump(6),
        Insn::Move(5, 2),
        Insn::Jump(6),
        Insn::LoadConst(2, Box::new(Value::from_u64(0, 8))),
    ];
    let mut live = RegLiveness::new(insns.len());
    assert!(live.read_before_write(&insns, &[0], 2));
    insns[3] = Insn::LoadConst(2, Box::new(Value::from_u64(0, 8)));
    assert!(!live.read_before_write(&insns, &[0], 2));
    // Long paths retain the original instruction conservatively rather
    // than making compilation quadratic or overlooking a distant use.
    let mut insns = vec![Insn::Nop; 1025];
    insns.push(Insn::Move(5, 2));
    let mut live = RegLiveness::new(insns.len());
    assert!(live.read_before_write(&insns, &[0], 2));
}

#[test]
fn constant_fusion_preserves_the_consumers_replacement_definition() {
    let mut insns = vec![
        Insn::LoadConst(2, Box::new(Value::from_u64(3, 8))),
        Insn::Add(2, 0, 2),
        Insn::Move(5, 2),
    ];
    BytecodeCompiler::fuse_binop_const(&mut insns);
    assert!(matches!(insns[0], Insn::Nop));
    assert!(matches!(
        insns[1],
        Insn::BinOpConst(2, 0, _, BinOpConstKind::Add)
    ));
    assert!(matches!(insns[2], Insn::Move(5, 2)));
}

/// §12.5: short dense tables preserve first-match dispatch; sparse cases stay chains.
#[test]
fn case_jump_admits_short_dense_tables() {
    let span = crate::ast::Span::dummy();
    for (count, stride, expected) in [
        (3, 1, false),
        (4, 1, true),
        (7, 1, true),
        (7, 100, false),
        (8, 100, true),
    ] {
        let mut signals = NameMap::default();
        signals.insert(Arc::from("selector"), 0);
        let arrays = HashMap::default();
        let widths = HashMap::default();
        let mut compiler = BytecodeCompiler::new(&signals, &[false], &[16], &arrays, &widths);
        let items: Vec<_> = (0..count)
            .map(|index| crate::ast::stmt::CaseItem {
                patterns: vec![Expression::new(
                    ExprKind::Number(NumberLiteral::Integer {
                        size: Some(16),
                        signed: false,
                        base: NumberBase::Decimal,
                        value: (index * stride).to_string(),
                        cached_val: crate::ast::cache::LiteralCache::new(None),
                    }),
                    span,
                )],
                is_default: false,
                stmt: Statement::new(StatementKind::Null, span),
                span,
                pattern: None,
                guard: None,
            })
            .collect();
        assert_eq!(
            compiler.compile_case_jump(&ident_expr("selector"), &items),
            expected
        );
        assert_eq!(
            compiler
                .insns
                .iter()
                .any(|insn| matches!(insn, Insn::CaseJump(..))),
            expected
        );
    }
}

/// The read counts the peephole passes use must answer exactly what a
/// scan of the block with `insn_reads_reg` answers, before and after
/// the edits they track.
#[test]
fn reg_readers_match_a_full_scan() {
    let k = || Box::new(Value::from_u64(3, 8));
    let fallback = || {
        Insn::EvalExprFallback(
            Box::new((
                Arc::new(ident_expr("x")),
                Arc::from(""),
                FbLocals::default(),
            )),
            9,
            8,
        )
    };
    let mut insns = vec![
        Insn::LoadConst(1, k()),
        Insn::Add(2, 1, 1),
        Insn::Select(3, 2, 1, 4),
        Insn::Concat(5, Box::new(vec![1, 3, 3])),
        Insn::BranchIfFalse(5, 0),
        Insn::Move(6, 2),
        Insn::Nop,
    ];
    let scan = |insns: &[Insn], r: RegId, skip: &[usize]| {
        insns
            .iter()
            .enumerate()
            .any(|(j, x)| !skip.contains(&j) && BytecodeCompiler::insn_reads_reg(x, r))
    };
    let skips: [&[usize]; 5] = [&[], &[0], &[1], &[1, 2], &[2, 3, 5]];
    let check = |insns: &[Insn], rr: &RegReaders| {
        for r in 0..12 {
            for skip in skips {
                assert_eq!(
                    rr.read_outside(insns, r, skip),
                    scan(insns, r, skip),
                    "r={r} skip={skip:?}"
                );
            }
        }
    };
    let mut rr = RegReaders::new(&insns);
    check(&insns, &rr);
    // Edit the way the passes do: remove the old form, add the new one.
    rr.remove(&insns[1]);
    insns[1] = Insn::BinOpConst(2, 4, k(), BinOpConstKind::Add);
    rr.add(&insns[1]);
    check(&insns, &rr);
    // A fallback reads every register.
    rr.remove(&insns[6]);
    insns[6] = fallback();
    rr.add(&insns[6]);
    check(&insns, &rr);
    rr.remove(&insns[6]);
    insns[6] = Insn::Nop;
    check(&insns, &rr);
}

fn ident_expr(name: &str) -> Expression {
    let span = crate::ast::Span::dummy();
    Expression::new(
        ExprKind::Ident(HierarchicalIdentifier {
            root: None,
            path: vec![HierPathSegment {
                name: crate::ast::Identifier {
                    name: name.to_owned(),
                    span,
                },
                selects: Vec::new(),
            }],
            span,
            cached_signal_id: crate::ast::cache::SignalCache::new(None),
            cached_resolved_name: crate::ast::cache::NameCache::new(),
            bound: crate::ast::binding::Bound::UNSET,
        }),
        span,
    )
}

fn indexed_expr(name: &str, index: char) -> Expression {
    let span = crate::ast::Span::dummy();
    Expression::new(
        ExprKind::Index {
            expr: Box::new(ident_expr(name)),
            index: Box::new(Expression::new(
                ExprKind::Number(NumberLiteral::UnbasedUnsized(index)),
                span,
            )),
        },
        span,
    )
}

fn nested_indexed_expr(name: &str, outer: char, inner: char) -> Expression {
    let span = crate::ast::Span::dummy();
    Expression::new(
        ExprKind::Index {
            expr: Box::new(indexed_expr(name, outer)),
            index: Box::new(Expression::new(
                ExprKind::Number(NumberLiteral::UnbasedUnsized(inner)),
                span,
            )),
        },
        span,
    )
}

#[test]
fn generated_nonzero_outer_index_resolves_to_flattened_signal() {
    let mut signals = NameMap::default();
    signals.insert(Arc::from("flat"), 0);
    let arrays: HashMap<String, (i64, i64, u32)> = HashMap::default();
    let widths: HashMap<String, u32> = HashMap::default();
    let compiler = BytecodeCompiler::new(&signals, &[false], &[160], &arrays, &widths);

    assert_eq!(
        compiler.flattened_outer_const_signal_id(&indexed_expr("flat", '1')),
        Some(0)
    );
}

#[test]
fn genuine_array_shapes_do_not_resolve_as_flattened_signals() {
    let mut signals = NameMap::default();
    signals.insert(Arc::from("flat"), 0);
    let widths: HashMap<String, u32> = HashMap::default();
    let expr = indexed_expr("flat", '1');

    let mut arrays: HashMap<String, (i64, i64, u32)> = HashMap::default();
    arrays.insert("flat".to_owned(), (0, 3, 160));
    let compiler = BytecodeCompiler::new(&signals, &[false], &[160], &arrays, &widths);
    assert_eq!(compiler.flattened_outer_const_signal_id(&expr), None);

    let arrays: HashMap<String, (i64, i64, u32)> = HashMap::default();
    let mut packed_elem_widths: xezim_core::shared_names::SharedNameMap<u32> = Default::default();
    packed_elem_widths.insert("flat".to_owned(), 32);
    let mut compiler = BytecodeCompiler::new(&signals, &[false], &[160], &arrays, &widths);
    compiler.set_packed_elem_widths(&packed_elem_widths);
    assert_eq!(compiler.flattened_outer_const_signal_id(&expr), None);

    let mut multi_dim_arrays: HashSet<String> = HashSet::default();
    multi_dim_arrays.insert("flat".to_owned());
    let mut compiler = BytecodeCompiler::new(&signals, &[false], &[160], &arrays, &widths);
    compiler.set_multi_dim_arrays(&multi_dim_arrays);
    assert_eq!(compiler.flattened_outer_const_signal_id(&expr), None);
}

#[test]
fn constant_multi_dim_array_element_uses_scalar_bytecode() {
    let mut signals = NameMap::default();
    signals.insert(Arc::from("m[1][0]"), 0);
    let arrays: HashMap<String, (i64, i64, u32)> = HashMap::default();
    let widths: HashMap<String, u32> = HashMap::default();
    let mut multi_dim_arrays: HashSet<String> = HashSet::default();
    multi_dim_arrays.insert("m".to_owned());
    let lhs = nested_indexed_expr("m", '1', '0');

    let mut compiler = BytecodeCompiler::new(&signals, &[false], &[8], &arrays, &widths);
    compiler.set_multi_dim_arrays(&multi_dim_arrays);
    assert_eq!(compiler.const_multi_dim_array_elem_signal_id(&lhs), Some(0));
    assert!(compiler.compile_nba_target(&lhs, 0, 8));
    let block = compiler.finish();
    assert!(matches!(
        block.instructions.as_slice(),
        [Insn::NbaAssign(0, 0, 8)]
    ));

    let mut compiler = BytecodeCompiler::new(&signals, &[false], &[8], &arrays, &widths);
    compiler.set_multi_dim_arrays(&multi_dim_arrays);
    assert!(compiler.compile_expr(&lhs, 0).is_some());
    let block = compiler.finish();
    assert!(matches!(
        block.instructions.as_slice(),
        [Insn::LoadSignal(0, 0)]
    ));
}

/// §9.4.1 / §22.7: a root expression's result is read after the block
/// runs, so the peepholes must not drop it. `1 * 2.0` is the form a
/// bare delay takes when the module's time unit is not the global tick
/// (`#1` under `timescale 1ns/100ps` becomes `1 * 10.0`); constant
/// folding reduced it to a `LoadConst` nothing in the block read, which
/// was then deleted — the fast `always #d` path read a stale register
/// and looped at zero delay.
#[test]
fn root_expr_result_survives_constant_folding() {
    let signals = NameMap::default();
    let arrays: HashMap<String, (i64, i64, u32)> = HashMap::default();
    let widths: HashMap<String, u32> = HashMap::default();
    let sp = crate::ast::Span::dummy();
    let one = Expression::new(
        ExprKind::Number(NumberLiteral::Integer {
            size: None,
            signed: true,
            base: crate::ast::expr::NumberBase::Decimal,
            value: "1".into(),
            cached_val: crate::ast::cache::LiteralCache::new(None),
        }),
        sp,
    );
    let two = Expression::new(ExprKind::Number(NumberLiteral::Real(2.0)), sp);
    let expr = Expression::new(
        ExprKind::Binary {
            op: crate::ast::expr::BinaryOp::Mul,
            left: Box::new(one),
            right: Box::new(two),
        },
        sp,
    );
    let mut compiler = BytecodeCompiler::new(&signals, &[], &[], &arrays, &widths);
    let result = compiler.compile_root_expr(&expr).expect("compiles");
    let block = compiler.finish();
    assert!(
        block.instructions.iter().any(|i| matches!(
            i,
            Insn::LoadConst(r, v) if *r == result && v.is_real && v.to_f64() == 2.0
        )),
        "result register {result} not written: {:?}",
        block.instructions
    );
}

#[test]
fn register_ids_do_not_wrap_at_u16_limit() {
    let signals = NameMap::default();
    let arrays: HashMap<String, (i64, i64, u32)> = HashMap::default();
    let widths: HashMap<String, u32> = HashMap::default();
    let mut compiler = BytecodeCompiler::new(&signals, &[], &[], &arrays, &widths);

    let mut last = 0;
    for _ in 0..=u16::MAX {
        last = compiler.alloc_reg();
    }
    compiler.emit(Insn::LoadConst(last, Box::new(Value::zero(1))));

    let block = compiler.finish();
    assert_eq!(last, u16::MAX);
    assert_eq!(block.num_regs, u16::MAX as u32 + 1);
    assert!(matches!(
        block.instructions.last(),
        Some(Insn::LoadConst(reg, _)) if u32::from(*reg) < block.num_regs
    ));

    // The next temporary is the first ID that cannot be represented by
    // the compact instruction encoding. It must request fallback instead
    // of wrapping to register zero as it did in BUG_REPORT.md.
    let mut compiler = BytecodeCompiler::new(&signals, &[], &[], &arrays, &widths);
    for _ in 0..=u16::MAX {
        compiler.alloc_reg();
    }
    let expr = Expression::new(
        ExprKind::Number(NumberLiteral::UnbasedUnsized('0')),
        crate::ast::Span::dummy(),
    );
    assert_eq!(compiler.compile_root_expr(&expr), None);
    assert!(compiler.register_overflow);
    assert_eq!(compiler.next_reg, u16::MAX as u32 + 1);
}

#[test]
fn redundant_resizes_become_nops_and_narrowing_ones_survive() {
    // Signal 0 is 8 bits, so the first resize is already satisfied; the
    // second genuinely narrows; the third is satisfied by the second.
    let mut insns = vec![
        Insn::LoadSignal(0, 0),
        Insn::Resize(0, 8),
        Insn::Resize(0, 4),
        Insn::Resize(0, 4),
    ];
    BytecodeCompiler::elide_redundant_resizes(&mut insns, &[8], None, 1);
    assert!(matches!(
        insns.as_slice(),
        [
            Insn::LoadSignal(0, 0),
            Insn::Nop,
            Insn::Resize(0, 4),
            Insn::Nop
        ]
    ));
}

#[test]
fn a_resize_that_is_a_branch_target_is_never_removed() {
    // Control can arrive at index 3 without having run index 2, so the
    // width of r0 there depends on the path taken.
    let mut insns = vec![
        Insn::LoadSignal(0, 0),
        Insn::BranchIfFalse(0, 3),
        Insn::Resize(0, 8),
        Insn::Resize(0, 8),
    ];
    BytecodeCompiler::elide_redundant_resizes(&mut insns, &[8], None, 1);
    assert!(matches!(insns[2], Insn::Nop));
    assert!(matches!(insns[3], Insn::Resize(0, 8)));
}

fn carrying_fallback(reg: Option<RegId>) -> Insn {
    let stmt = Statement::new(
        StatementKind::Expr(ident_expr("x")),
        crate::ast::Span::dummy(),
    );
    let locals: FbLocals = match reg {
        Some(reg) => vec![FbLocal {
            name: Arc::from("x"),
            reg,
            kind: LocalKind::Int {
                width: 8,
                signed: false,
                two_state: false,
            },
            via_binding: false,
            class: None,
        }]
        .into_boxed_slice(),
        None => FbLocals::default(),
    };
    Insn::StmtFallback(Box::new((Arc::new(stmt), Arc::from(""), locals)))
}

/// A fallback that carries a register-backed local copies it back, so
/// nothing known about that register before it may be used after it.
#[test]
fn constant_folding_stops_at_a_fallback_that_carries_the_register() {
    let run = |fb: Insn| {
        let mut insns = vec![
            Insn::LoadConst(0, Box::new(Value::from_u64(3, 8))),
            Insn::LoadSignal(2, 0),
            fb,
            Insn::BitSelect(1, 2, 0),
            Insn::BlockingAssign(0, 1, 8),
        ];
        BytecodeCompiler::fold_const_regs(&mut insns, &[8]);
        insns
    };
    let carried = run(carrying_fallback(Some(0)));
    assert!(
        matches!(carried[3], Insn::BitSelect(1, 2, 0)),
        "{:?}",
        carried[3]
    );
    // An ordinary fallback cannot see registers: the index still folds.
    let plain = run(carrying_fallback(None));
    assert!(
        matches!(plain[3], Insn::BitSelectConst(1, 2, 3)),
        "{:?}",
        plain[3]
    );
}

/// Carried locals live in the four-state register file; the two-state
/// lowering must leave such a block alone.
#[test]
fn two_state_lowering_refuses_a_fallback_that_carries_locals() {
    let block = |fb: Insn| CompiledBlock {
        instructions: vec![
            Insn::LoadConst(0, Box::new(Value::from_u64(3, 8))),
            fb,
            Insn::NbaAssign(0, 0, 8),
        ],
        num_regs: 1,
        has_fallback: true,
        nba_dup_targets: false,
        foreach_slots: 0,
        static_locals: Box::new([]),
    };
    let lower =
        |cb: &CompiledBlock| lower_two_state(cb, &[8], &[false], &[false], &HashMap::default());
    assert!(lower(&block(carrying_fallback(Some(0)))).is_none());
    assert!(lower(&block(carrying_fallback(None))).is_some());
}

#[test]
fn arithmetic_on_a_possibly_real_operand_keeps_its_resize() {
    // A signal's declared type may be `real`, and `Value::add` then returns
    // a 64-bit `from_f64` instead of `max(width)` — so the result width is
    // not established here even though both operand widths are known.
    // `bitwise_or` has no such special case, so its resize does go.
    let mut insns = vec![
        Insn::LoadSignal(0, 0),
        Insn::LoadSignal(1, 0),
        Insn::Add(2, 0, 1),
        Insn::Resize(2, 8),
        Insn::BitOr(3, 0, 1),
        Insn::Resize(3, 8),
    ];
    BytecodeCompiler::elide_redundant_resizes(&mut insns, &[8], None, 4);
    assert!(matches!(insns[3], Insn::Resize(2, 8)));
    assert!(matches!(insns[5], Insn::Nop));
}

// class-perf: an all-or-nothing method-body compile lowers `this.<member>`
// reads and writes to LoadClassMember/StoreClassMember, never an AST
// fallback, for a body the compiler fully understands.
#[test]
fn method_body_this_member_lowers() {
    fn span() -> crate::ast::Span {
        crate::ast::Span::dummy()
    }
    fn this_member(member: &str) -> Expression {
        Expression::new(
            ExprKind::MemberAccess {
                expr: Box::new(Expression::new(ExprKind::This, span())),
                member: crate::ast::Identifier {
                    name: member.to_string(),
                    span: span(),
                },
            },
            span(),
        )
    }
    fn num(n: u64) -> Expression {
        Expression::new(
            ExprKind::Number(NumberLiteral::Integer {
                size: None,
                signed: false,
                base: NumberBase::Decimal,
                value: n.to_string(),
                cached_val: Default::default(),
            }),
            span(),
        )
    }
    // function int f(int delta); this.acc = 5; return this.acc; endfunction
    let formals = vec![("delta".to_string(), 32u32)];
    let class_formals: HashSet<String> = Default::default();
    let stmt_store = Statement::new(
        StatementKind::BlockingAssign {
            lvalue: this_member("acc"),
            rvalue: num(5),
        },
        span(),
    );
    let stmt_ret = Statement::new(StatementKind::Return(Some(this_member("acc"))), span());
    let body: Vec<&Statement> = vec![&stmt_store, &stmt_ret];

    let sigmap: NameMap = Default::default();
    let sig_signed: Vec<bool> = Vec::new();
    let sig_w: Vec<u32> = Vec::new();
    let arrays: HashMap<String, (i64, i64, u32)> = Default::default();
    let widths: HashMap<String, u32> = Default::default();
    let mut compiler = BytecodeCompiler::new(&sigmap, &sig_signed, &sig_w, &arrays, &widths);

    let mut admission = MethodAdmission::default();
    admission
        .instance_fields
        .insert(String::new(), ["acc".to_string()].into_iter().collect());
    compiler.method_admission = Some(std::rc::Rc::new(admission));

    let empty_fns: HashMap<String, FunctionDeclaration> = HashMap::default();
    let out = compiler.compile_class_method(
        &formals,
        &class_formals,
        &HashSet::default(),
        &HashSet::default(),
        &HashSet::default(),
        &None,
        &HashMap::default(),
        &HashMap::default(),
        &HashSet::default(),
        &HashSet::default(),
        &HashMap::default(),
        &HashSet::default(),
        &HashMap::default(),
        &HashSet::default(),
        &empty_fns,
        &HashMap::default(),
        "",
        &HashMap::default(),
        &HashSet::default(),
        &HashSet::default(),
        &HashSet::default(),
        &HashSet::default(),
        &HashSet::default(),
        &HashSet::default(),
        &HashSet::default(),
        &HashSet::default(),
        &HashSet::default(),
        &[],
        Some(("f", 32, false, false)),
        false,
        &body,
    );
    let (block, this_reg, _result_reg, _ret_reg) =
        out.expect("simple this.member body should compile all-or-nothing");
    // `this` occupies the method's first allocated register (slot 0).
    assert_eq!(this_reg, 0);
    // Exactly one store and one load of the `acc` member.
    let stores = block
        .instructions
        .iter()
        .filter(|i| matches!(i, Insn::StoreClassMember(..)))
        .count();
    let loads = block
        .instructions
        .iter()
        .filter(|i| matches!(i, Insn::LoadClassMember(..)))
        .count();
    assert_eq!(stores, 1, "expected one StoreClassMember, got {stores}");
    assert_eq!(loads, 1, "expected one LoadClassMember, got {loads}");
    assert!(!block.has_fallback, "method body must have no AST fallback");
}

// class-perf: a body that CALLS a method (`this.compute(1)`) in an
// EXPRESSION position on `this` now LOWERS to a CallMethod instruction
// (Step 4): the compiled body re-enters the interpreter's method
// dispatcher for the callee, so the caller stays 100% VM. Still
// byte-identical by construction (the callee keeps full AST semantics).
#[test]
fn method_body_with_call_lowers_to_callmethod() {
    fn span() -> crate::ast::Span {
        crate::ast::Span::dummy()
    }
    fn this_member(member: &str) -> Expression {
        Expression::new(
            ExprKind::MemberAccess {
                expr: Box::new(Expression::new(ExprKind::This, span())),
                member: crate::ast::Identifier {
                    name: member.to_string(),
                    span: span(),
                },
            },
            span(),
        )
    }
    // return this.compute(1);   (a virtual method call)
    let call = Expression::new(
        ExprKind::Call {
            func: Box::new(this_member("compute")),
            args: vec![Expression::new(
                ExprKind::Number(NumberLiteral::UnbasedUnsized('1')),
                span(),
            )],
        },
        span(),
    );
    let stmt_ret = Statement::new(StatementKind::Return(Some(call)), span());
    let body: Vec<&Statement> = vec![&stmt_ret];

    let sigmap: NameMap = Default::default();
    let sig_signed: Vec<bool> = Vec::new();
    let sig_w: Vec<u32> = Vec::new();
    let arrays: HashMap<String, (i64, i64, u32)> = Default::default();
    let widths: HashMap<String, u32> = Default::default();
    let mut compiler = BytecodeCompiler::new(&sigmap, &sig_signed, &sig_w, &arrays, &widths);
    compiler.method_admission = Some(std::rc::Rc::new(MethodAdmission::default()));

    let empty_fns: HashMap<String, FunctionDeclaration> = HashMap::default();
    let out = compiler.compile_class_method(
        &[],
        &HashSet::default(),
        &HashSet::default(),
        &HashSet::default(),
        &HashSet::default(),
        &None,
        &HashMap::default(),
        &HashMap::default(),
        &HashSet::default(),
        &HashSet::default(),
        &HashMap::default(),
        &HashSet::default(),
        &HashMap::default(),
        &HashSet::default(),
        &empty_fns,
        &HashMap::default(),
        "",
        &HashMap::default(),
        &HashSet::default(),
        &HashSet::default(),
        &HashSet::default(),
        &HashSet::default(),
        &HashSet::default(),
        &HashSet::default(),
        &HashSet::default(),
        &HashSet::default(),
        &HashSet::default(),
        &[],
        Some(("f", 32, false, false)),
        false,
        &body,
    );
    let compiled =
        out.expect("a body whose only call is this.compute(1) must now compile (CallMethod)");
    let cm = compiled
        .0
        .instructions
        .iter()
        .filter(|i| {
            matches!(
                i,
                Insn::CallMethod(..)
                    | Insn::CallCollMethod(..)
                    | Insn::LoadCollElem(..)
                    | Insn::StoreCollElem(..)
            )
        })
        .count();
    assert_eq!(cm, 1, "expected one CallMethod, got {cm}");
    assert!(
        !compiled.0.has_fallback,
        "method body must have no AST fallback"
    );
}

// A class method with an unresolved bare name must decline compilation
// so the interpreter can resolve the name against its lexical class scope.
#[test]
fn method_body_with_unresolved_class_scope_name_bails() {
    // return os; (no register, member metadata, or module signal binds os)
    let stmt_ret = Statement::new(
        StatementKind::Return(Some(ident_expr("os"))),
        crate::ast::Span::dummy(),
    );
    let body: Vec<&Statement> = vec![&stmt_ret];

    let sigmap: NameMap = Default::default();
    let sig_signed: Vec<bool> = Vec::new();
    let sig_w: Vec<u32> = Vec::new();
    let arrays: HashMap<String, (i64, i64, u32)> = Default::default();
    let widths: HashMap<String, u32> = Default::default();

    let mut compiler = BytecodeCompiler::new(&sigmap, &sig_signed, &sig_w, &arrays, &widths);
    let empty_fns: HashMap<String, FunctionDeclaration> = HashMap::default();
    assert!(
        compiler
            .compile_class_method(
                &[],
                &HashSet::default(),
                &HashSet::default(),
                &HashSet::default(),
                &HashSet::default(),
                &None,
                &HashMap::default(),
                &HashMap::default(),
                &HashSet::default(),
                &HashSet::default(),
                &HashMap::default(),
                &HashSet::default(),
                &HashMap::default(),
                &HashSet::default(),
                &empty_fns,
                &HashMap::default(),
                "",
                &HashMap::default(),
                &HashSet::default(),
                &HashSet::default(),
                &HashSet::default(),
                &HashSet::default(),
                &HashSet::default(),
                &HashSet::default(),
                &HashSet::default(),
                &HashSet::default(),
                &HashSet::default(),
                &[],
                Some(("f", 32, false, false)),
                false,
                &body
            )
            .is_none(),
        "an unresolved class-scope name must leave the method on the interpreter"
    );
}
