"""Tests for xezim_runner.py: each option of cocotb's Runner must reach the
xezim run with the meaning cocotb's other runners give it.

Needs cocotb 2, pytest and an xezim build. From the repository root, after
`cargo build --release`:

    XEZIM_BIN=$PWD/target/release/xezim python -m pytest contrib/cocotb/tests
"""

import importlib.util
import json
import sys
from pathlib import Path

import cocotb_tools.config
import cocotb_tools.runner
import pytest

HERE = Path(__file__).resolve().parent
# runner_checks.py for the simulator (the runner hands sys.path to it), and
# the runner itself.
sys.path[:0] = [str(HERE), str(HERE.parent)]

import xezim_runner  # noqa: E402,F401  registers "xezim"
from cocotb_tools.check_results import get_results  # noqa: E402
from cocotb_tools.runner import VHDL, Verilog, get_runner  # noqa: E402

TOP = "runner_dut"


def build(runner, tmp, **kwargs):
    kwargs.setdefault("hdl_toplevel", TOP)
    runner.build(
        sources=[HERE / "runner_dut.sv"],
        includes=[HERE / "include"],
        build_dir=tmp / "build",
        **kwargs,
    )


def run(runner, tmp, expect, **kwargs):
    """Run runner_checks.py: `expect` maps runner_dut outputs to their values."""
    results = runner.test(
        hdl_toplevel=TOP,
        test_module="runner_checks",
        build_dir=tmp / "build",
        test_dir=tmp / "test",
        extra_env={"EXPECT": json.dumps(expect)},
        **kwargs,
    )
    assert get_results(results) == (1, 0)


def test_parameters(tmp_path):
    """parameters become -g, whose value is a constant expression such as a
    concatenation of sized literals. Parameters given to test() replace those
    given to build()."""
    r = get_runner("xezim")
    build(r, tmp_path, parameters={"W": 8, "P": "{3'd7,3'd5}"})
    run(r, tmp_path, {"w": 8, "p": 61})
    run(r, tmp_path, {"w": 3, "p": 0}, parameters={"W": 3})


def test_build_args(tmp_path):
    """build_args apply at run time; so do Verilog-tagged ones."""
    r = get_runner("xezim")
    build(r, tmp_path, build_args=["-DFOO", Verilog("-DBAR")])
    run(r, tmp_path, {"foo": 1, "bar": 1})


def test_vhdl_is_refused(tmp_path):
    r = get_runner("xezim")
    with pytest.raises(ValueError, match="build_args"):
        build(r, tmp_path, build_args=[VHDL("--std=08")])
    with pytest.raises(ValueError, match="cannot be compiled"):
        r.build(
            sources=[HERE / "runner_dut.sv", tmp_path / "other.vhd"],
            hdl_toplevel=TOP,
            build_dir=tmp_path / "build",
        )


def test_defines_and_includes(tmp_path):
    """A define's value is a number as written or a string as a string literal,
    as with cocotb's other runners; None defines the macro with no value."""
    r = get_runner("xezim")
    build(r, tmp_path, defines={"FOO": None, "MSG": "hi", "NUM": 42})
    run(r, tmp_path, {"foo": 1, "msg": 0x6869, "num": 42, "inc": 5})


def test_timescale(tmp_path):
    """A timescale given to build() outlasts test(); one given to test()
    replaces it."""
    r = get_runner("xezim")
    build(r, tmp_path, timescale=("1ns", "1ps"))
    run(r, tmp_path, {"fine": 1})
    run(r, tmp_path, {"fine": 0}, timescale=("1ns", "1ns"))
    build(r, tmp_path)
    run(r, tmp_path, {"fine": 0})
    run(r, tmp_path, {"fine": 1}, timescale=("1ns", "1ps"))


def test_run_options(tmp_path):
    """elab_args, test_args and plusargs reach the run; waves writes an FST
    dump to the build directory."""
    r = get_runner("xezim")
    build(r, tmp_path)
    run(
        r,
        tmp_path,
        {"foo": 1, "bar": 1, "pa": 9},
        elab_args=["-DFOO"],
        test_args=["-DBAR"],
        plusargs=["+PA=9"],
        waves=True,
    )
    assert (tmp_path / "build" / f"{TOP}.fst").stat().st_size > 0


def test_toplevel_comes_from_test(tmp_path):
    """test() elaborates its hdl_toplevel alone, even when build() named none:
    runner_dut.sv's other module stops the run if it is elaborated too."""
    r = get_runner("xezim")
    build(r, tmp_path, hdl_toplevel=None)
    run(r, tmp_path, {"w": 1})


