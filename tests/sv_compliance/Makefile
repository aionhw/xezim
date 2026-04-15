# --- Variables ---
TEST        ?= 01_lexical_identifiers
SISVSIM     = ../target/release/sisvsim
LOG_DIR     = ./logs
TEST_DIR    = ./tests
RTL_DIR     = ./rtl

# Updated Test List (Mapping to filenames in tests/ directory)
TEST_LIST   = 01_lexical_identifiers 02_preprocessor 03_literal_values \
              04_data_types 05_aggregate_types 06_arrays \
              07_operators_expressions 08_assignments 09_control_flow \
              10_tasks_functions 11_modules_ports_params 12_generate \
              13_packages_imports 14_interfaces_modports 15_processes_events \
              16_classes_oop 17_randomization_constraints 18_assertions_basic \
              19_covergroups_basic 20_clocking_blocks

# Simulation Flags
# -c usually denotes batch/command-line mode
# -l for logging
SIS_FLAGS   =   -l $(LOG_DIR)/$(TEST).log

.PHONY: help run run_all clean

help:
	@echo "Usage:"
	@echo "  make run TEST=<name>  : Run a specific test (e.g., 01_lexical_identifiers)"
	@echo "  make run_all          : Run all 20 tests"

# 1. Run a single test
# We pass the RTL files (if they exist) and the specific test file to sisvsim
run:
	@mkdir -p $(LOG_DIR)
	@echo "Running simulation for $(TEST)..."
	$(SISVSIM) $(SIS_FLAGS) $(wildcard $(RTL_DIR)/*.v) $(TEST_DIR)/$(TEST).sv

# 2. Run all tests in the list
run_all:
	@mkdir -p $(LOG_DIR)
	@for t in $(TEST_LIST); do \
		echo "------------------------------------------------"; \
		$(MAKE) -f $(firstword $(MAKEFILE_LIST)) run TEST=$$t || exit 1; \
	done
	@echo "------------------------------------------------"
	@echo "All tests completed. Check $(LOG_DIR) for logs."

# 3. Cleanup
clean:
	rm -rf $(LOG_DIR) transcript *.wlf work
