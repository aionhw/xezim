// Driver for tests/dpi/vpi_systf.c — $systf arguments, system functions,
// vpi_chk_error and vpi_control.
module tb;
  int a, b, r;
  int sv_errors;

  `define check(cond, msg) \
    if (!(cond)) begin sv_errors = sv_errors + 1; $display("FAIL: %s", msg); end

  initial begin
    sv_errors = 0;
    a = 7;
    b = 5;

    // vpiSysTfCall + vpiArgument: read a signal, a literal, another signal.
    $st_args(a, 42, b);

    // A signal-backed argument is writable: this is how an output argument works.
    $st_bump(a);
    `check(a === 107, "an output argument must be written back")

    // A literal argument is a read-only vpiConstant.
    $st_const(42);

    // A registered system FUNCTION, dispatched from an expression.
    r = $st_triple(a);
    `check(r === 321, "$st_triple(107) must be 321")

    // ...and inside a larger expression. The operand must be evaluated ONCE:
    // `infer_width` used to learn a width by evaluating, so the calltf ran
    // twice for every system function inside a binary operator.
    r = $st_triple(4) + 1;
    `check(r === 13, "$st_triple(4) + 1 must be 13")

    // A function that deposits no value returns 0.
    r = $st_silent();
    `check(r === 0, "a function that deposits nothing returns 0")

    // vpiSizedFunc: sizetf sets the width, and the deposit truncates to it.
    r = $st_sized();
    `check(r === 8'hFF, "a vpiSizedFunc return must truncate to its sizetf width")

    // vpi_chk_error round trip.
    $st_errors;

    // vpi_control(vpiFinish) ends the run.
    $st_report;
    if (sv_errors == 0) $display("RESULT: PASSED");
    else                $display("RESULT: FAILED (%0d)", sv_errors);
    $st_finish;
    $display("FAIL: vpi_control(vpiFinish) did not end the run");
  end
endmodule
