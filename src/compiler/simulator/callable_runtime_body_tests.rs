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
