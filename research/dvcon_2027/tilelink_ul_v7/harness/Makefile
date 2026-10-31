PYTHON ?= python3
ROOT := $(shell pwd)
SEED ?= 123
BIN ?= $(ROOT)/.deps/xezim/target/release/xezim
.PHONY: test pro-check setup stimuli clean-baseline help
help:
	@echo "make test | make pro-check | make setup | make stimuli | make clean-baseline BIN=/abs/path/xezim"
test:
	$(PYTHON) -m unittest discover -s tests -v
pro-check:
	$(PYTHON) tools/check_pro_auth.py
setup:
	bash tools/setup_xezim.sh
stimuli:
	$(PYTHON) tools/generate_stimulus.py --seed $(SEED) --count 96 --out stimulus/seed_$(SEED)
clean-baseline: stimuli
	$(PYTHON) tools/run_sim.py --xezim $(BIN) --stimuli stimulus/seed_$(SEED) --out results/clean_$(SEED)
