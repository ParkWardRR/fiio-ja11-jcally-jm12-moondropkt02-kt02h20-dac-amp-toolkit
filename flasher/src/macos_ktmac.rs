//! macOS-only: locate and shell out to the `ktmac` companion binary for the native HID unlock.
//!
//! `rusb` cannot claim the normal-mode HID interface on macOS (`IOHIDFamily` owns it) — that
//! part of the platform limitation is real and unfixable from within libusb.
//! [`macos/native/ktmac`](../../macos/native/ktmac) reaches the same device via `IOHIDManager`
//! instead, which needs no interface claim. Confirmed on hardware, 2026‑09‑05
//! (`ROADMAP.md` Appendix D, `docs/MACOS-NATIVE.md`).
//!
//! This is a process boundary, not a reimplementation: the actual `IOHIDDeviceSetReport` call
//! lives only in `ktmac`'s Swift, so there is exactly one implementation of the thing that can
//! leave a dongle's unlock state disturbed, not two.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Find the `ktmac` binary, checked in order:
/// 1. `KTMAC_PATH` env var — explicit override for packaging, testing, or unusual installs.
/// 2. Next to the running `ktflash` binary — the layout once both ship together.
/// 3. On `$PATH`.
///
/// Returns `None` (not an error) if nothing is found — the caller falls back to the `rusb`
/// path, which is the correct behavior on a machine that never built `ktmac`.
pub fn find_ktmac() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("KTMAC_PATH") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("ktmac");
            if sibling.is_file() {
                return Some(sibling);
            }
        }
    }
    which("ktmac")
}

/// Minimal `$PATH` search — std almost has this, so no new dependency for it.
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(name)).find(|p| p.is_file())
}

/// Run `ktmac unlock --send` and reshape its outcome into the same `Result<String, String>`
/// shape [`crate::try_unlock`]'s `rusb` path returns, so callers (CLI + TUI) don't need to know
/// which mechanism actually ran.
///
/// Non-interactive on purpose: `unlock` never needs a confirmation prompt (it's recoverable — a
/// power-cycle undoes it), so `Command::output()` (no inherited stdin/tty) is safe here.
pub fn unlock_via_ktmac(ktmac: &Path) -> Result<String, String> {
    let out = Command::new(ktmac)
        .args(["unlock", "--send"])
        .output()
        .map_err(|e| format!("running {}: {e}", ktmac.display()))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        Ok(format!("(native macOS unlock via ktmac)\n{}", stdout.trim_end()))
    } else {
        let tail = if stderr.trim().is_empty() { String::new() } else { format!("\n{}", stderr.trim_end()) };
        Err(format!("ktmac unlock --send failed:\n{}{tail}", stdout.trim_end()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_finds_a_real_binary_and_not_a_fake_one() {
        // `sh` exists on every macOS box this code targets; a name this specific never does.
        assert!(which("sh").is_some());
        assert!(which("definitely-not-a-real-binary-name-xyz").is_none());
    }

    #[test]
    fn ktmac_path_env_override_is_honored_when_the_file_exists() {
        let dir = std::env::temp_dir().join(format!("ktmac-find-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("ktmac");
        std::fs::write(&fake, b"#!/bin/sh\n").unwrap();

        // SAFETY: this test does not run concurrently with anything else that reads this var —
        // `cargo test` runs each test in its own thread, but env vars are process-global, so a
        // real risk exists if another test also touches KTMAC_PATH. None currently does.
        unsafe { std::env::set_var("KTMAC_PATH", &fake) };
        let found = find_ktmac();
        unsafe { std::env::remove_var("KTMAC_PATH") };

        assert_eq!(found.as_deref(), Some(fake.as_path()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ktmac_path_env_pointing_nowhere_is_ignored_not_trusted() {
        unsafe { std::env::set_var("KTMAC_PATH", "/definitely/not/a/real/path/ktmac") };
        let found = find_ktmac();
        unsafe { std::env::remove_var("KTMAC_PATH") };
        // Must not blindly return the nonexistent override path.
        assert_ne!(found.as_deref(), Some(Path::new("/definitely/not/a/real/path/ktmac")));
    }
}
