//! `$fopen` handles with buffered writes.
//!
//! A handle on a regular file collects `$fwrite`/`$fdisplay` output in memory
//! and writes it out in large chunks: a per-cycle trace log used to cost one
//! `write` syscall per call. Every point where the buffered bytes could be
//! observed flushes first:
//!   * a read, seek or tell on the same handle (the `Read`/`Seek` impls below);
//!   * `$fflush`, `$fclose`, and dropping the handle;
//!   * anything that may look at the file by path, or outside the simulator:
//!     `$fopen`, `$readmem*`, `$writemem*`, `$system`, a DPI import call,
//!     `$finish` and the end of the run (`Simulator::flush_file_writes`).
//!
//! Two handles open on the same file write through (see `set_write_through`):
//! their interleaving is program order, which separate buffers would reorder.
//! Anything that is not a regular file (a FIFO, a terminal, `/dev/stdout`)
//! is written through as well.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};

/// Bytes collected before a handle writes its buffer out.
const SV_FILE_BUF_CAP: usize = 64 * 1024;

pub(super) struct SvFile {
    file: File,
    buf: Vec<u8>,
    buffered: bool,
    /// (device, inode) of a regular file, to find two handles on one file.
    ident: Option<(u64, u64)>,
    /// The path and `$fopen` mode it was opened with, so a checkpoint can
    /// open it again.
    path: String,
    mode: String,
}

impl SvFile {
    pub(super) fn new(file: File, path: &str, mode: &str) -> Self {
        let ident = regular_file_ident(&file);
        let buffered = ident.is_some() && !path.starts_with("/dev/") && !path.starts_with("/proc/");
        SvFile {
            file,
            buf: Vec::new(),
            buffered,
            ident,
            path: path.to_string(),
            mode: mode.to_string(),
        }
    }

    /// The path and mode the file was opened with.
    pub(super) fn origin(&self) -> (&str, &str) {
        (&self.path, &self.mode)
    }

    pub(super) fn ident(&self) -> Option<(u64, u64)> {
        self.ident
    }

    /// Bytes written by the design that have not reached the file yet.
    pub(super) fn has_pending(&self) -> bool {
        !self.buf.is_empty()
    }

    /// Write out the buffer and stop buffering from here on.
    pub(super) fn set_write_through(&mut self) {
        let _ = self.write_out();
        self.buffered = false;
    }

    /// Hand the buffered bytes to the file.
    pub(super) fn write_out(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let r = self.file.write_all(&self.buf);
        self.buf.clear();
        r
    }
}

/// (device, inode) of an open regular file; None for anything else, and on
/// hosts where it cannot be told (those handles are never buffered).
#[cfg(unix)]
fn regular_file_ident(file: &File) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    file.metadata()
        .ok()
        .filter(|m| m.file_type().is_file())
        .map(|m| (m.dev(), m.ino()))
}

#[cfg(not(unix))]
fn regular_file_ident(_file: &File) -> Option<(u64, u64)> {
    None
}

impl Write for SvFile {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if !self.buffered {
            return self.file.write(data);
        }
        if self.buf.len() + data.len() > SV_FILE_BUF_CAP {
            self.write_out()?;
            if data.len() >= SV_FILE_BUF_CAP {
                return self.file.write(data);
            }
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.write_out()?;
        self.file.flush()
    }
}

impl Read for SvFile {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.write_out()?;
        self.file.read(out)
    }
}

impl Seek for SvFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.write_out()?;
        self.file.seek(pos)
    }
}

impl Drop for SvFile {
    fn drop(&mut self) {
        let _ = self.write_out();
    }
}
