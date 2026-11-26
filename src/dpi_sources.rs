//! C and C++ DPI sources named among the design files (IEEE 1800-2023 §35).
//!
//! A testbench's file list often names its DPI-C code next to the RTL
//! (`add.c`, `model.cpp`). Such a file is not SystemVerilog: it is compiled
//! with the host C or C++ compiler, linked with any object files and static
//! archives named the same way into one shared library, and loaded exactly
//! like a `--dpi-lib` library. A shared library named among the files
//! (`.so`, `.dylib`, `.dll`) is loaded as it is.
//!
//! The compiler is the one the DPI export trampoline uses: `$CC` for C and
//! `$CXX` for C++ (either may carry leading arguments, `ccache gcc`), else
//! `clang`/`clang++`, else `cc`/`c++` when Clang is not on `PATH`. xezim's
//! own `svdpi.h`, `vpi_user.h`, `sv_vpi_user.h` and `veriuser.h` are on the
//! include path. `-CFLAGS`/`-LDFLAGS` add compiler and linker arguments.
//!
//! Builds are cached by content under `<cache>/xezim/dpi`. Each source's
//! object is keyed by its PREPROCESSED text (so an edited header misses) and
//! its compile command; the library by those objects, the listed object
//! files' bytes and the link command. A hit skips the compiler, apart from
//! the preprocessing pass. `-LDFLAGS` can name files the key does not see
//! (`./libmodel.a`, a `-l` library on the search path), so with them the
//! library is linked again on every run, from the cached objects, and named
//! by its own bytes as well as the key: two runs that link different
//! libraries never share a path.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What a design-file argument is when it is not HDL source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForeignKind {
    /// A C source: `.c`.
    C,
    /// A C++ source: `.cc`, `.cpp`, `.cxx`, `.c++` or `.C`.
    Cxx,
    /// An object file or static archive (`.o`, `.a`), linked into the library.
    Object,
    /// A shared library (`.so`, `.dylib`, `.dll`), loaded as `--dpi-lib` is.
    SharedLibrary,
}

/// The kind of `path` by its extension, or `None` for HDL source.
pub fn foreign_kind(path: &str) -> Option<ForeignKind> {
    let ext = Path::new(path).extension()?.to_str()?;
    Some(match ext {
        "c" => ForeignKind::C,
        "C" | "cc" | "cpp" | "cxx" | "c++" => ForeignKind::Cxx,
        "o" | "a" => ForeignKind::Object,
        "so" | "dylib" | "dll" => ForeignKind::SharedLibrary,
        _ => return None,
    })
}

/// The non-HDL files of a run, in the order they were named.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ForeignInputs {
    /// C and C++ sources.
    pub sources: Vec<(String, ForeignKind)>,
    /// Object files and static archives.
    pub objects: Vec<String>,
    /// Shared libraries.
    pub shared_libraries: Vec<String>,
}

impl ForeignInputs {
    /// Move every C/C++ source, object, archive and shared library out of
    /// `files`, which keeps its HDL sources in order.
    pub fn take_from(files: &mut Vec<String>) -> Self {
        let mut out = Self::default();
        files.retain(|f| match foreign_kind(f) {
            None => true,
            Some(k @ (ForeignKind::C | ForeignKind::Cxx)) => {
                out.sources.push((f.clone(), k));
                false
            }
            Some(ForeignKind::Object) => {
                out.objects.push(f.clone());
                false
            }
            Some(ForeignKind::SharedLibrary) => {
                out.shared_libraries.push(f.clone());
                false
            }
        });
        out
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty() && self.objects.is_empty() && self.shared_libraries.is_empty()
    }

    /// Whether there is a library to build: a source, object or archive.
    pub fn needs_build(&self) -> bool {
        !self.sources.is_empty() || !self.objects.is_empty()
    }

    /// Every file, for `--dump-files-list`.
    pub fn files(&self) -> impl Iterator<Item = &str> {
        self.sources
            .iter()
            .map(|(f, _)| f.as_str())
            .chain(self.objects.iter().map(String::as_str))
            .chain(self.shared_libraries.iter().map(String::as_str))
    }
}

