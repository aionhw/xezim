// Baseline semantics for class reclamation: every retained handle must survive
// allocation churn, including handles that are not ordinary scalar globals.
#[test]
fn retained_class_handles_survive_allocation_churn() {
    let sim = crate::simulate(
        include_str!("../../../tests/fixtures/memory/heap_lifetimes.sv"),
        100_000,
    )
    .expect("heap lifetime fixture");
    assert!(sim.finished);
    assert_eq!(sim.error_count, 0);
    assert!(sim
        .output
        .iter()
        .any(|line| { line.message.contains("HEAP_LIFETIMES_PASS checks=5") }));
}

#[test]
fn lifetime_fixture_exercises_hidden_roots_while_processes_are_parked() {
    let sim = crate::simulate(
        include_str!("../../../tests/fixtures/memory/heap_lifetimes.sv"),
        8,
    )
    .expect("parked heap lifetime fixture");
    assert!(!sim.finished);
    assert_eq!(sim.error_count, 0);
    let node = |id| {
        sim.heap
            .iter()
            .enumerate()
            .find_map(|(handle, object)| {
                let object = object.as_ref()?;
                (object.class_name == "Node"
                    && object.properties.get("id").and_then(|v| v.to_u64()) == Some(id))
                .then_some(handle as u64)
            })
            .expect("retained node")
    };
    // These values are captured outside the ordinary current local frame.
    let blocked = node(66);
    assert!(sim
        .mailbox_put_waiters
        .values()
        .flatten()
        .any(|waiter| { waiter.value.to_u64() == Some(blocked) }));
    let mut trace = super::heap_trace::HandleTrace::new(sim.heap.len());
    trace.trace(&sim.process_contexts).unwrap();
    let shadowed = node(99);
    assert!(
        trace.candidates.get(shadowed as usize),
        "serialized context omitted shadowed handle"
    );
    assert!(sim.process_contexts.values().any(|ctx| {
        ctx.block_scope_saves.iter().any(|scope| {
            scope
                .shadowed
                .iter()
                .any(|(_, value)| value.to_u64() == Some(shadowed))
        })
    }));
}

#[test]
fn heap_graph_keeps_hidden_roots_and_drops_unreferenced_churn() {
    let sim = crate::simulate(
        include_str!("../../../tests/fixtures/memory/heap_lifetimes.sv"),
        8,
    )
    .expect("parked class graph");
    let marked = sim
        .class_heap_reachability_probe()
        .expect("trace class graph");
    for id in [11, 22, 23, 33, 44, 55, 66, 71, 77, 99] {
        let handle = sim
            .heap
            .iter()
            .position(|object| {
                object.as_ref().is_some_and(|o| {
                    o.class_name == "Node"
                        && o.properties.get("id").and_then(|v| v.to_u64()) == Some(id)
                })
            })
            .expect("retained node");
        assert!(marked[handle], "live node {id} was omitted from the graph");
    }
    let allocated = sim.heap.iter().filter(|o| o.is_some()).count();
    let reachable = marked.iter().filter(|&&b| b).count();
    assert!(
        reachable < allocated / 2,
        "graph retained all churn: {reachable}/{allocated}"
    );
}

#[test]
fn class_sweep_removes_owned_storage_and_preserves_other_instances() {
    let mut sim = crate::simulate(
        r#"
        class Storage;
          int id;
          bit [127:0] fixed_data[4];
          int queue_data[$];
          int assoc_data[int];
          function new(int n);
            id = n;
            fixed_data[0] = 128'h123456789abcdef0123456789abcdef0;
            queue_data.push_back(n);
            assoc_data[7] = n;
          endfunction
        endclass
        module top;
          Storage retained, temporary;
          initial begin
            retained = new(42);
            temporary = new(43);
            temporary = null;
            $finish;
          end
        endmodule
    "#,
        100,
    )
    .expect("owned storage fixture");
    let owner = |id| {
        sim.heap
            .iter()
            .position(|o| {
                o.as_ref().is_some_and(|o| {
                    o.class_name == "Storage"
                        && o.properties.get("id").and_then(|v| v.to_u64()) == Some(id)
                })
            })
            .unwrap()
    };
    let live = owner(42);
    let dead = owner(43);
    let dead_array = format!("{dead}#fixed_data");
    let live_array = format!("{live}#fixed_data");
    assert!(!sim.signals.elem_keys(&dead_array).is_empty());
    let saved = sim
        .signals
        .get(&format!("{live_array}[0]"))
        .cloned()
        .unwrap();
    let mut mask = vec![false; sim.heap.len()];
    mask[dead] = true;
    let slots = sim.heap.len();
    assert_eq!(sim.release_unreachable_class_storage(&mask), 1);
    assert!(sim.heap[dead].is_none());
    assert!(sim.heap[live].is_some());
    assert_eq!(sim.heap.len(), slots);
    assert!(sim.signals.elem_keys(&dead_array).is_empty());
    assert!(!sim.module.arrays.contains_key(&dead_array));
    assert!(
        !sim.signals
            .keys()
            .any(|n| super::Simulator::class_storage_owner(&n) == Some(dead))
    );
    assert_eq!(sim.signals.get(&format!("{live_array}[0]")), Some(&saved));
    assert!(sim.module.arrays.contains_key(&live_array));
    assert_eq!(sim.release_unreachable_class_storage(&mask), 0);
}

