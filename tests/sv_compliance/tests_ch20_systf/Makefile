# IEEE 1800-2023 Clause 20 — Utility system tasks/functions (compliance tests)
#
# Run against ANY reference SystemVerilog simulator. Set SIM to a command that
# compiles+runs a single .sv top module, e.g.:
#   make run_all SIM="<your simulator> <flags>"
# NOTE: Verilator is NOT suitable for 41 (aborts on $error) nor 43 ($q_* unsupported).
#
# Pass criteria (intentionally NOT exit-code based):
#   - The self-checking tests (41/43/44/45) deliberately call $error/$warning
#     which inflate the tool's error/warning counters and make some simulator
#     drivers exit NONZERO even on a correct run. The single source of truth is
#     the "TEST_PASS" marker printed by SVTEST_PASSFAIL (its absence => failure,
#     since on failure SVTEST_PASSFAIL calls $fatal and never prints it).
#   - 42_severity_fatal has SPECIAL criteria (it must terminate with $fatal).
#
# $stacktrace (46) is a NEWER LRM feature and is OPTIONAL:
#   - Older simulator releases (released before the feature was added) do NOT
#     support it and fail at design load. It is therefore NOT part of run_all
#     by default.
#   - Verilator 5.x DOES support $stacktrace and is available on this host, so
#     there is an out-of-the-box target that needs no setup:
#         make run_stacktrace_vl
#   - Or run it against a NEWER commercial tool that implements the feature:
#         make run_stacktrace SIM="<your simulator> <flags>"
#
# By default SIM is empty and only the file list is printed.

TEST_DIR = .
LOG_DIR  = ./logs

# Match the TEST_PASS marker as a whole word, tolerating a transcript's
# leading "# " prefix (some drivers prefix captured output lines). Word
# boundaries also correctly EXCLUDE
# "TEST_PASS_IF_FATAL_DID_NOT_TERMINATE" (the trailing "_" is an identifier char).
GREP_PASS = grep -Eq '(^|[^A-Za-z_])TEST_PASS([^A-Za-z_]|$$)'

.PHONY: help run run_all run_stacktrace run_stacktrace_vl clean

help:
	@echo "Targets:"
	@echo "  make run_all SIM=\"<sim cmd>\"            # run 41/42/43/44/45"
	@echo "  make run_stacktrace_vl                   # 46 via Verilator (out of the box)"
	@echo "  make run_stacktrace SIM=\"<sim cmd>\"     # 46 via a 2023-capable tool"
	@echo "  make run TEST=<name> SIM=\"<sim cmd>\"    # one test (e.g. 41_severity_tasks)"

# 1. Run a single self-checking test. Pass iff the TEST_PASS marker appears.
run:
	@mkdir -p $(LOG_DIR)
	@if [ -z "$(SIM)" ]; then echo "Set SIM=<simulator command>"; exit 2; fi
	@echo "------------------------------------------------------------"
	@echo "[$(TEST)] running with: $(SIM)"
	@$(SIM) $(TEST_DIR)/$(TEST).sv > $(LOG_DIR)/$(TEST).log 2>&1; \
	if $(GREP_PASS) $(LOG_DIR)/$(TEST).log; then \
	  echo "[$(TEST)] PASS"; \
	else \
	  echo "[$(TEST)] FAIL - no TEST_PASS marker - log: $(LOG_DIR)/$(TEST).log"; \
	  cat $(LOG_DIR)/$(TEST).log; exit 1; \
	fi

