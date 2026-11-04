"""cocotb runner backend for xezim.

``cocotb_tools.runner.get_runner()`` only knows the simulators cocotb ships
support for, so a harness that calls it directly — NVIDIA's CVDP benchmark
does, in ``test_runner.py`` — cannot select xezim no matter how complete the
VPI surface is. This module supplies the missing :class:`Runner` subclass and
registers it under the name ``xezim``.

Usage::

    import xezim_runner            # registers the backend as a side effect
    from cocotb_tools.runner import get_runner

    runner = get_runner("xezim")
    runner.build(sources=["dut.sv"], hdl_toplevel="dut", parameters={"W": 8})
    runner.test(hdl_toplevel="dut", test_module="test_dut")

or, without touching the harness, set ``SIM=xezim`` and preload this module
via ``PYTHONSTARTUP``/``sitecustomize``/``python -m``.

xezim has no separate elaboration step — it reads sources at run time — so
``build()`` records what a compiling simulator would compile and ``test()``
runs the simulator once on that record. The tests are in ``tests/``.
"""

from __future__ import annotations

import inspect
import json
import os
import shutil
from pathlib import Path
from typing import List, Mapping, Sequence, Union

import cocotb_tools.config
from cocotb_tools.runner import VHDL, Runner, Verilog, _as_sv_literal

PathLike = Union[str, os.PathLike]
_Command = List[str]

#: Binary to invoke. Override with ``XEZIM_BIN`` for an out-of-PATH build.
XEZIM_BIN = os.environ.get("XEZIM_BIN", "xezim")

#: cocotb ships one shared object per simulator, but the VPI ones are not
#: simulator-specific — they are plain IEEE 1800 clause 38 clients. The flavour
#: named below is the plain-VPI build and is what xezim is verified against.
_VPI_LIB_FLAVOUR = os.environ.get("XEZIM_COCOTB_VPI_FLAVOUR", "vcs")