#[test]
fn paused_lifetime_fixture_resumes_after_prototype_mark_and_sweep() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let mut sim = crate::simulate(
                include_str!("../../../tests/fixtures/memory/heap_lifetimes.sv"),
                8,
            )
            .expect("paused collection fixture");
            let marked = sim.class_heap_reachability_probe().expect("mark roots");
            let mut dead: Vec<_> = marked.into_iter().map(|mark| !mark).collect();
            dead[0] = false;
            let before = sim.heap.iter().filter(|o| o.is_some()).count();
            let released = sim.release_unreachable_class_storage(&dead);
            assert!(released > before / 2, "reclaimed only {released}/{before}");
            sim.max_time = 100_000;
            sim.event_loop(false);
            assert!(sim.finished);
            assert_eq!(sim.error_count, 0);
            assert!(sim
                .output
                .iter()
                .any(|o| o.message.contains("HEAP_LIFETIMES_PASS checks=5")));
        })
        .unwrap()
        .join()
        .unwrap();
}

// §8.7: with reclamation on (the default), objects no handle reaches are
// freed between time slots while a retained alias keeps its object and data.
#[test]
fn default_reclamation_bounds_live_objects_under_churn() {
    if super::heap_gc::class_gc_mode() == super::heap_gc::ClassGcMode::Off {
        return;
    }
    let sim = crate::simulate(
        r#"
        class Sample;
          int data[16];
          int q[$];
          function new(int n); foreach (data[i]) data[i] = n + i; q.push_back(n); endfunction
        endclass
        module top;
          Sample current, kept;
          initial begin
            for (int n = 0; n < 60000; n++) begin
              current = new(n);
              if (n == 17) kept = current;
              if (n % 100 == 0) #1;
            end
            if (kept.data[3] != 20 || kept.q[0] != 17 || current.data[0] != 59999)
              $fatal(1, "retained data lost");
            $display("CHURN_PASS");
            $finish;
          end
        endmodule
    "#,
        100_000,
    )
    .expect("churn fixture");
    assert_eq!(sim.error_count, 0);
    assert!(sim.output.iter().any(|o| o.message.contains("CHURN_PASS")));
    assert!(sim.class_gc_runs > 0, "no collection ran");
    let live = sim.heap.iter().filter(|o| o.is_some()).count();
    assert!(live < 25_000, "{live} objects still live after 60000 allocations");
}

// §8.7 (#1054): the elements of a fixed array member are name-keyed values
// of their own, so objects far fewer than the allocation threshold can hold
// most of the store. Their growth triggers a collection too.
#[test]
fn array_payloads_trigger_reclamation_below_the_allocation_count() {
    if super::heap_gc::class_gc_mode() != super::heap_gc::ClassGcMode::On {
        return;
    }
    let sim = crate::simulate(
        r#"
        class Item;
          byte data[256];
        endclass
        module top;
          Item h, kept;
          initial begin
            for (int n = 0; n < 2048; n++) begin
              h = new();
              h.data[0] = 7;
              h.data[255] = n[6:0];
              if (n == 17) kept = h;
              h = null;
              if (n % 64 == 63) #1;
            end
            if (kept.data[0] != 7 || kept.data[255] != 17) $fatal(1, "retained data lost");
            $display("PAYLOAD_PASS");
            $finish;
          end
        endmodule
    "#,
        100_000,
    )
    .expect("payload fixture");
    assert_eq!(sim.error_count, 0);
    assert!(
        sim.output
            .iter()
            .any(|o| o.message.contains("PAYLOAD_PASS"))
    );
    assert!(sim.class_gc_runs > 0, "no collection ran");
    let live = sim.heap.iter().filter(|o| o.is_some()).count();
    assert!(
        live < 1024,
        "{live} objects still live after 2048 allocations"
    );
}

