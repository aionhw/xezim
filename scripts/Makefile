# xezim/scripts/Makefile — builds the UVM DPI shared objects loaded by xezim.
#
# Path layout relative to this Makefile:
#   ../include/uvm_dpi_xezim.cc    drop-in DPI driver
#   ../uvm-2017-1.0.so             IEEE 1800.2-2017 output
#   ../uvm-2020.3.1.so             IEEE 1800.2-2020.3.1 output
#   ../../1800.2-2017-1.0/src/dpi  extracted 2017 DPI sources
#   ../../1800.2-2020.3.1/src/dpi  extracted 2020.3.1 DPI sources
#   ../../Accellera-1800.2-2017-1.0.tar.gz  2017 release archive
#   ../../UVM-1800.2-2020.3.1.tar.gz        2020.3.1 release archive
#
# The uvm-core-derived uvm.so is intentionally NOT built here.
#
# Invocation (from xezim/scripts/, or invoke via build_uvm_dpi.sh):
#   make                        # build both .so files
#   make ../uvm-2017-1.0.so     # build only the 2017 variant
#   make fetch                  # download tarballs only
#   make clean                  # remove built .so files
#   make distclean              # also remove tarballs and extracted dirs
#   make help                   # print this text
#
# Environment overrides (standard make vars):
#   CXX       compiler (default: clang++)
#   CXXFLAGS  extra flags appended after the defaults

SHELL := /bin/bash

ifeq ($(origin CXX),default)
CXX      = clang++
endif
CXX      ?= clang++
CXXFLAGS ?=

XEZIM_INCLUDE := ../include
XEZIM_DRIVER  := $(XEZIM_INCLUDE)/uvm_dpi_xezim.cc

# ---- 1800.2-2017-1.0 ----
UVM_2017_DIR     := ../../1800.2-2017-1.0
UVM_2017_TARBALL := ../../Accellera-1800.2-2017-1.0.tar.gz
UVM_2017_URL     := https://www.accellera.org/images/downloads/standards/uvm/Accellera-1800.2-2017-1.0.tar.gz
UVM_2017_SRC     := $(UVM_2017_DIR)/src/dpi
UVM_2017_OUT     := ../uvm-2017-1.0.so

# ---- 1800.2-2020.3.1 ----
UVM_2020_DIR     := ../../1800.2-2020.3.1
UVM_2020_TARBALL := ../../UVM-1800.2-2020.3.1.tar.gz
UVM_2020_URL     := https://www.accellera.org/images/downloads/standards/uvm/UVM-1800.2-2020.3.1.tar.gz
UVM_2020_SRC     := $(UVM_2020_DIR)/src/dpi
UVM_2020_OUT     := ../uvm-2020.3.1.so

.PHONY: all fetch clean distclean help
.DEFAULT_GOAL := all
all: $(UVM_2017_OUT) $(UVM_2020_OUT)

# ---- Fetch -----------------------------------------------------------------
$(UVM_2017_TARBALL):
	@echo "fetching $(UVM_2017_URL)"
	@curl -fL --retry 3 -o $@ $(UVM_2017_URL) \
		|| { rm -f $@; echo "fetch failed: $(UVM_2017_URL)" >&2; exit 1; }

$(UVM_2020_TARBALL):
	@echo "fetching $(UVM_2020_URL)"
	@curl -fL --retry 3 -o $@ $(UVM_2020_URL) \
		|| { rm -f $@; echo "fetch failed: $(UVM_2020_URL)" >&2; exit 1; }

fetch: $(UVM_2017_TARBALL) $(UVM_2020_TARBALL)

# ---- Extract ---------------------------------------------------------------
# `tar -xzf` creates $(UVM_..._DIR) at top level relative to the archive.
# We run tar from the archive's directory so the extracted tree lands at
# the repo root, not inside xezim/scripts/. The marker file is touched so
# Make can detect an existing extraction without re-extracting.
$(UVM_2017_SRC)/uvm_dpi.h: $(UVM_2017_TARBALL)
	@echo "extracting $(UVM_2017_TARBALL)"
	cd $(dir $(UVM_2017_TARBALL)) && tar -xzf $(notdir $(UVM_2017_TARBALL))
	@touch $@

$(UVM_2020_SRC)/uvm_dpi.h: $(UVM_2020_TARBALL)
	@echo "extracting $(UVM_2020_TARBALL)"
	cd $(dir $(UVM_2020_TARBALL)) && tar -xzf $(notdir $(UVM_2020_TARBALL))
	@touch $@

# ---- Compile ---------------------------------------------------------------
# `uvm_hdl_polling.c` ships with the 2020.3.1 source tree but is absent from
# 2017-1.0 (whose uvm_dpi.cc has the include `#ifdef UVM_PLI_POLLING_ENABLE`'d
# out). xezim/include/uvm_dpi_xezim.cc gates its own #include on the
# XEZIM_UVM_POLLING define, so we set it only for the 2020 variant.

$(UVM_2017_OUT): $(XEZIM_DRIVER) $(UVM_2017_SRC)/uvm_dpi.h
	@echo "==> $(UVM_2017_OUT)"
	$(CXX) -shared -fPIC -std=c++17 -Wno-format-security \
		$(CXXFLAGS) \
		-I $(XEZIM_INCLUDE) -I $(UVM_2017_SRC) \
		$(XEZIM_DRIVER) -o $(UVM_2017_OUT)

$(UVM_2020_OUT): $(XEZIM_DRIVER) $(UVM_2020_SRC)/uvm_dpi.h
	@echo "==> $(UVM_2020_OUT)"
	$(CXX) -shared -fPIC -std=c++17 -Wno-format-security \
		-DXEZIM_UVM_POLLING=1 \
		$(CXXFLAGS) \
		-I $(XEZIM_INCLUDE) -I $(UVM_2020_SRC) \
		$(XEZIM_DRIVER) -o $(UVM_2020_OUT)

# ---- Clean -----------------------------------------------------------------
clean:
	rm -f $(UVM_2017_OUT) $(UVM_2020_OUT)

distclean: clean
	rm -rf $(UVM_2017_DIR) $(UVM_2020_DIR)
	rm -f $(UVM_2017_TARBALL) $(UVM_2020_TARBALL)

# ---- Help ------------------------------------------------------------------
help:
	@echo 'Targets:'
	@echo '  all         build both .so files (default)'
	@echo '  fetch       download tarballs only (no extraction/build)'
	@echo '  clean       remove built .so files'
	@echo '  distclean   also remove tarballs and extracted source dirs'
	@echo 'Variables (overridable on the cmdline or via env):'
	@echo '  CXX         compiler to use (default: clang++)'
	@echo '  CXXFLAGS    extra compile flags'
