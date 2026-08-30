//! ZCode quick launcher: multi-path detection, single-instance launch,
//! window activation, and on-demand status.
//!
//! Design constraints:
//! - **No busy-loop process scanning.** The running-check runs on demand:
//!   when the UI asks for status, right after a launch, and (cheaply) when
//!   the provider hub refreshes the ZCode card. On Windows it is a single
//!   `CreateToolhelp32Snapshot` per check.
//! - If ZCode is already running, launching **activates/focuses** the
//!   existing window instead of spawning a second instance.
//! - The executable is located by probing common install paths (registry
//!   per-user dirs first) plus PATH lookup; the user can override the path
//!   in settings.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::LauncherStatus;

/// How the executable was found.
#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    pub path: PathBuf,
    pub via: &'static str,
}

/// Candidate executables for the ZCode desktop app / CLI, most likely first.
pub fn exe_candidates(home: Option<&Path>) -> Vec<PathBuf> {
    let home = home.map(|h| h.to_path_buf()).or_else(dirs::home_dir).unwrap_or_default();
    let mut v: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        let local = home.join("AppData/Local");
        v.push(local.join("Programs/ZCode/ZCode.exe"));
        v.push(local.join("Programs/zcode/ZCode.exe"));
        v.push(local.join("ZCode/ZCode.exe"));
        v.push(PathBuf::from("C:/Program Files/ZCode/ZCode.exe"));
        v.push(PathBuf::from("C:/Program Files (x86)/ZCode/ZCode.exe"));
        v.push(local.join("Programs/ZCode/zcode.exe"));
    } else {
        v.push(home.join(".local/bin/zcode"));
        v.push(PathBuf::from("/usr/bin/zcode"));
        v.push(PathBuf::from("/usr/local/bin/zcode"));
        v.push(PathBuf::from("/opt/ZCode/zcode"));
    }
    v
}