/// §8.7 / §15.4.3: a captured pattern, rather than its tagged token, owns
/// the object until a queued message is consumed.
#[test]
fn mailbox_pattern_objects_survive_collection_and_delivery() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let mut sim = crate::simulate(
                include_str!("../../../tests/fixtures/memory/mailbox_pattern_gc.sv"),
                1,
            )
            .expect("queued pattern fixture");
            assert_eq!(sim.error_count, 0);
            let handle = sim
                .heap
                .iter()
                .position(|o| {
                    o.as_ref().is_some_and(|o| {
                        o.class_name == "Node"
                            && o.properties.get("tag").and_then(|v| v.to_u64()) == Some(777)
                    })
                })
                .expect("queued object");
            let marked = sim
                .class_heap_reachability_probe()
                .expect("trace queued pattern");
            assert!(marked[handle], "pattern's class handle was not retained");
            let mut dead: Vec<_> = marked.into_iter().map(|m| !m).collect();
            dead[0] = false;
            let released = sim.release_unreachable_class_storage(&dead);
            assert!(
                released > 0 || sim.class_gc_runs > 0,
                "neither automatic nor explicit collection ran"
            );
            assert!(sim.heap[handle].is_some());
            sim.max_time = 10;
            sim.event_loop(false);
            assert!(sim.finished);
            assert_eq!(sim.error_count, 0);
            assert_eq!(
                sim.output
                    .iter()
                    .filter(|o| o.message == "T|peek|777|99")
                    .count(),
                2
            );
            assert!(sim.output.iter().any(|o| o.message == "T|pattern|777|99"));
        })
        .unwrap()
        .join()
        .unwrap();
}

