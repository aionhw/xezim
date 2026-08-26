# --- Variables ---
TEST        ?= 01_lexical_identifiers
XEZIM     = ../../target/release/xezim
LOG_DIR     = ./logs
TEST_DIR    = ./tests
TEST_DIR_ADV = ./tests_advanced
RTL_DIR     = ./rtl

# Updated Test List (Mapping to filenames in tests/ directory)
TEST_LIST   = 01_lexical_identifiers 02_preprocessor 03_literal_values \
              04_data_types 05_aggregate_types 06_arrays \
              07_operators_expressions 08_assignments 09_control_flow \
              10_tasks_functions 11_modules_ports_params 12_generate \
              13_packages_imports 14_interfaces_modports 15_processes_events \
              16_classes_oop 17_randomization_constraints 18_assertions_basic \
              19_covergroups_basic 20_clocking_blocks

# Advanced tests (from tests_advanced/ directory)
ADVANCED_TEST_LIST = 45_specparam_hierarchical

# Simulation Flags
# -c usually denotes batch/command-line mode
# -l for logging
SIS_FLAGS   =   -l $(LOG_DIR)/$(TEST).log

.PHONY: help run run_all clean

help:
	@echo "Usage:"
	@echo "  make run TEST=<name>       : Run a specific test from tests/ (e.g., 01_lexical_identifiers)"
	@echo "  make run_adv TEST=<name>   : Run a specific test from tests_advanced/ (e.g., 45_specparam_hierarchical)"
	@echo "  make run_all               : Run all 20 core tests + advanced tests"

# 1. Run a single test from tests/
# We pass the RTL files (if they exist) and the specific test file to xezim
run:
	@mkdir -p $(LOG_DIR)
	@echo "Running simulation for $(TEST)..."
	$(XEZIM) $(SIS_FLAGS) $(wildcard $(RTL_DIR)/*.v) $(TEST_DIR)/$(TEST).sv

# 1b. Run a single test from tests_advanced/
run_adv:
	@mkdir -p $(LOG_DIR)
	@echo "Running advanced simulation for $(TEST)..."
	$(XEZIM) $(SIS_FLAGS) $(wildcard $(RTL_DIR)/*.v) $(TEST_DIR_ADV)/$(TEST).sv

# 2. Run all tests in the list (core + advanced)
run_all:
	@mkdir -p $(LOG_DIR)
	@for t in $(TEST_LIST); do \
		echo "------------------------------------------------"; \
		$(MAKE) -f $(firstword $(MAKEFILE_LIST)) run TEST=$$t || exit 1; \
	done
	@for t in $(ADVANCED_TEST_LIST); do \
		echo "------------------------------------------------"; \
		$(MAKE) -f $(firstword $(MAKEFILE_LIST)) run_adv TEST=$$t || exit 1; \
	done
	@echo "------------------------------------------------"
	@echo "All tests completed. Check $(LOG_DIR) for logs."

# 3. Cleanup
clean:
	rm -rf $(LOG_DIR) transcript *.wlf work
