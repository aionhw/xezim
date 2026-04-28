#!/bin/bash
# Run E902 hello in xxxyyy  (Windows host via REMOTE).
# Stages source files at D:\tmp\e902, copies tb.v with optional probes,
# and produces SIM.log with TEST PASSED. Use this as a 3rd reference
# (alongside iverilog) for cycle-by-cycle xezim divergence hunting.
set -eu
SRC="${1:-/tmp/xezim_e902_inc}"
DST="/mnt/d/tmp/e902"
mkdir -p "$DST/incdir"
cp -L "$SRC"/*.v "$SRC"/*.h "$SRC"/case.pat "$SRC"/filelist.f "$DST/" 2>/dev/null
cp -L "$SRC"/incdir/* "$DST/incdir/" 2>/dev/null
cat > "$DST/run_xxxyyy.bat" <<'BAT'
@echo off
D:
cd \tmp\e902
if exist work rmdir /s /q work
"D:\EDA\xxxyyy64_\win64\vlib.exe" work >/dev/null 2>&1
"D:\EDA\xxxyyy64_\win64\COMPILE.exe" -sv -mfcu -timescale=1ns/100ps +incdir+. +incdir+incdir +define+SIMULATION=1 -f filelist.f > COMPILE.log 2>&1
findstr /C:"Errors:" COMPILE.log
echo == compile_done ==
"D:\EDA\xxxyyy64_\win64\SIM.exe" -c -do "run -all; quit -f" tb > SIM.log 2>&1
findstr /R "Hello TEST simulation Error PIPE BMU ARB DBG DECD" SIM.log
BAT
cmd.exe /c "D:\\tmp\\e902\\run_xxxyyy.bat"