class Xezim(Runner):
    """:class:`Runner` implementation for xezim.

    .. admonition:: Simulator-specific usage

       * There is no compile step. ``build()`` records the sources, include
         directories, defines, ``parameters``, ``build_args`` and
         ``timescale`` in ``xezim_build.json`` in the build directory, and
         ``test()`` runs xezim with them, also from another runner. So
         ``build_args`` apply at run time, and ``always`` has no effect.
         ``hdl_library`` and ``hdl_toplevel_library`` are ignored: xezim keeps
         no libraries.
       * ``parameters`` become ``-gNAME=VALUE``, which sets the default of
         parameter ``NAME`` in every module that declares it overridable; a
         value given at an instantiation still wins. A value is a constant
         expression, such as ``8``, ``8'hff`` or ``{3'd7,3'd5}``; a
         ``string`` parameter also takes plain text. Parameters given to
         ``test()`` replace those given to ``build()`` and remain in effect
         for later tests until replaced or another build is selected.
       * A ``timescale`` given to ``test()`` replaces the one given to
         ``build()``. It applies to modules without a timescale of their own.
       * A define whose value is ``None`` is defined with no value.
       * ``waves`` writes ``<hdl_toplevel>.fst`` to the build directory. xezim
         has no GUI; ``gui`` also writes it, and under pytest cocotb then
         opens it in a waveform viewer.
       * Only Verilog and SystemVerilog are supported.
       * ``pre_cmd`` is not supported.
    """

    supported_gpi_interfaces = {"verilog": ["vpi"]}

    # -- discovery ---------------------------------------------------------

    def _simulator_in_path(self) -> None:
        if shutil.which(XEZIM_BIN) is None and not Path(XEZIM_BIN).is_file():
            raise SystemExit(
                f"ERROR: {XEZIM_BIN} executable not found! "
                "Put it on PATH or set XEZIM_BIN to its full path."
            )

    # -- option translation ------------------------------------------------

    # `-I` and `-D` rather than `+incdir+` and `+define+`, which split at `+`.

    def _get_include_options(self, includes: Sequence[PathLike]) -> _Command:
        return [arg for inc in includes for arg in ("-I", str(inc))]

    def _get_define_options(self, defines: Mapping[str, object]) -> _Command:
        # Values as cocotb's other runners give them, through the same helper:
        # a number as written, a string as a string literal.
        return [
            arg
            for name, value in defines.items()
            for arg in (
                "-D",
                name if value is None else f"{name}={_as_sv_literal(value)}",
            )
        ]

    def _get_parameter_options(self, parameters: Mapping[str, object]) -> _Command:
        return [f"-g{name}={value}" for name, value in parameters.items()]

    # -- artefacts ---------------------------------------------------------

    @property
    def build_file(self) -> Path:
        """Arguments recorded by :meth:`build`, used by :meth:`test`."""
        return self.build_dir / "xezim_build.json"

    @staticmethod
    def _parameter_build_identity(record: Path) -> tuple:
        record = record.resolve()
        if not record.is_file():
            return record, None, None
        # A rewrite resets overrides even when it records the same options.
        # Contents also catch replacements that preserve the timestamp.
        return record, record.stat().st_mtime_ns, record.read_bytes()

    @property
    def _vpi_lib(self) -> Path:
        return Path(cocotb_tools.config.libs_dir) / cocotb_tools.config.lib_name(
            "vpi", _VPI_LIB_FLAVOUR
        )

    def _waves_file(self) -> str:
        return str(self.build_dir / f"{self.sim_hdl_toplevel}.fst")

    def _use_external_viewer(self) -> bool:
        return True

    # -- build -------------------------------------------------------------

    def _build_command(self) -> List[_Command]:
        # Entries are `_ValueAndTag`: `.value` is the path, `.tag` the language.
        sources = self._sources + self._vhdl_sources + self._verilog_sources
        if not sources:
            raise ValueError("xezim needs at least one Verilog/SystemVerilog source")
        for source in sources:
            if source.tag is not Verilog:
                raise ValueError(
                    f"xezim only supports Verilog/SystemVerilog. "
                    f"{str(source.value)!r} cannot be compiled."
                )
        for arg in self._build_args:
            if arg.tag is VHDL:
                raise ValueError(
                    f"xezim only supports Verilog/SystemVerilog. "
                    f"build_args {arg.value!r} cannot be applied."
                )

        # Kept for the command line rather than written as an args file, which
        # splits at whitespace, drops quotes, expands `$VAR` and knows fewer
        # options.
        args = self._get_include_options(self.includes)
        args += self._get_define_options(self.defines)
        if self.timescale is not None:
            args += ["--module-timescale", "{}/{}".format(*self.timescale)]
        args += [str(arg.value) for arg in self._build_args]
        args += [str(source.value) for source in sources]

        record = {
            "args": args,
            "parameters": dict(self.parameters),
        }
        self.build_dir.mkdir(parents=True, exist_ok=True)
        self.build_file.write_text(json.dumps(record, indent=1) + "\n")
        self._parameter_build = self._parameter_build_identity(self.build_file)

        # Nothing to execute: xezim elaborates at run time, so the "build" is
        # the record above. Returning no commands keeps `build()` a no-op
        # rather than inventing a compile that would only be thrown away.
        return []

    # -- test --------------------------------------------------------------

    def test(self, *args, **kwargs) -> Path:
        # Bind against Runner's signature so positional parameters keep the
        # same meaning. Reload a rebuilt record even when another runner
        # wrote it; retain explicit overrides while the record is unchanged.
        bound = inspect.signature(Runner.test).bind(self, *args, **kwargs)
        build_dir = bound.arguments.get("build_dir") or self.build_dir
        record = Path(build_dir).resolve() / "xezim_build.json"
        identity = self._parameter_build_identity(record)
        if getattr(self, "_parameter_build", None) != identity:
            if bound.arguments.get("parameters") is None and identity[2] is not None:
                self.parameters = dict(json.loads(identity[2])["parameters"])
            self._parameter_build = identity
        return super().test(*args, **kwargs)

    def _test_command(self) -> List[_Command]:
        if self.pre_cmd is not None:
            raise RuntimeError("pre_cmd is not implemented for xezim.")

        if not self.build_file.is_file():
            raise FileNotFoundError(
                f"{self.build_file} is missing — call build() before test()."
            )

        # libcocotbvpi_*.so links against libcocotb.so beside it.
        libs_dir = str(cocotb_tools.config.libs_dir)
        ld = self.env.get("LD_LIBRARY_PATH", os.environ.get("LD_LIBRARY_PATH", ""))
        self.env["LD_LIBRARY_PATH"] = f"{libs_dir}:{ld}" if ld else libs_dir

        record = json.loads(self.build_file.read_text())
        cmd: _Command = [XEZIM_BIN, "--sv2017", "--vpi-lib", str(self._vpi_lib)]
        cmd += record["args"]
        cmd += ["-s", self.sim_hdl_toplevel]
        # The last --module-timescale wins, so this replaces build()'s.
        if self.timescale is not None:
            cmd += ["--module-timescale", "{}/{}".format(*self.timescale)]
        if self.waves or self.gui:
            cmd += ["--fst", self._waves_file()]
        cmd += self.elab_args
        cmd += self.test_args
        cmd += self._get_parameter_options(self.parameters)
        cmd += self.plusargs
        return [cmd]


def register() -> None:
    """Teach ``cocotb_tools.runner.get_runner`` about ``"xezim"``.

    Wraps the stock ``get_runner`` rather than editing its table, so it keeps
    working for every simulator cocotb already supports and stays correct if
    that table changes.
    """
    import cocotb_tools.runner as _runner

    if getattr(_runner.get_runner, "_xezim_wrapped", False):
        return

    _stock = _runner.get_runner

    def get_runner(simulator_name: str):
        if simulator_name == "xezim":
            return Xezim()
        return _stock(simulator_name)

    get_runner._xezim_wrapped = True  # type: ignore[attr-defined]
    get_runner.__doc__ = _stock.__doc__
    _runner.get_runner = get_runner
    _runner.Xezim = Xezim  # type: ignore[attr-defined]


register()
