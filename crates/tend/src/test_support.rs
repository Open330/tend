//! Helpers for tests that run fixture executables. Not part of the public
//! API; public only so the other workspace crates' tests can share it.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Create an executable script at `path` without ever holding it open for
/// writing in this process.
///
/// Tests run in parallel threads. If one writes a script while another
/// forks a child, the child inherits the write handle until it execs, and
/// executing the script fails with ETXTBSY ("Text file busy") — seen in
/// 4 of 5 runs of the work-control tests. Writing the content to a side file
/// and letting `cp` create the executable keeps that handle out of this
/// process entirely.
pub fn write_executable(path: &Path, contents: impl AsRef<[u8]>, mode: u32) {
    let source = path.with_file_name(format!(
        ".{}.src",
        path.file_name()
            .map_or_else(Default::default, |name| name.to_string_lossy())
    ));
    std::fs::write(&source, contents).expect("write script source");
    let copied = std::process::Command::new("cp")
        .arg(&source)
        .arg(path)
        .status()
        .expect("spawn cp");
    assert!(
        copied.success(),
        "cp {} {}",
        source.display(),
        path.display()
    );
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod script");
    let _ = std::fs::remove_file(source);
}

/// Replace the executable at `path` with one holding `contents`, the way
/// an install does: [`write_executable`] under a sibling name, then a
/// rename over `path`. Neither this process nor anything polling `path`
/// ever sees it open for writing or half written.
pub fn replace_executable(path: &Path, contents: impl AsRef<[u8]>, mode: u32) {
    let staged = path.with_file_name(format!(
        ".{}.new",
        path.file_name()
            .map_or_else(Default::default, |name| name.to_string_lossy())
    ));
    write_executable(&staged, contents, mode);
    std::fs::rename(&staged, path).expect("rename the replacement into place");
}