/// A dead mailbox releases captured syntax; a full live mailbox and a
/// producer parked on it keep both their patterns (§8.7 / §15.4.1).
#[test]
fn dead_mailboxes_release_patterns_but_parked_producers_keep_theirs() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let mut sim = crate::simulate(
                r#"
typedef struct { int a; int b; } pair_t;
module top;
 mailbox #(pair_t) live = new(1), dropped = new();
 initial begin
   dropped.put('{a:11,b:22}); dropped=null;
   live.put('{a:33,b:44});
   fork live.put('{a:55,b:66}); join_none
   #10; $finish;
 end
endmodule
"#,
                1,
            )
            .expect("pattern cleanup fixture");
            assert_eq!(sim.error_count, 0);
            assert_eq!(sim.mbx_patterns.len(), 3);
            assert_eq!(
                sim.mailbox_put_waiters
                    .values()
                    .map(|q| q.len())
                    .sum::<usize>(),
                1
            );
            let dead_mailbox = *sim
                .mailboxes
                .keys()
                .find(|h| sim.mailbox_bound.get(h).copied().unwrap_or(0) == 0)
                .expect("unreferenced mailbox");
            let mut dead = vec![false; sim.heap.len()];
            dead[dead_mailbox] = true;
            sim.release_unreachable_class_storage(&dead);
            assert!(!sim.mailboxes.contains_key(&dead_mailbox));
            assert_eq!(sim.mbx_patterns.len(), 2);
            for v in sim
                .mailboxes
                .values()
                .flatten()
                .chain(sim.mailbox_put_waiters.values().flatten().map(|w| &w.value))
            {
                assert!(sim.mbx_pattern_items(v).is_some());
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

// §8.7: collections after the first trace only the previous survivors and
// the objects allocated since. An object that survived (through an edge from
// another survivor) must still be reclaimed once that edge goes away, and an
// object allocated later and reachable only from a survivor must stay live.
#[test]
fn incremental_collection_tracks_survivors_across_runs() {
    let sim = crate::simulate(
        include_str!("../../../tests/fixtures/memory/heap_incremental.sv"),
        100_000,
    )
    .expect("incremental heap fixture");
    assert!(sim.finished);
    assert_eq!(sim.error_count, 0);
    assert!(sim.class_gc_runs >= 3, "only {} collections", sim.class_gc_runs);
    assert!(sim
        .output
        .iter()
        .any(|line| line.message.contains("GC_INCR sum=9 next_null=1")));
    let ids: Vec<u64> = sim
        .heap
        .iter()
        .flatten()
        .filter(|object| object.class_name == "Node")
        .filter_map(|object| object.properties.get("id").and_then(|v| v.to_u64()))
        .filter(|&id| id < 1000)
        .collect();
    assert_eq!(ids, vec![1, 8], "node 7 must be reclaimed, 1 and 8 kept");
}

/// §7.5.1, §7.10.2, §7.12.1, §8.7 (#1667): removed elements and
/// locator temporaries no longer keep their class handles reachable.
#[test]
fn sequence_shrinks_and_locator_temps_release_handles() {
    let source = r#"
class Item;
  int id = 9917;
endclass
module top;
  Item h, keep;
  Item pop_q[$], del_q[$], locator_q[$], da[];
  initial begin
    da = new[256];
    for (int i = 0; i < 256; i++) begin
      h = new;
      pop_q.push_back(h);
      del_q.push_back(h);
      locator_q.push_back(h);
      da[i] = h;
    end
    keep = h; h = null;
    if (locator_q.unique().size() != 256) $fatal(1, "locator size");
    $display("locator: %p", locator_q.find_first with (item != null));
    repeat (256) begin
      h = pop_q.pop_back(); h = null;
      h = locator_q.pop_front(); h = null;
    end
    del_q.delete();
    da = new[0](da);
    if (keep.id != 9917 || pop_q.size() != 0 || del_q.size() != 0 ||
        locator_q.size() != 0 || da.size() != 0) $fatal(1, "live handle or size");
    $finish;
  end
endmodule
"#;
    let sim = crate::simulate(source, 100).expect("collection fixture");
    assert_eq!(sim.error_count, 0);
    for name in ["pop_q", "del_q", "locator_q", "da"] {
        assert!(
            sim.signals.elem_keys(name).is_empty(),
            "stale {name} values"
        );
    }
    assert!(
        !sim.signals
            .keys()
            .any(|key| key.starts_with("@locator") || key.starts_with("__xz_locator"))
    );
    let marked = sim.class_heap_reachability_probe().expect("trace roots");
    let keep = sim.get_signal("keep").unwrap().to_u64().unwrap() as usize;
    assert!(marked[keep], "live handle was not traced");
    let reachable = marked.iter().filter(|&&mark| mark).count();
    assert!(reachable < 32, "{reachable} surviving handles");
}

/// §7.10, §8.7 (#1667): integer queue positions are not object references.
#[test]
fn dense_integer_queue_positions_do_not_keep_class_objects() {
    let source = r#"
class Item;
endclass
module top;
  int q[$];
  Item h;
  initial begin
    repeat (256) begin
      h = new;
      q.push_back(0);
    end
    h = null;
    if (q.size() != 256) $fatal(1, "queue size");
    $finish;
  end
endmodule
"#;
    let sim = crate::simulate(source, 100).expect("index fixture");
    assert_eq!(sim.error_count, 0);
    let marked = sim.class_heap_reachability_probe().expect("trace roots");
    let reachable = marked.iter().filter(|&&mark| mark).count();
    assert!(
        reachable < 32,
        "{reachable} handles retained by queue positions"
    );
}

/// §6.21, §7.8 (#1667): associative metadata has the local array's lifetime.
#[test]
fn local_collection_metadata_is_reclaimed_with_the_frame() {
    let source = r#"
module top;
  task automatic churn();
    int a[int] = '{default: 17};
    int rows[][int];
    a[3] = 4;
    rows = new[2];
    rows[1][3] = 4;
    if (a[0] != 17 || rows[1][3] != 4) $fatal(1, "local collection");
  endtask
  initial begin
    repeat (32) churn();
    $finish;
  end
endmodule
"#;
    let sim = crate::simulate(source, 100).expect("metadata fixture");
    assert_eq!(sim.error_count, 0);
    assert!(
        !sim.module
            .assoc_elem_widths
            .keys()
            .any(|k| k.starts_with('@'))
    );
    assert!(!sim.module.assoc_defaults.keys().any(|k| k.starts_with('@')));
}