/// A path `dlopen` takes as a file rather than a name to search the library
/// path for: one without a `/` (`lib.so`) is made relative to `.`.
pub fn loadable_path(path: &str) -> String {
    if path.contains('/') {
        path.to_string()
    } else {
        format!("./{}", path)
    }
}

/// Where built libraries are kept: `xezim/dpi` under `$XEZIM_CACHE_DIR`,
/// `$XDG_CACHE_HOME` or `~/.cache`, as the native-code cache does. With none
/// of them set, a directory of this user's in the temporary directory
/// (`xezim-<uid>` on Unix), which must be private (see `private_dir`): the
/// library built there is loaded into the simulator.
pub fn default_cache_dir() -> Result<PathBuf, String> {
    let set = |v: &str| std::env::var_os(v).filter(|p| !p.is_empty());
    let base = match set("XEZIM_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| set("XDG_CACHE_HOME").map(PathBuf::from))
        .or_else(|| set("HOME").map(|h| PathBuf::from(h).join(".cache")))
    {
        Some(base) => base,
        None => {
            let dir = std::env::temp_dir().join(private_dir_name());
            private_dir(&dir)?;
            dir
        }
    };
    Ok(base.join("xezim").join("dpi"))
}

/// The fallback cache's name in the temporary directory.
#[cfg(unix)]
fn private_dir_name() -> String {
    // SAFETY: geteuid has no preconditions and cannot fail.
    format!("xezim-{}", unsafe { libc::geteuid() })
}

#[cfg(not(unix))]
fn private_dir_name() -> String {
    "xezim".to_string()
}

/// Create `dir` for this user only, or check that the existing one is a
/// directory (not a symbolic link) that this user owns and nobody else can
/// write to. In a shared temporary directory anyone may create the name
/// first, with headers or a library of their own in it.
#[cfg(unix)]
fn private_dir(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    let err = |why: String| {
        format!(
            "the DPI cache {} {}; set $XEZIM_CACHE_DIR to a directory of your own",
            dir.display(),
            why
        )
    };
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(err(format!("cannot be created ({e})"))),
    }
    let meta = std::fs::symlink_metadata(dir).map_err(|e| err(format!("cannot be read ({e})")))?;
    // SAFETY: as above.
    let uid = unsafe { libc::geteuid() };
    if !meta.file_type().is_dir() {
        return Err(err("is not a directory".into()));
    }
    if meta.uid() != uid {
        return Err(err(format!("belongs to another user (uid {})", meta.uid())));
    }
    if meta.mode() & 0o022 != 0 {
        return Err(err(format!(
            "can be written by other users (mode {:o})",
            meta.mode() & 0o777
        )));
    }
    Ok(())
}

/// Elsewhere (Windows) the temporary directory is the user's own.
#[cfg(not(unix))]
fn private_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("cannot create the DPI cache {}: {}", dir.display(), e))
}

/// The headers DPI-C code includes, as xezim ships them in `include/`.
const HEADERS: [(&str, &str); 4] = [
    ("svdpi.h", include_str!("../include/svdpi.h")),
    ("sv_vpi_user.h", include_str!("../include/sv_vpi_user.h")),
    ("vpi_user.h", include_str!("../include/vpi_user.h")),
    ("veriuser.h", include_str!("../include/veriuser.h")),
];

/// The compiler command for one language: `$CC`/`$CXX` split on whitespace,
/// else Clang, else the system's default `cc`/`c++`.
fn compiler(var: &str, clang: &str, fallback: &str) -> Vec<OsString> {
    if let Ok(v) = std::env::var(var) {
        let words: Vec<OsString> = v.split_whitespace().map(Into::into).collect();
        if !words.is_empty() {
            return words;
        }
    }
    let on_path = std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(clang).is_file()));
    vec![if on_path { clang } else { fallback }.into()]
}

