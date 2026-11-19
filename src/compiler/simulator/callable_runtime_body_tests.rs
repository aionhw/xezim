//! IEEE 1800-2023 §13.3/§13.4/§23.6: one private body per called lexical
//! declaration restores runtime memoization without expanding unused bodies.
use super::*;
use std::sync::Arc;

#[test]
fn called_declarations_materialize_independent_cached_bodies() {
    let mut sim = crate::simulate(
        "module leaf;
           function automatic int f(input int x); return x+1; endfunction
           task automatic t(input int x); x=x+1; endtask
           class C;
             function int f(input int x); return x+1; endfunction
             task t(input int x); x=x+1; endtask
           endclass
         endmodule
         module top; leaf a(); leaf b(); initial $finish; endmodule",
        0,
    )
    .expect("callable ownership fixture");
    let source_a = sim.module.functions["a.f"].items.clone();
    let source_b = sim.module.functions["b.f"].items.clone();
    assert!(Arc::ptr_eq(source_a.as_arc(), source_b.as_arc()));
    let a = sim.fn_decl_rc("a.f").unwrap();
    let b = sim.fn_decl_rc("b.f").unwrap();
    assert!(!Arc::ptr_eq(source_a.as_arc(), a.items.as_arc()));
    assert!(!Arc::ptr_eq(a.items.as_arc(), b.items.as_arc()));
    assert!(std::rc::Rc::ptr_eq(&a, &sim.fn_decl_rc("a.f").unwrap()));
    fn returned(items: &crate::ast::shared_vec::SharedVec<Statement>) -> &Expression {
        match &items[0].kind {
            StatementKind::Return(Some(expr)) => expr,
            _ => panic!("return expected"),
        }
    }
    let a_expr = returned(&a.items);
    let b_expr = returned(&b.items);
    a_expr.cached_width.set(Some(8));
    b_expr.cached_width.set(Some(16));
    assert_eq!(a_expr.cached_width.get(), Some(8));
    assert_eq!(b_expr.cached_width.get(), Some(16));
    let source_expr = returned(&source_a);
    source_expr.cached_width.set(Some(99));
    assert_eq!(source_expr.cached_width.get(), None);
    let ExprKind::Binary { left, .. } = &a_expr.kind else {
        panic!("binary expected")
    };
    let ExprKind::Ident(name) = &left.kind else {
        panic!("formal expected")
    };
    name.cached_resolved_name.set("a.x".into()).unwrap();
    name.cached_signal_id.set(Some(7));
    assert_eq!(name.cached_signal_id.get(), Some(7));

    let task_source = sim.module.tasks["a.t"].items.clone();
    let task = sim.task_decl_rc("a.t").unwrap();
    let sibling_task = sim.task_decl_rc("b.t").unwrap();
    assert!(!Arc::ptr_eq(
        task.items.as_arc(),
        sibling_task.items.as_arc()
    ));
    assert!(!Arc::ptr_eq(task_source.as_arc(), task.items.as_arc()));
    assert!(std::rc::Rc::ptr_eq(
        &task,
        &sim.task_decl_rc("a.t").unwrap()
    ));
    let StatementKind::BlockingAssign { rvalue, .. } = &task.items[0].kind else {
        panic!("assignment expected");
    };
    rvalue.cached_width.set(Some(32));
    assert_eq!(rvalue.cached_width.get(), Some(32));
    let StatementKind::BlockingAssign { rvalue, .. } = &sibling_task.items[0].kind else {
        panic!("assignment expected");
    };
    assert_eq!(rvalue.cached_width.get(), None);
    let StatementKind::BlockingAssign { rvalue, .. } = &task_source[0].kind else {
        panic!("assignment expected");
    };
    rvalue.cached_width.set(Some(99));
    assert_eq!(rvalue.cached_width.get(), None);

    fn method_body(
        method: &crate::ast::decl::ClassMethod,
    ) -> &crate::ast::shared_vec::SharedVec<Statement> {
        match &method.kind {
            crate::ast::decl::ClassMethodKind::Function(fd) => &fd.items,
            crate::ast::decl::ClassMethodKind::Task(td) => &td.items,
            _ => panic!("callable method expected"),
        }
    }
    let method_source = method_body(&sim.module.classes["a.C"].methods["f"]).clone();
    let sibling_source = method_body(&sim.module.classes["b.C"].methods["f"]).clone();
    assert!(Arc::ptr_eq(method_source.as_arc(), sibling_source.as_arc()));
    let method = sim.cached_class_method("a.C", "f").unwrap();
    let sibling = sim.cached_class_method("b.C", "f").unwrap();
    assert!(Arc::ptr_eq(
        &method,
        &sim.cached_class_method("a.C", "f").unwrap()
    ));
    assert!(!Arc::ptr_eq(
        method_body(&method).as_arc(),
        method_source.as_arc()
    ));
    assert!(!Arc::ptr_eq(
        method_body(&method).as_arc(),
        method_body(&sibling).as_arc()
    ));
    returned(method_body(&method)).cached_width.set(Some(8));
    assert_eq!(returned(method_body(&method)).cached_width.get(), Some(8));
    assert_eq!(returned(method_body(&sibling)).cached_width.get(), None);
    returned(&method_source).cached_width.set(Some(99));
    assert_eq!(returned(&method_source).cached_width.get(), None);
    let class_task = sim.cached_class_method("a.C", "t").unwrap();
    let StatementKind::BlockingAssign { rvalue, .. } = &method_body(&class_task)[0].kind else {
        panic!("class task assignment expected");
    };
    rvalue.cached_width.set(Some(32));
    assert_eq!(rvalue.cached_width.get(), Some(32));

    // A checkpoint stores AST caches only as derived data. Restoring must
    // discard declaration handles so their next use rebuilds enabled caches.
    let directory = std::env::temp_dir().join(format!(
        "xezim_callable_checkpoint_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    std::fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let checkpoint = directory.join("state.ckpt");
    let checkpoint = checkpoint.to_str().unwrap();
    sim.checkpoint_save(checkpoint)
        .expect("save private callable caches");
    sim.checkpoint_restore(checkpoint)
        .expect("restore callable caches");
    assert!(sim.fn_decl_cache.is_empty());
    assert!(sim.task_decl_cache.is_empty());
    assert!(sim.class_method_cache.borrow().is_empty());
    let restored_method = sim.cached_class_method("a.C", "f").unwrap();
    assert!(!Arc::ptr_eq(&method, &restored_method));
    returned(method_body(&restored_method))
        .cached_width
        .set(Some(16));
    assert_eq!(
        returned(method_body(&restored_method)).cached_width.get(),
        Some(16)
    );
    assert_eq!(
        returned(method_body(&sim.module.classes["a.C"].methods["f"]))
            .cached_width
            .get(),
        None
    );
    let restored = sim.fn_decl_rc("a.f").unwrap();
    assert!(!std::rc::Rc::ptr_eq(&restored, &a));
    assert!(!Arc::ptr_eq(restored.items.as_arc(), source_a.as_arc()));
    let expr = returned(&restored.items);
    assert_eq!(expr.cached_width.get(), None);
    expr.cached_width.set(Some(32));
    assert_eq!(expr.cached_width.get(), Some(32));
    assert_eq!(
        returned(&sim.module.functions["a.f"].items)
            .cached_width
            .get(),
        None
    );
    let restored_task = sim.task_decl_rc("a.t").unwrap();
    assert!(!Arc::ptr_eq(
        restored_task.items.as_arc(),
        task_source.as_arc()
    ));
    let StatementKind::BlockingAssign { rvalue, .. } = &restored_task.items[0].kind else {
        panic!("assignment expected");
    };
    rvalue.cached_width.set(Some(64));
    assert_eq!(rvalue.cached_width.get(), Some(64));
}

#[test]
fn deferred_module_state_callables_share_source_and_run_independently() {
    let mut sim = crate::simulate(
        "module leaf #(parameter int INIT = 0);
           int state = INIT;
           int out_v;
           int unused_out;
           function automatic int read_add(input int x);
             return x + state;
           endfunction
           function automatic int shadow_add(input int state);
             return state + 100;
           endfunction
           function automatic int rec_sum(input int n);
             if (n <= 0) return state;
             return n + rec_sum(n - 1);
           endfunction
           function int static_local_fn(input int x);
             static int base = state;
             base = base + x;
             return base;
           endfunction
           task automatic bump(input int d);
             state = state + d;
             out_v = state;
           endtask
         endmodule
         module top;
           leaf #(10) a();
           leaf #(20) b();
           leaf #(99) u_unused();
           int ra, rb, sa, rc, sl1, sl2, ta, tb;
           logic [31:0] comb_out, cont_out, ff_out;
           logic clk = 0;
           assign cont_out = a.read_add(5);
           always_comb comb_out = b.read_add(7);
           always_ff @(posedge clk) ff_out <= a.read_add(1);
           initial begin
             a.bump(3);
             b.bump(4);
             ra = a.read_add(2);
             rb = b.read_add(2);
             sa = a.shadow_add(9);
             rc = b.rec_sum(3);
             sl1 = a.static_local_fn(5);
             sl2 = a.static_local_fn(2);
             ta = a.out_v;
             tb = b.out_v;
             #1; clk = 1; #1;
             a.state = 40;
             b.state = 50;
             #1;
             $finish;
           end
         endmodule",
        20,
    )
    .expect("deferred module state callables");
    assert!(sim.finished);
    assert_eq!(sim.error_count, 0);

    // Module-level declarations keep shared source bodies, and uncalled
    // instances remain absent from the per-callable runtime caches.
    assert!(Arc::ptr_eq(
        sim.module.functions["a.read_add"].items.as_arc(),
        sim.module.functions["u_unused.read_add"].items.as_arc(),
    ));
    assert!(Arc::ptr_eq(
        sim.module.functions["b.shadow_add"].items.as_arc(),
        sim.module.functions["u_unused.shadow_add"].items.as_arc(),
    ));
    assert!(Arc::ptr_eq(
        sim.module.tasks["a.bump"].items.as_arc(),
        sim.module.tasks["u_unused.bump"].items.as_arc(),
    ));
    assert!(!sim.fn_decl_cache.contains_key("u_unused.read_add"));
    assert!(!sim.fn_decl_cache.contains_key("u_unused.shadow_add"));
    assert!(!sim.task_decl_cache.contains_key("u_unused.bump"));

    for (sig, expected) in [
        ("ta", 13),
        ("tb", 24),
        ("ra", 15),
        ("rb", 26),
        ("sa", 109),
        ("rc", 30),
        ("sl1", 15),
        ("sl2", 17),
        ("ff_out", 14),
        ("cont_out", 45),
        ("comb_out", 57),
    ] {
        assert_eq!(
            sim.get_signal(sig).and_then(Value::to_u64),
            Some(expected),
            "signal {sig}"
        );
    }

    // Checkpoint save/restore clears callable caches and re-materializes cleanly.
    let directory = std::env::temp_dir().join(format!(
        "xezim_deferred_callable_ckpt_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    std::fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let ckpt = directory.join("state.ckpt");
    let ckpt = ckpt.to_str().unwrap();
    sim.checkpoint_save(ckpt).expect("save deferred callable ckpt");
    sim.checkpoint_restore(ckpt)
        .expect("restore deferred callable ckpt");
    assert!(sim.fn_decl_cache.is_empty());
    assert!(sim.task_decl_cache.is_empty());
    let ra_fd = sim.fn_decl_rc("a.read_add").unwrap();
    let rb_fd = sim.fn_decl_rc("b.read_add").unwrap();
    assert!(!Arc::ptr_eq(ra_fd.items.as_arc(), rb_fd.items.as_arc()));
    fn rhs_name(fd: &FunctionDeclaration) -> String {
        let StatementKind::Return(Some(expr)) = &fd.items[0].kind else {
            panic!("return expected")
        };
        let ExprKind::Binary { right, .. } = &expr.kind else {
            panic!("binary expected")
        };
        let ExprKind::Ident(h) = &right.kind else {
            panic!("ident expected")
        };
        h.path
            .iter()
            .map(|s| s.name.name.as_str())
            .collect::<Vec<_>>()
            .join(".")
    }
    assert_eq!(rhs_name(&ra_fd), "a.state");
    assert_eq!(rhs_name(&rb_fd), "b.state");
}