/// PATH search (bounded, no recursion into subtrees).
pub fn find_on_path(exe_name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(exe_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Resolve the ZCode executable: user override wins, then common paths,
/// then PATH. Returns `None` when nothing plausible exists.
pub fn detect_exe(configured: Option<&str>, home: Option<&Path>) -> Option<Detection> {
    if let Some(cfg) = configured.filter(|s| !s.trim().is_empty()) {
        let p = PathBuf::from(cfg);
        if p.is_file() {
            return Some(Detection { path: p, via: "configured" });
        }
        // A configured-but-missing path is reported as-is (the UI shows an
        // error) — we do not silently fall back to another copy.
        return None;
    }
    for c in exe_candidates(home) {
        if c.is_file() {
            return Some(Detection { path: c, via: "common_path" });
        }
    }
    let exe_name = if cfg!(windows) { "zcode.exe" } else { "zcode" };
    find_on_path(exe_name).map(|path| Detection { path, via: "PATH" })
}

// ---------------------------------------------------------------------------
// Platform process operations
// ---------------------------------------------------------------------------

/// Process/window operations, `#[cfg]`-split so the data layer stays
/// testable on any host.
pub trait ProcessOps {
    /// PIDs whose executable name matches the ZCode binary (one snapshot).
    fn find_pids(&self, exe: &Path) -> Vec<u32>;
    /// Focus/restore the window of one of `pids` (no-op if none).
    fn focus_window(&self, pids: &[u32]) -> bool;
    /// Spawn the exe detached; returns Ok(pid) when the OS accepted it.
    fn spawn(&self, exe: &Path) -> Result<u32, String>;
    /// File-product version of the exe, when readable.
    fn exe_version(&self, exe: &Path) -> Option<String>;
}

/// Probe a PID list by executable name match. Tolerant of case, `.exe`
/// suffix differences (a `/proc` comm name vs a Windows image name) and
/// helper-suffixed variants (`zcode-helper`), but rejects distinct names
/// (`code.exe`, `zcodec.exe`).
pub fn process_match(exe: &Path, process_name: &str) -> bool {
    exe_matches(exe, process_name)
}

fn exe_matches(exe: &Path, process_name: &str) -> bool {
    let file = exe
        .file_name()
        .map(|f| f.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let stem = file.trim_end_matches(".exe");
    let pname = process_name.to_ascii_lowercase();
    let pstem = pname.trim_end_matches(".exe");
    !file.is_empty() && (pname == file || pstem == stem || pstem.starts_with(&format!("{stem}-")) || pstem.starts_with(&format!("{stem} ")))
}

// Linux/dev implementation: /proc scan on demand (what pgrep does), plus
// `--version` probing for the CLI. Windows gets the real implementation in
// `crate::windows::launcher`.
#[cfg(not(windows))]
mod sys {
    use super::*;

    pub struct ProcOps;

    impl ProcessOps for ProcOps {
        fn find_pids(&self, exe: &Path) -> Vec<u32> {
            let mut out = Vec::new();
            let Ok(rd) = std::fs::read_dir("/proc") else { return out };
            for e in rd.flatten() {
                let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
                    continue;
                };
                if let Ok(comm) = std::fs::read_to_string(e.path().join("comm")) {
                    if exe_matches(exe, comm.trim()) {
                        out.push(pid);
                    }
                }
            }
            out.sort_unstable();
            out
        }

        fn focus_window(&self, _pids: &[u32]) -> bool {
            false // no Win32 window management outside Windows
        }

        fn spawn(&self, exe: &Path) -> Result<u32, String> {
            use std::process::{Command, Stdio};
            let child = Command::new(exe)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| e.to_string())?;
            Ok(child.id())
        }

        fn exe_version(&self, exe: &Path) -> Option<String> {
            use std::process::{Command, Stdio};
            let out = Command::new(exe)
                .arg("--version")
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .ok()?;
            let text = String::from_utf8_lossy(&out.stdout);
            // CLI --version output can be mixed with log lines; find the
            // first semver-looking token instead of trusting line order.
            text.split_whitespace()
                .map(|t| t.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.'))
                .find(|t| looks_like_version(t))
                .map(|s| s.to_string())
        }
    }
}

#[cfg(not(windows))]
pub use sys::ProcOps as PlatformProcOps;

#[cfg(windows)]
pub use crate::windows::launcher::WinProcOps as PlatformProcOps;

// ---------------------------------------------------------------------------
/// Result of a launch attempt, mapped 1:1 onto UI copy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LaunchResult {
    /// ZCode was already running; its window was focused.
    Focused,
    /// A new process was started.
    Started { pid: u32 },
    /// Exe not found.
    NotFound,
    /// Spawn failed (path moved, blocked, …).
    Failed(String),
}

pub struct Launcher<P: ProcessOps> {
    pub ops: P,
    pub configured: Option<String>,
    pub home: Option<PathBuf>,
    detection_cache: Option<Detection>,
    version_cache: Option<String>,
}

impl<P: ProcessOps> Launcher<P> {
    pub fn new(ops: P) -> Self {
        Self {
            ops,
            configured: None,
            home: dirs::home_dir(),
            detection_cache: None,
            version_cache: None,
        }
    }

    pub fn configure(&mut self, configured: Option<String>) {
        if self.configured != configured {
            self.detection_cache = None;
            self.version_cache = None;
        }
        self.configured = configured;
    }

    fn detect(&mut self) -> Option<Detection> {
        if let Some(d) = &self.detection_cache {
            if d.path.is_file() || d.via == "configured" {
                return Some(d.clone());
            }
            self.detection_cache = None;
        }
        let d = detect_exe(self.configured.as_deref(), self.home.as_deref())?;
        self.detection_cache = Some(d.clone());
        Some(d)
    }

    /// Current status (one process snapshot when an exe is known).
    pub fn status(&mut self) -> LauncherStatus {
        let Some(det) = self.detect() else {
            return LauncherStatus {
                state: "not_installed".into(),
                exe_path: self.configured.clone(),
                version: None,
                detected_via: self.configured.as_ref().map(|_| "configured".to_string()),
            };
        };
        let running = !self.ops.find_pids(&det.path).is_empty();
        if self.version_cache.is_none() {
            self.version_cache = self.ops.exe_version(&det.path);
        }
        LauncherStatus {
            state: if running { "running" } else { "not_running" }.into(),
            exe_path: Some(det.path.to_string_lossy().into_owned()),
            version: self.version_cache.clone(),
            detected_via: Some(det.via.into()),
        }
    }

    /// Launch or focus. `Starting` is a frontend-side transient state; here
    /// we return the outcome immediately after the OS call.
    pub fn launch(&mut self) -> LaunchResult {
        let Some(det) = self.detect() else {
            return LaunchResult::NotFound;
        };
        let pids = self.ops.find_pids(&det.path);
        if !pids.is_empty() {
            let focused = self.ops.focus_window(&pids);
            let _ = focused; // focus may legitimately fail (no window yet)
            return LaunchResult::Focused;
        }
        match self.ops.spawn(&det.path) {
            Ok(pid) => LaunchResult::Started { pid },
            Err(why) => LaunchResult::Failed(why),
        }
    }

    /// Focus an already-running window without launching.
    pub fn reveal(&mut self) -> LaunchResult {
        let Some(det) = self.detect() else {
            return LaunchResult::NotFound;
        };
        let pids = self.ops.find_pids(&det.path);
        if pids.is_empty() {
            return LaunchResult::NotFound;
        }
        self.ops.focus_window(&pids);
        LaunchResult::Focused
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct FakeOps {
        pids: RefCell<Vec<u32>>,
        spawn_ok: bool,
        focus_called: RefCell<bool>,
        version: Option<String>,
    }
    impl ProcessOps for FakeOps {
        fn find_pids(&self, exe: &Path) -> Vec<u32> {
            assert!(exe.is_file() || exe.to_string_lossy().contains("fake"));
            self.pids.borrow().clone()
        }
        fn focus_window(&self, _p: &[u32]) -> bool {
            *self.focus_called.borrow_mut() = true;
            true
        }
        fn spawn(&self, _exe: &Path) -> Result<u32, String> {
            if self.spawn_ok {
                Ok(4321)
            } else {
                Err("access denied".into())
            }
        }
        fn exe_version(&self, _exe: &Path) -> Option<String> {
            self.version.clone()
        }
    }

    fn fake_launcher(dir: &tempfile::TempDir) -> Launcher<FakeOps> {
        let exe = dir.path().join("zcode");
        std::fs::write(&exe, b"#!/bin/sh\n").unwrap();
        let mut l = Launcher::new(FakeOps {
            pids: RefCell::new(vec![]),
            spawn_ok: true,
            focus_called: RefCell::new(false),
            version: Some("1.2.3".into()),
        });
        l.configured = Some(exe.to_string_lossy().into_owned());
        l
    }

    #[test]
    fn status_not_running_then_running() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = fake_launcher(&dir);
        let s = l.status();
        assert_eq!(s.state, "not_running");
        assert_eq!(s.version.as_deref(), Some("1.2.3"));
        assert_eq!(s.detected_via.as_deref(), Some("configured"));
        l.ops.pids.borrow_mut().push(100);
        assert_eq!(l.status().state, "running");
    }

    #[test]
    fn launch_when_running_focuses_instead() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = fake_launcher(&dir);
        l.ops.pids.borrow_mut().push(7);
        match l.launch() {
            LaunchResult::Focused => {}
            other => panic!("expected focused, got {other:?}"),
        }
        assert!(*l.ops.focus_called.borrow());
    }

    #[test]
    fn launch_starts_and_reports_failure() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = fake_launcher(&dir);
        match l.launch() {
            LaunchResult::Started { pid } => assert_eq!(pid, 4321),
            other => panic!("expected started, got {other:?}"),
        }
        l.ops.spawn_ok = false;
        assert!(matches!(l.launch(), LaunchResult::Failed(_)));
    }

    #[test]
    fn configured_missing_path_is_not_installed() {
        let mut l = Launcher::new(FakeOps {
            pids: RefCell::new(vec![]),
            spawn_ok: true,
            focus_called: RefCell::new(false),
            version: None,
        });
        l.configure(Some("/definitely/not/here/zcode.exe".into()));
        assert_eq!(l.status().state, "not_installed");
        assert_eq!(l.launch(), LaunchResult::NotFound);
    }

    #[test]
    fn exe_name_matching_tolerant() {
        assert!(exe_matches(Path::new("/x/ZCode.exe"), "zcode.exe"));
        assert!(exe_matches(Path::new("/x/zcode"), "zcode"));
        assert!(exe_matches(Path::new("/x/zcode"), "ZCode.exe")); // /proc comm vs exe name
        assert!(!exe_matches(Path::new("/x/zcode"), "code.exe"));
        assert!(!exe_matches(Path::new("/x/zcode"), "zcodec.exe"));
    }

    #[test]
    fn path_search_finds_real_binary_on_dev_hosts() {
        // /bin:/usr/bin is on PATH for every sane host; probe with `sh`.
        let name = if cfg!(windows) { "cmd.exe" } else { "sh" };
        let found = find_on_path(name);
        assert!(found.is_some(), "expected to locate {name} on PATH");
    }
}

/// "1.2.3" / "v1.2.3-beta" style tokens.
pub fn looks_like_version(t: &str) -> bool {
    let core = t.strip_prefix('v').unwrap_or(t);
    let mut parts = core.split('.');
    let first = parts.next().unwrap_or("");
    if first.is_empty() || !first.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let second = parts.next().unwrap_or("");
    if second.is_empty() || !second.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    true
}

#[cfg(test)]
mod version_tests {
    #[test]
    fn version_token_detection() {
        assert!(super::looks_like_version("1.2.3"));
        assert!(super::looks_like_version("v0.51.0"));
        assert!(super::looks_like_version("2.1"));
        assert!(!super::looks_like_version("zcode"));
        assert!(!super::looks_like_version("1"));
        assert!(!super::looks_like_version("1.x.2"));
        assert!(!super::looks_like_version(""));
    }
}