# 2. Run the core suite. 42_severity_fatal has SPECIAL criteria handled below.
run_all:
	@mkdir -p $(LOG_DIR)
	@if [ -z "$(SIM)" ]; then echo "Set SIM=<simulator command>"; exit 2; fi
	@for t in 41_severity_tasks 43_stochastic_queue 44_system 45_real_math_trig; do \
	  $(MAKE) run TEST=$$t SIM="$(SIM)" || exit 1; \
	done
	@echo "------------------------------------------------------------"
	@echo "[42_severity_fatal] running with: $(SIM) (expects \$$fatal to terminate)"
	@$(SIM) $(TEST_DIR)/42_severity_fatal.sv > $(LOG_DIR)/42_severity_fatal.log 2>&1; \
	rc=$$?; \
	if grep -q 'BEFORE_FATAL' $(LOG_DIR)/42_severity_fatal.log \
	   && ! grep -q 'SHOULD_NOT_REACH_HERE' $(LOG_DIR)/42_severity_fatal.log \
	   && ! grep -q 'TEST_PASS_IF_FATAL_DID_NOT_TERMINATE' $(LOG_DIR)/42_severity_fatal.log; then \
	  echo "[42_severity_fatal] PASS (terminated; tool rc=$$rc)"; \
	else \
	  echo "[42_severity_fatal] FAIL (rc=$$rc) - log: $(LOG_DIR)/42_severity_fatal.log"; \
	  cat $(LOG_DIR)/42_severity_fatal.log; exit 1; \
	fi
	@echo "------------------------------------------------------------"
	@echo "Core Clause-20 systf tests passed (41/42/43/44/45)."
	@echo "Note: 46_stacktrace is a newer feature; run 'make run_stacktrace' or 'make run_stacktrace_vl'."

# 3. Optional: $stacktrace (newer LRM feature, may be unsupported by older tools).
#
# Two ways to run it:
#   a) make run_stacktrace_vl            # Verilator 5.x (available out of the box here)
#   b) make run_stacktrace SIM="<your simulator> <flags>" # a NEWER commercial
#                                                       # simulator that implements
#                                                       # the $stacktrace feature

# 3a. Out-of-the-box path: Verilator 5.x (compile + run two-step).
run_stacktrace_vl:
	@mkdir -p $(LOG_DIR)
	@echo "------------------------------------------------------------"
	@echo "[46_stacktrace] building+running with Verilator"
	@rm -rf obj_dir
	@verilator --binary --timing -Wno-DECLFILENAME -I$(TEST_DIR) \
	    --top-module test_stacktrace $(TEST_DIR)/46_stacktrace.sv \
	    > $(LOG_DIR)/46_stacktrace.vl.build.log 2>&1; \
	if [ $$? -ne 0 ]; then \
	  echo "[46_stacktrace] BUILD FAILED - log: $(LOG_DIR)/46_stacktrace.vl.build.log"; \
	  cat $(LOG_DIR)/46_stacktrace.vl.build.log; exit 1; \
	fi
	@./obj_dir/Vtest_stacktrace > $(LOG_DIR)/46_stacktrace.vl.log 2>&1; \
	if $(GREP_PASS) $(LOG_DIR)/46_stacktrace.vl.log; then \
	  echo "[46_stacktrace] PASS (Verilator)"; \
	else \
	  echo "[46_stacktrace] FAIL (Verilator) - log: $(LOG_DIR)/46_stacktrace.vl.log"; \
	  cat $(LOG_DIR)/46_stacktrace.vl.log; exit 1; \
	fi

# 3b. Newer-tool path: any simulator implementing the $stacktrace feature.
run_stacktrace:
	@mkdir -p $(LOG_DIR)
	@if [ -z "$(SIM)" ]; then echo "Set SIM=<simulator command> (or use: make run_stacktrace_vl)"; exit 2; fi
	@echo "------------------------------------------------------------"
	@echo "[46_stacktrace] running with: $(SIM)"
	@echo "(newer LRM feature; older tools reject it at load time)"
	@$(SIM) $(TEST_DIR)/46_stacktrace.sv > $(LOG_DIR)/46_stacktrace.log 2>&1; \
	if $(GREP_PASS) $(LOG_DIR)/46_stacktrace.log; then \
	  echo "[46_stacktrace] PASS"; \
	else \
	  echo "[46_stacktrace] UNSUPPORTED or FAILED - log: $(LOG_DIR)/46_stacktrace.log"; \
	  cat $(LOG_DIR)/46_stacktrace.log; exit 1; \
	fi

clean:
	rm -rf $(LOG_DIR) obj_dir xezim_systf_out.txt