fn display(cmd: &[OsString]) -> String {
    cmd.iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Run `cmd`; its diagnostics go straight to the terminal. An error names
/// what was being done.
fn run(cmd: &[OsString], what: &str) -> Result<(), String> {
    match Command::new(&cmd[0]).args(&cmd[1..]).status() {
        Ok(st) if st.success() => Ok(()),
        Ok(st) => Err(format!(
            "{} failed (exit {}): {}",
            what,
            st.code().map_or("signal".to_string(), |c| c.to_string()),
            display(cmd)
        )),
        Err(e) => Err(format!(
            "{}: cannot run `{}` ({}); set $CC/$CXX or put clang or cc on PATH",
            what,
            cmd[0].to_string_lossy(),
            e
        )),
    }
}

/// Write the shipped headers into a directory named by their content, once.
/// An existing copy is used only when every header in it is xezim's own.
fn header_dir(cache: &Path) -> Result<PathBuf, String> {
    let mut h = crate::CacheHash::new();
    for (name, text) in HEADERS {
        h.text(name);
        h.text(text);
    }
    let dir = cache.join(format!("include-{}", &h.finish()[..16]));
    let intact = || {
        HEADERS
            .iter()
            .all(|(n, text)| std::fs::read(dir.join(n)).is_ok_and(|b| b == text.as_bytes()))
    };
    if intact() {
        return Ok(dir);
    }
    // Written beside it and renamed, so a concurrent run never reads a
    // half-written header.
    let tmp = cache.join(format!("include-tmp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(&tmp)?;
        for (name, text) in HEADERS {
            std::fs::write(tmp.join(name), text)?;
        }
        Ok(())
    };
    write().map_err(|e| format!("cannot write DPI headers under {}: {}", cache.display(), e))?;
    // A copy that is not intact (edited, or cut short) is replaced; one
    // another run has put there meanwhile is kept.
    if dir.exists() && !intact() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    if std::fs::rename(&tmp, &dir).is_err() {
        // Another run put them there first.
        let _ = std::fs::remove_dir_all(&tmp);
        if !intact() {
            return Err(format!("cannot write DPI headers to {}", dir.display()));
        }
    }
    Ok(dir)
}

/// Compile the C/C++ sources of `inputs` and link them with its objects and
/// archives into one shared library under `cache`, or reuse the library an
/// earlier run built from the same inputs. `None` when there is nothing to
/// build. `cflags` go to every compile after xezim's own arguments, so they
/// can override them (`-O0 -g`); `ldflags` go last on the link. With
/// `ldflags` the library is linked again every run (they can name files the
/// key does not cover) and its path names its bytes too; the compiled
/// objects are reused either way.
pub fn build_library(
    inputs: &ForeignInputs,
    cflags: &[String],
    ldflags: &[String],
    cache: &Path,
) -> Result<Option<PathBuf>, String> {
    if !inputs.needs_build() {
        return Ok(None);
    }
    std::fs::create_dir_all(cache)
        .map_err(|e| format!("cannot create the DPI cache {}: {}", cache.display(), e))?;
    let include = header_dir(cache)?;
    let cc = compiler("CC", "clang", "cc");
    let cxx = compiler("CXX", "clang++", "c++");
    let flags = |lang: ForeignKind| -> Vec<OsString> {
        let mut cmd = if lang == ForeignKind::Cxx {
            cxx.clone()
        } else {
            cc.clone()
        };
        cmd.extend(["-fPIC", "-O2", "-I"].map(OsString::from));
        cmd.push(include.clone().into());
        cmd.extend(cflags.iter().map(OsString::from));
        cmd
    };

    // Each source's object is keyed by what the source preprocesses to under
    // its exact compile command; the library by those objects, the listed
    // objects' bytes, and the link.
    let mut key = crate::CacheHash::new();
    key.text("dpi-library-2");
    key.text(std::env::consts::OS);
    key.text(std::env::consts::ARCH);
    // (cached object, source, language)
    let mut compiled: Vec<(PathBuf, &str, ForeignKind)> = Vec::new();
    for (src, lang) in &inputs.sources {
        let compile = flags(*lang);
        let mut pp = compile.clone();
        pp.extend(["-E".into(), src.into()]);
        let out = Command::new(&pp[0]).args(&pp[1..]).output().map_err(|e| {
            format!(
                "DPI source '{}': cannot run `{}` ({}); set $CC/$CXX or put clang or cc \
                     on PATH",
                src,
                pp[0].to_string_lossy(),
                e
            )
        })?;
        if !out.status.success() {
            return Err(format!(
                "DPI source '{}' failed to preprocess: {}\n{}",
                src,
                display(&pp),
                String::from_utf8_lossy(&out.stderr).trim_end()
            ));
        }
        let mut obj_key = crate::CacheHash::new();
        obj_key.text("dpi-object-1");
        obj_key.text(std::env::consts::OS);
        obj_key.text(std::env::consts::ARCH);
        obj_key.text(&display(&compile));
        obj_key.bytes(&out.stdout);
        let obj_key = obj_key.finish();
        key.text(&obj_key);
        compiled.push((
            cache.join(format!("obj-{}.o", &obj_key[..24])),
            src.as_str(),
            *lang,
        ));
    }
    for obj in &inputs.objects {
        let bytes =
            std::fs::read(obj).map_err(|e| format!("cannot read DPI object '{}': {}", obj, e))?;
        key.text(obj);
        key.bytes(&bytes);
    }
    let any_cxx = inputs.sources.iter().any(|(_, k)| *k == ForeignKind::Cxx);
    // An object or archive carries no source-language extension. Use the C++
    // driver when available so precompiled C++ models can resolve their
    // runtime symbols. A C-only installation can still link C objects.
    let cxx_driver = std::env::var_os("CXX").is_some_and(|v| !v.is_empty())
        || std::env::var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths).any(|dir| dir.join(&cxx[0]).is_file())
        });
    let mut link = if any_cxx || (!inputs.objects.is_empty() && cxx_driver) {
        cxx.clone()
    } else {
        cc.clone()
    };
    link.extend(["-shared", "-fPIC"].map(OsString::from));
    // Exported SV subroutines and the svdpi/VPI calls resolve when the
    // library is loaded into xezim, not when it is linked.
    if cfg!(target_os = "macos") {
        link.extend(["-undefined", "dynamic_lookup"].map(OsString::from));
        // The install name defaults to the output path, which names this
        // run's build directory; a fixed one keeps the same link's bytes
        // the same from run to run.
        link.push("-Wl,-install_name,xezim-dpi.so".into());
    }
    key.text(&display(&link));
    for f in ldflags {
        key.text(f);
    }
    let key = key.finish();
    let lib = cache.join(format!("dpi-{}.so", &key[..24]));
    // `-LDFLAGS "./libmodel.a"` or `-lmodel`: an edit to that library leaves
    // the key as it was, so the link is not skipped then.
    if ldflags.is_empty() && lib.is_file() {
        return Ok(Some(lib));
    }

    let build = cache.join(format!("build-{}-{}", &key[..24], std::process::id()));
    let _ = std::fs::remove_dir_all(&build);
    std::fs::create_dir_all(&build)
        .map_err(|e| format!("cannot create {}: {}", build.display(), e))?;
    let result = (|| -> Result<PathBuf, String> {
        for (i, (obj, src, lang)) in compiled.iter().enumerate() {
            if obj.is_file() {
                continue;
            }
            let tmp = build.join(format!("{}.o", i));
            let mut cmd = flags(*lang);
            cmd.extend(["-c".into(), (*src).into(), "-o".into(), tmp.clone().into()]);
            run(&cmd, &format!("compiling DPI source '{}'", src))?;
            std::fs::rename(&tmp, obj)
                .map_err(|e| format!("cannot move a DPI object to {}: {}", obj.display(), e))?;
        }
        let tmp = build.join("lib.so");
        link.extend(["-o".into(), tmp.clone().into()]);
        link.extend(compiled.iter().map(|(obj, _, _)| OsString::from(obj)));
        for obj in &inputs.objects {
            // Every member of an archive: nothing in the library references
            // them, the simulator looks them up by name.
            if obj.ends_with(".a") {
                if cfg!(target_os = "macos") {
                    link.push(format!("-Wl,-force_load,{}", obj).into());
                } else {
                    link.extend([
                        "-Wl,--whole-archive".into(),
                        obj.into(),
                        "-Wl,--no-whole-archive".into(),
                    ]);
                }
            } else {
                link.push(obj.into());
            }
        }
        link.extend(ldflags.iter().map(OsString::from));
        run(&link, "linking the DPI library")?;
        // With `ldflags`, runs with the same key can link different
        // libraries (two directories with their own `./libmodel.a`). Each is
        // named by its bytes as well, so no run replaces the library another
        // is about to load, and runs that link the same bytes share one.
        let lib = if ldflags.is_empty() {
            lib
        } else {
            let bytes = std::fs::read(&tmp)
                .map_err(|e| format!("cannot read the DPI library {}: {}", tmp.display(), e))?;
            let mut linked = crate::CacheHash::new();
            linked.bytes(&bytes);
            cache.join(format!("dpi-{}-{}.so", &key[..24], &linked.finish()[..16]))
        };
        std::fs::rename(&tmp, &lib)
            .map_err(|e| format!("cannot move the DPI library to {}: {}", lib.display(), e))?;
        Ok(lib)
    })();
    let _ = std::fs::remove_dir_all(&build);
    result.map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_extension() {
        assert_eq!(foreign_kind("a/add.c"), Some(ForeignKind::C));
        for f in ["m.cc", "m.cpp", "m.cxx", "m.c++", "m.C"] {
            assert_eq!(foreign_kind(f), Some(ForeignKind::Cxx), "{f}");
        }
        assert_eq!(foreign_kind("x.o"), Some(ForeignKind::Object));
        assert_eq!(foreign_kind("libx.a"), Some(ForeignKind::Object));
        assert_eq!(foreign_kind("libx.so"), Some(ForeignKind::SharedLibrary));
        for f in ["t.sv", "t.v", "t.svh", "t.vh", "noext", "t.sv.bak"] {
            assert_eq!(foreign_kind(f), None, "{f}");
        }
    }

    #[test]
    fn take_from_keeps_hdl_order() {
        let mut files: Vec<String> = ["a.sv", "add.c", "b.v", "m.cpp", "x.o", "l.so", "c.svh"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let f = ForeignInputs::take_from(&mut files);
        assert_eq!(files, ["a.sv", "b.v", "c.svh"]);
        assert_eq!(
            f.sources,
            [
                ("add.c".to_string(), ForeignKind::C),
                ("m.cpp".to_string(), ForeignKind::Cxx)
            ]
        );
        assert_eq!(f.objects, ["x.o"]);
        assert_eq!(f.shared_libraries, ["l.so"]);
    }

    #[cfg(unix)]
    #[test]
    fn private_dir_refuses_shared_and_linked_directories() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(format!("xezim_private_dir_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let own = base.join("own");
        private_dir(&own).expect("a new directory");
        let mode = std::fs::metadata(&own).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        private_dir(&own).expect("the same directory again");
        let open = base.join("open");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o775)).unwrap();
        let e = private_dir(&open).unwrap_err();
        assert!(
            e.contains("can be written by other users (mode 775)"),
            "{e}"
        );
        let link = base.join("link");
        std::os::unix::fs::symlink(&own, &link).unwrap();
        let e = private_dir(&link).unwrap_err();
        assert!(e.contains("is not a directory"), "{e}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn bare_library_names_load_as_files() {
        assert_eq!(loadable_path("lib.so"), "./lib.so");
        assert_eq!(loadable_path("d/lib.so"), "d/lib.so");
        assert_eq!(loadable_path("/abs/lib.so"), "/abs/lib.so");
    }
}
