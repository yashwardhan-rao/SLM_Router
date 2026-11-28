//! `slm-router init`: write the documented example config, never clobbering one by accident.

use std::path::Path;

/// The shipped example config, compiled in so `init` works from a bare binary.
pub const EXAMPLE: &str = include_str!("../slm-router.example.toml");

/// Write `EXAMPLE` to `path`. An existing file is an error unless `force`. The text goes to a temp
/// file in the same directory first and is renamed into place, so a crash or full disk never
/// leaves a half-written config.
pub fn write(path: &Path, force: bool) -> Result<(), String> {
    let shown = path.display();
    // shortcut: exists-then-rename can race a concurrent writer, use a no-clobber link if init ever runs unattended.
    if path.exists() && !force {
        return Err(format!(
            "{shown} already exists; pass --force to overwrite it"
        ));
    }
    if path.is_dir() {
        return Err(format!("{shown} is a directory"));
    }
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    if !dir.is_dir() {
        return Err(format!("directory {} does not exist", dir.display()));
    }
    let name = path
        .file_name()
        .ok_or_else(|| format!("{shown} is not a file path"))?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        name.to_string_lossy(),
        std::process::id()
    ));
    std::fs::write(&tmp, EXAMPLE).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    // std's rename replaces an existing file on Windows too (MoveFileExW with REPLACE_EXISTING),
    // but Windows still refuses over a read-only target; with --force, remove it and retry once.
    let first = std::fs::rename(&tmp, path);
    let second = match first {
        Err(_) if force && path.is_file() => {
            std::fs::remove_file(path).and_then(|()| std::fs::rename(&tmp, path))
        }
        r => r,
    };
    second.map_err(|e| {
        if path.exists() {
            let _ = std::fs::remove_file(&tmp);
            format!("cannot write {shown}: {e}")
        } else {
            // The old file is gone; keep the new text where the user can find it.
            format!(
                "cannot write {shown}: {e}; the new config is in {}",
                tmp.display()
            )
        }
    })
}
