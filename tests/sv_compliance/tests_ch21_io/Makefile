# IEEE 1800-2023 Clause 21 — I/O system tasks/functions (compliance tests)
#
# Run against ANY reference SystemVerilog simulator:
#   make run_all SIM="<your simulator> <flags>"
#   make run_all SIM="/path/to/xezim"
#
# Pass criteria: presence of the "TEST_PASS" marker (word-boundary match).
# Exit code is NOT used (some tests call $error/$warning which inflate counts).

TEST_DIR = .
LOG_DIR  = ./logs

GREP_PASS = grep -Eq '(^|[^A-Za-z_])TEST_PASS([^A-Za-z_]|$$)'

.PHONY: help run run_all clean

help:
	@echo "Targets:"
	@echo "  make run_all SIM=\"<sim cmd>\"           # run all 6 tests"
	@echo "  make run TEST=<name> SIM=\"<sim cmd>\"   # one test"

run:
	@mkdir -p $(LOG_DIR)
	@if [ -z "$(SIM)" ]; then echo "Set SIM=<simulator command>"; exit 2; fi
	@echo "[$(TEST)] running with: $(SIM)"
	@$(SIM) $(TEST_DIR)/$(TEST).sv > $(LOG_DIR)/$(TEST).log 2>&1; \
	if $(GREP_PASS) $(LOG_DIR)/$(TEST).log; then \
	  echo "[$(TEST)] PASS"; \
	else \
	  echo "[$(TEST)] FAIL - log: $(LOG_DIR)/$(TEST).log"; \
	  cat $(LOG_DIR)/$(TEST).log; exit 1; \
	fi

TESTS = 51_format_specs 52_file_radix_output 53_file_read \
        54_memory_file 55_monitor_control 56_vcd_misc

run_all:
	@mkdir -p $(LOG_DIR)
	@if [ -z "$(SIM)" ]; then echo "Set SIM=<simulator command>"; exit 2; fi
	@for t in $(TESTS); do \
	  $(MAKE) run TEST=$$t SIM="$(SIM)" || exit 1; \
	done
	@echo "Clause-21 I/O tests passed."

clean:
	rm -rf $(LOG_DIR) ch21_*.vcd ch21_*.txt ch21_*.hex ch21_*.bin ch21_*.dec work obj_dir *.swp
