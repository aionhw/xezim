//! §9.7.4: `wait(cond)` inside a `foreach` loop body must block when the
//! condition is false, not silently fall through. The synchronous loop-body
//! path (`exec_statement`) must park the process and resume it when the
//! condition becomes true.
//!
//! This is a regression test for the "phasing" bug: a `wait(state == DONE)`
//! inside `foreach(arr[i])` was silently ignored when the condition was false,
//! causing code after the foreach to execute prematurely.

use xezim::simulate;

const SV: &str = r#"
module automatic tb;
    logic [7:0] arr[3];
    logic done_flag;
    int post_foreach_time;

    initial begin
        arr[0] = 1;
        arr[1] = 2;
        arr[2] = 3;
        done_flag = 0;
        post_foreach_time = -1;

        // Foreach loop with a wait inside the body that blocks until done_flag
        foreach (arr[i]) begin
            // done_flag is 0 on all three iterations, so wait() must block.
            wait (done_flag);
        end

        // This line should only execute AFTER done_flag is set (at time 20).
        post_foreach_time = $time;
    end

    // Separate process sets the flag at time 20.
    initial begin
        #20 done_flag = 1;
    end
endmodule
"#;

#[test]
fn wait_in_foreach_blocks_until_condition() {
    let sim = simulate(SV, 200).expect("simulate failed");
    let post = sim
        .get_signal("post_foreach_time")
        .expect("post_foreach_time not found");
    let t: u64 = post.to_u128() as u64;
    // Should be 20, not 0 — proves wait() inside foreach blocked.
    assert_eq!(
        t, 20,
        "wait() inside foreach did not block — expected post_foreach_time=20, got {}",
        t
    );
}
