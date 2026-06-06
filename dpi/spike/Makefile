# xezim_spike_dpi — DPI-C shim around Spike (riscv-isa-sim).
#
#   make              -> ./xezim_spike_dpi.so (stub mode, no Spike calls)
#   make real         -> .so with XEZIM_SPIKE_REAL=1; links against
#                        libriscv.so + libfesvr.a + libsoftfloat.so
#                        under SPIKE_PREFIX (defaults to /home/bondan/gnu).
#   make clean        -> remove build outputs.

SPIKE_PREFIX ?= /home/bondan/gnu

CXX      ?= g++
CXXFLAGS ?= -O2 -fPIC -std=c++17 -Wall -Wextra
LDFLAGS  ?= -shared

SRC = xezim_spike_dpi.cpp
OUT = xezim_spike_dpi.so

# Real-Spike build adds the include + lib paths and links libriscv.
# Libraries go AFTER the source file (LIBS), search paths + rpath stay
# in front (LDFLAGS_FRONT), so the GNU linker actually resolves the
# Spike symbols (cfg_t::cfg_t, sim_t::sim_t, etc.).
REAL_CXXFLAGS     = -DXEZIM_SPIKE_REAL=1 -I$(SPIKE_PREFIX)/include
REAL_LDFLAGS_FRONT = -L$(SPIKE_PREFIX)/lib -Wl,-rpath,$(SPIKE_PREFIX)/lib
REAL_LIBS         = -lriscv -lfesvr -lsoftfloat

.PHONY: all real clean
all: $(OUT)

$(OUT): $(SRC)
	$(CXX) $(CXXFLAGS) $(LDFLAGS) -o $@ $< $(LIBS)

real: CXXFLAGS += $(REAL_CXXFLAGS)
real: LDFLAGS  += $(REAL_LDFLAGS_FRONT)
real: LIBS     += $(REAL_LIBS)
real: $(OUT)

clean:
	rm -f $(OUT) *.o