def test_fresh_runner_uses_the_build(tmp_path):
    """test() on another runner uses what build() recorded, parameters too
    unless test() replaces them."""
    build(
        get_runner("xezim"),
        tmp_path,
        defines={"FOO": None},
        parameters={"W": 8, "P": "{3'd7,3'd5}"},
        build_args=["-DBAR"],
        timescale=("1ns", "1ps"),
    )
    expect = {"foo": 1, "bar": 1, "fine": 1, "w": 8, "p": 61}
    run(get_runner("xezim"), tmp_path, expect, hdl_toplevel_lang="verilog")
    expect.update(w=3, p=0)
    run(
        get_runner("xezim"),
        tmp_path,
        expect,
        hdl_toplevel_lang="verilog",
        parameters={"W": 3},
    )


def test_test_needs_a_build(tmp_path):
    with pytest.raises(FileNotFoundError, match="call build"):
        run(get_runner("xezim"), tmp_path, {}, hdl_toplevel_lang="verilog")


def test_parameter_override_persists(tmp_path):
    r = get_runner("xezim")
    build(r, tmp_path, parameters={"W": 8})
    run(r, tmp_path, {"w": 3}, parameters={"W": 3})
    run(r, tmp_path, {"w": 3})


def test_fresh_runner_override_persists(tmp_path):
    build(get_runner("xezim"), tmp_path, parameters={"W": 8})
    r = get_runner("xezim")
    run(r, tmp_path, {"w": 8}, hdl_toplevel_lang="verilog")
    run(r, tmp_path, {"w": 3}, hdl_toplevel_lang="verilog", parameters={"W": 3})
    run(r, tmp_path, {"w": 3}, hdl_toplevel_lang="verilog")


def test_empty_parameter_override_persists(tmp_path):
    r = get_runner("xezim")
    build(r, tmp_path, parameters={"W": 8})
    run(r, tmp_path, {"w": 3}, parameters={"W": 3})
    run(r, tmp_path, {"w": 1}, parameters={})
    run(r, tmp_path, {"w": 1})


def test_new_build_replaces_parameter_override(tmp_path):
    r = get_runner("xezim")
    build(r, tmp_path, parameters={"W": 8})
    run(r, tmp_path, {"w": 3}, parameters={"W": 3})
    build(r, tmp_path, parameters={"W": 5})
    run(r, tmp_path, {"w": 5})


def test_positional_parameter_override(tmp_path):
    r = get_runner("xezim")
    build(r, tmp_path, parameters={"W": 8})
    # Runner.test accepts its optional parameters positionally as well.
    results = r.test(
        "runner_checks", TOP, "top", "verilog", None, None, None,
        [], [], [], {"EXPECT": json.dumps({"w": 5})}, False, False, {"W": 5},
        build_dir=tmp_path / "build", test_dir=tmp_path / "test",
    )
    assert get_results(results) == (1, 0)


def test_vpi_lib_resolves_with_and_without_lib_name_path(monkeypatch):
    """_vpi_lib uses config.lib_name_path (cocotb 2.1+) and falls back to
    config.libs_dir / config.lib_name (cocotb 2.0.x)."""
    r = get_runner("xezim")
    assert r._vpi_lib.is_file()

    monkeypatch.delattr(cocotb_tools.config, "lib_name_path", raising=False)
    monkeypatch.setattr(
        cocotb_tools.config,
        "lib_name",
        lambda iface, sim: f"stub_{iface}_{sim}.so",
        raising=False,
    )
    expected = (
        Path(cocotb_tools.config.libs_dir)
        / f"stub_vpi_{xezim_runner._VPI_LIB_FLAVOUR}.so"
    )
    assert r._vpi_lib == expected


def test_as_sv_literal_import_fallback(monkeypatch):
    """xezim_runner resolves cocotb_tools.runner.as_sv_literal (2.1+) and
    falls back to _as_sv_literal (2.0.x)."""
    runner_path = Path(xezim_runner.__file__)
    supported = getattr(cocotb_tools.runner, "SUPPORTED_RUNNERS", None)

    for attr, tag in (("_as_sv_literal", "2.0"), ("as_sv_literal", "2.1")):
        with monkeypatch.context() as mp:
            mp.delattr(cocotb_tools.runner, "as_sv_literal", raising=False)
            mp.delattr(cocotb_tools.runner, "_as_sv_literal", raising=False)
            mp.setattr(
                cocotb_tools.runner, attr, lambda v, t=tag: f"{t}:{v}", raising=False
            )
            if isinstance(supported, dict):
                mp.setitem(supported, "xezim", xezim_runner.Xezim)
            spec = importlib.util.spec_from_file_location(
                f"_xezim_runner_probe_{tag}", runner_path
            )
            assert spec is not None and spec.loader is not None
            mod = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(mod)
            assert mod.as_sv_literal("x") == f"{tag}:x"


def test_supported_runners_includes_xezim():
    """cocotb 2.1+ exposes SUPPORTED_RUNNERS; register() adds xezim to it."""
    supported = getattr(cocotb_tools.runner, "SUPPORTED_RUNNERS", None)
    if supported is not None:
        assert supported["xezim"] is xezim_runner.Xezim
