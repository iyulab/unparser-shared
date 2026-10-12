//! Update notice and self-update for the family's command-line tools, from GitHub releases.
//!
//! Each tool's CLI used to carry its own copy of this, and the copies had drifted apart.
//! What they share:
//!
//! - **The notice is cheap, and it can be turned off.** A command may ask GitHub for the
//!   latest release at most once a day. The answer is cached, so the other runs of the day
//!   make no network request and wait for nothing. No check is made when the user turns it off
//!   ([`UpdateCheck::start`]'s `disabled`, wired to a `--no-update-check` flag, or the
//!   `<TOOL>_NO_UPDATE_CHECK` / `UNPARSER_NO_UPDATE_CHECK` environment variables), in CI (`CI` is
//!   set), or when stderr, where the notice is printed, is not a terminal.
//! - **The update installs the tool's own archive.** A release carries the CLI archive
//!   `<bin>-<os>-<arch>-v<version>.<ext>` and the C library archive `lib<bin>-…`, whose name
//!   contains the CLI's. Asset names are therefore compared for equality, never by substring.

use colored::Colorize;
use self_update::backends::github::ReleaseList;
use self_update::update::Release;
use semver::Version;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long a cached answer stands before a command asks GitHub again.
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// How long a command that did ask waits for the answer at exit. Once a day at most.
const CHECK_WAIT: Duration = Duration::from_millis(500);

/// The tool an update is for.
#[derive(Debug, Clone, Copy)]
pub struct Tool {
    /// GitHub owner of the repository the releases live in.
    pub repo_owner: &'static str,
    /// GitHub repository name.
    pub repo_name: &'static str,
    /// The executable's name, without an extension; the archives are named after it.
    pub bin_name: &'static str,
    /// The crate `cargo install` installs it from.
    pub crate_name: &'static str,
    /// The running version (`env!("CARGO_PKG_VERSION")` of the CLI crate).
    pub current_version: &'static str,
}

impl Tool {
    /// The environment variable that turns the notice off for this tool alone:
    /// `UNPDF_NO_UPDATE_CHECK` for `unpdf`.
    pub fn opt_out_variable(&self) -> String {
        format!(
            "{}_NO_UPDATE_CHECK",
            self.bin_name.to_ascii_uppercase().replace('-', "_")
        )
    }

    fn command(&self) -> String {
        format!("{} update", self.bin_name)
    }
}

/// The variable that turns the notice off for every tool of the family.
pub const FAMILY_OPT_OUT_VARIABLE: &str = "UNPARSER_NO_UPDATE_CHECK";

/// Why no check is made, if one is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// The caller passed `disabled` — the tool's `--no-update-check` flag.
    Flag,
    /// An opt-out variable is set to something other than empty or `0`.
    Environment,
    /// `CI` is set.
    Ci,
    /// stderr is not a terminal, so nobody would see the notice.
    NotInteractive,
}

/// Whether a run should look for an update, given its environment. `env` reads a variable,
/// `stderr_is_terminal` says where the notice would go.
pub fn skip_reason(
    tool: &Tool,
    disabled: bool,
    env: impl Fn(&str) -> Option<String>,
    stderr_is_terminal: bool,
) -> Option<Skip> {
    let set = |name: &str| env(name).is_some_and(|v| !v.is_empty() && v != "0");
    if disabled {
        Some(Skip::Flag)
    } else if set(&tool.opt_out_variable()) || set(FAMILY_OPT_OUT_VARIABLE) {
        Some(Skip::Environment)
    } else if env("CI").is_some_and(|v| !v.is_empty()) {
        Some(Skip::Ci)
    } else if !stderr_is_terminal {
        Some(Skip::NotInteractive)
    } else {
        None
    }
}

/// A cached answer: when GitHub was last asked, and the latest release it named.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cached {
    checked_at: u64,
    latest: String,
}

impl Cached {
    fn parse(text: &str) -> Option<Self> {
        let (at, latest) = text.trim().split_once(' ')?;
        Some(Self {
            checked_at: at.parse().ok()?,
            latest: latest.to_string(),
        })
    }

    fn render(&self) -> String {
        format!("{} {}\n", self.checked_at, self.latest)
    }

    fn is_fresh(&self, now: u64) -> bool {
        now.saturating_sub(self.checked_at) < CHECK_INTERVAL.as_secs()
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Where the answer is cached: the platform's per-user cache directory, `<tool>/update-check`.
fn cache_file(tool: &Tool) -> Option<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(windows) {
        var("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library").join("Caches"))
    } else {
        var("XDG_CACHE_HOME").or_else(|| var("HOME").map(|home| home.join(".cache")))
    }?;
    Some(base.join(tool.bin_name).join("update-check"))
}

fn read_cache(tool: &Tool) -> Option<Cached> {
    Cached::parse(&std::fs::read_to_string(cache_file(tool)?).ok()?)
}

fn write_cache(tool: &Tool, cached: &Cached) {
    // A cache that cannot be written only means asking again next time.
    if let Some(path) = cache_file(tool) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, cached.render());
    }
}

fn latest_release(tool: &Tool) -> Result<Release, Box<dyn std::error::Error>> {
    let releases = ReleaseList::configure()
        .repo_owner(tool.repo_owner)
        .repo_name(tool.repo_name)
        .build()?
        .fetch()?;
    let latest = releases.latest().ok_or("No releases found on GitHub.")?;
    Ok(latest.clone())
}

fn newer(current: &str, latest: &str) -> bool {
    match (Version::parse(current), Version::parse(latest)) {
        (Ok(current), Ok(latest)) => latest > current,
        _ => false,
    }
}

enum Pending {
    /// A fresh cached answer — nothing to wait for.
    Known(String),
    /// Asking GitHub on a background thread.
    Asking(mpsc::Receiver<Option<String>>),
}

/// A run's update check: started when the command starts, finished when it ends.
pub struct UpdateCheck {
    tool: Tool,
    pending: Pending,
}

impl UpdateCheck {
    /// Start the check for `tool`, or `None` when no check should be made (see
    /// [`skip_reason`]; `disabled` is the tool's `--no-update-check`). A fresh cached answer
    /// is used as it is; otherwise GitHub is asked on a background thread while the command
    /// runs.
    pub fn start(tool: Tool, disabled: bool) -> Option<Self> {
        skip_reason(
            &tool,
            disabled,
            |name| std::env::var(name).ok(),
            std::io::stderr().is_terminal(),
        )
        .is_none()
        .then(|| Self::begin(tool))
    }

    fn begin(tool: Tool) -> Self {
        if let Some(cached) = read_cache(&tool).filter(|c| c.is_fresh(now())) {
            return Self {
                tool,
                pending: Pending::Known(cached.latest),
            };
        }
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let latest = latest_release(&tool)
                .ok()
                .map(|release| release.version().trim_start_matches('v').to_string());
            if let Some(latest) = &latest {
                write_cache(
                    &tool,
                    &Cached {
                        checked_at: now(),
                        latest: latest.clone(),
                    },
                );
            }
            let _ = tx.send(latest);
        });
        Self {
            tool,
            pending: Pending::Asking(rx),
        }
    }

    /// Print the notice on stderr if a newer release exists. Waits at most half a second
    /// for an answer still on its way — on the one run a day that asks.
    ///
    /// stderr, not stdout: commands write document data to stdout, and a notice there would
    /// corrupt piped output.
    pub fn finish(self) {
        let latest = match self.pending {
            Pending::Known(latest) => Some(latest),
            Pending::Asking(rx) => rx.recv_timeout(CHECK_WAIT).ok().flatten(),
        };
        if let Some(latest) = latest.filter(|l| newer(self.tool.current_version, l)) {
            eprintln!();
            eprintln!(
                "{} {} → {} available! Run '{}' to update.",
                "Update:".yellow().bold(),
                self.tool.current_version,
                latest.green(),
                self.tool.command().cyan()
            );
            eprintln!(
                "{}",
                format!(
                    "(--no-update-check or {}=1 turns this check off)",
                    self.tool.opt_out_variable()
                )
                .dimmed()
            );
        }
    }
}

/// Platform info for asset matching.
struct PlatformInfo {
    /// Human-friendly OS name (windows, linux, macos)
    os_name: &'static str,
    /// Human-friendly arch name (x86_64, aarch64, x86_64-musl)
    arch_name: &'static str,
    /// Rust target triple (x86_64-pc-windows-msvc, etc.)
    target_triple: &'static str,
    /// Archive extension (zip for Windows, tar.gz for Unix)
    archive_ext: &'static str,
}

fn current_platform() -> PlatformInfo {
    platform_for(
        std::env::consts::OS,
        std::env::consts::ARCH,
        cfg!(target_env = "musl"),
    )
}

/// The release archive naming for a platform.
///
/// A musl build is its own platform: it is what runs where the glibc build cannot (Alpine,
/// distroless images), so it must update to the musl archive and never to the glibc one,
/// which would install a binary that does not start. Kept apart from the `cfg!` probe so that
/// every platform's answer is testable on any host.
fn platform_for(os: &'static str, arch: &'static str, musl: bool) -> PlatformInfo {
    let (os_name, arch_name, target_triple, archive_ext) = match (os, arch, musl) {
        ("windows", "x86_64", _) => ("windows", "x86_64", "x86_64-pc-windows-msvc", "zip"),
        ("linux", "x86_64", false) => ("linux", "x86_64", "x86_64-unknown-linux-gnu", "tar.gz"),
        ("linux", "x86_64", true) => (
            "linux",
            "x86_64-musl",
            "x86_64-unknown-linux-musl",
            "tar.gz",
        ),
        ("macos", "x86_64", _) => ("macos", "x86_64", "x86_64-apple-darwin", "tar.gz"),
        ("macos", "aarch64", _) => ("macos", "aarch64", "aarch64-apple-darwin", "tar.gz"),
        // No release archive is built for anything else; "unknown" matches no asset.
        (os, arch, _) => (os, arch, "unknown", "tar.gz"),
    };
    PlatformInfo {
        os_name,
        arch_name,
        target_triple,
        archive_ext,
    }
}

/// The CLI archive names to look for, in preference order.
fn asset_patterns(bin: &str, platform: &PlatformInfo, version: &str) -> Vec<String> {
    let v = version.trim_start_matches('v');
    let (os, arch, triple, ext) = (
        platform.os_name,
        platform.arch_name,
        platform.target_triple,
        platform.archive_ext,
    );
    vec![
        // Human-friendly (preferred): <bin>-windows-x86_64-v0.2.0.zip
        format!("{bin}-{os}-{arch}-v{v}.{ext}"),
        format!("{bin}-{os}-{arch}-{v}.{ext}"),
        // Target triple: <bin>-x86_64-pc-windows-msvc-v0.2.0.zip
        format!("{bin}-{triple}-v{v}.{ext}"),
        format!("{bin}-{triple}-{v}.{ext}"),
    ]
}

/// The first of `patterns` that is exactly one of `asset_names`.
fn find_matching_asset(asset_names: &[String], patterns: &[String]) -> Option<String> {
    patterns
        .iter()
        .find(|pattern| asset_names.iter().any(|name| name == *pattern))
        .cloned()
}

/// Whether the running executable was installed by `cargo install` (it lives in `.cargo/bin`).
fn is_cargo_install() -> bool {
    std::env::current_exe().is_ok_and(|exe| {
        let path = exe.to_string_lossy();
        path.contains(".cargo") && path.contains("bin")
    })
}

/// The `update` command: report the latest release and, unless `check_only`, replace the
/// running executable with it. `force` reinstalls even when up to date. A `cargo install`
/// is pointed at `cargo install` instead.
pub fn run_update(
    tool: &Tool,
    check_only: bool,
    force: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let current_version = tool.current_version;
    println!("{} {}", "Current version:".cyan().bold(), current_version);
    println!("{}", "Checking for updates...".cyan());

    let latest = latest_release(tool)?;
    let latest_version = latest.version().trim_start_matches('v').to_string();
    println!("{} {}", "Latest version:".cyan().bold(), latest_version);
    write_cache(
        tool,
        &Cached {
            checked_at: now(),
            latest: latest_version.clone(),
        },
    );

    let current = Version::parse(current_version)?;
    let latest_ver = Version::parse(&latest_version)?;
    if current >= latest_ver && !force {
        println!();
        println!("{} You are running the latest version!", "✓".green().bold());
        return Ok(());
    }
    if current < latest_ver {
        println!();
        println!(
            "{} New version available: {} → {}",
            "↑".yellow().bold(),
            current_version.yellow(),
            latest_version.green().bold()
        );
    }

    let cargo_command = format!("cargo install {}", tool.crate_name);
    if check_only {
        println!();
        let command = if is_cargo_install() {
            cargo_command
        } else {
            tool.command()
        };
        println!("Run '{}' to update.", command.cyan());
        return Ok(());
    }
    if is_cargo_install() {
        println!();
        println!(
            "{} Installed via cargo. Please run:",
            "Note:".yellow().bold()
        );
        println!("  {}", cargo_command.cyan().bold());
        println!();
        println!(
            "{}",
            "This ensures proper integration with your Rust toolchain.".dimmed()
        );
        return Ok(());
    }

    println!();
    println!("{}", "Downloading update...".cyan());
    let patterns = asset_patterns(tool.bin_name, &current_platform(), &latest_version);
    let asset_names: Vec<String> = latest
        .assets()
        .iter()
        .map(|a| a.name().to_string())
        .collect();
    let Some(asset_name) = find_matching_asset(&asset_names, &patterns) else {
        println!("{}", "No matching asset found.".red());
        println!("{}", "Searched for:".dimmed());
        for p in &patterns {
            println!("  - {}", p.dimmed());
        }
        println!();
        println!(
            "{} {}",
            "Available assets:".dimmed(),
            asset_names.join(", ")
        );
        return Err("No compatible binary found for this platform".into());
    };
    println!("{} {}", "Found asset:".dimmed(), asset_name.dimmed());

    // The direct download URL: the API asset URL serves the binary only to a request that
    // carries `Accept: application/octet-stream`, which self_update 1.x can no longer set.
    let download_url = format!(
        "https://github.com/{}/{}/releases/download/v{}/{}",
        tool.repo_owner, tool.repo_name, latest_version, asset_name
    );
    let tmp_dir = tempfile::TempDir::new()?;
    let tmp_archive_path = tmp_dir.path().join(&asset_name);
    let mut tmp_archive = std::fs::File::create(&tmp_archive_path)?;
    let mut download = self_update::Download::from_url(&download_url);
    download.show_download_progress(true);
    download.download_to(&mut tmp_archive)?;

    println!("{}", "Extracting archive...".dimmed());
    let bin_name = format!("{}{}", tool.bin_name, std::env::consts::EXE_SUFFIX);
    self_update::Extract::from_source(&tmp_archive_path).extract_file(tmp_dir.path(), &bin_name)?;
    self_replace::self_replace(tmp_dir.path().join(&bin_name))?;

    println!();
    println!(
        "{} Successfully updated to v{}!",
        "✓".green().bold(),
        latest_version
    );
    println!();
    println!("Restart {} to use the new version.", tool.bin_name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const TOOL: Tool = Tool {
        repo_owner: "iyulab",
        repo_name: "unpdf",
        bin_name: "unpdf",
        crate_name: "unpdf-cli",
        current_version: "0.9.0",
    };

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| vars.get(name).cloned()
    }

    #[test]
    fn an_interactive_run_with_nothing_set_checks() {
        assert_eq!(skip_reason(&TOOL, false, env(&[]), true), None);
    }

    #[test]
    fn every_way_of_turning_the_check_off_is_honoured() {
        assert_eq!(skip_reason(&TOOL, true, env(&[]), true), Some(Skip::Flag));
        for var in ["UNPDF_NO_UPDATE_CHECK", FAMILY_OPT_OUT_VARIABLE] {
            assert_eq!(
                skip_reason(&TOOL, false, env(&[(var, "1")]), true),
                Some(Skip::Environment),
                "{var}"
            );
        }
        assert_eq!(
            skip_reason(&TOOL, false, env(&[("CI", "true")]), true),
            Some(Skip::Ci)
        );
        assert_eq!(
            skip_reason(&TOOL, false, env(&[]), false),
            Some(Skip::NotInteractive)
        );
    }

    /// `VAR=0` and `VAR=` read as «not set», as shells and CI systems write them.
    #[test]
    fn an_opt_out_set_to_zero_or_empty_does_not_turn_the_check_off() {
        for value in ["0", ""] {
            assert_eq!(
                skip_reason(&TOOL, false, env(&[("UNPDF_NO_UPDATE_CHECK", value)]), true),
                None
            );
        }
    }

    #[test]
    fn the_opt_out_variable_is_named_after_the_executable() {
        assert_eq!(TOOL.opt_out_variable(), "UNPDF_NO_UPDATE_CHECK");
        let dashed = Tool {
            bin_name: "my-tool",
            ..TOOL
        };
        assert_eq!(dashed.opt_out_variable(), "MY_TOOL_NO_UPDATE_CHECK");
    }

    #[test]
    fn a_cached_answer_stands_for_a_day() {
        let cached = Cached {
            checked_at: 1_000_000,
            latest: "0.9.1".into(),
        };
        assert_eq!(Cached::parse(&cached.render()), Some(cached.clone()));
        assert!(cached.is_fresh(1_000_000 + 23 * 3600));
        assert!(!cached.is_fresh(1_000_000 + 24 * 3600));
        assert_eq!(Cached::parse("garbage"), None);
    }

    #[test]
    fn only_a_newer_release_is_an_update() {
        assert!(newer("0.9.0", "0.10.0"));
        assert!(!newer("0.9.0", "0.9.0"));
        assert!(!newer("0.10.0", "0.9.0"));
        assert!(!newer("0.9.0", "not-a-version"));
    }

    fn windows_x86_64() -> PlatformInfo {
        platform_for("windows", "x86_64", false)
    }

    /// A release carries two archives whose names differ only by a `lib` prefix: the CLI
    /// executable and the shared library for the language bindings. Substring matching would
    /// let `libunpdf-…` satisfy a search for `unpdf-…`, and the updater would replace the
    /// executable with a library — silently, since both are valid archives.
    #[test]
    fn cli_archive_is_chosen_over_the_library_archive() {
        let patterns = asset_patterns("unpdf", &windows_x86_64(), "0.9.0");
        // Listed library-first, so a match that scans the release in order fails here too.
        let release = vec![
            "libunpdf-windows-x86_64-v0.9.0.zip".to_string(),
            "unpdf-windows-x86_64-v0.9.0.zip".to_string(),
        ];
        assert_eq!(
            find_matching_asset(&release, &patterns).as_deref(),
            Some("unpdf-windows-x86_64-v0.9.0.zip")
        );
    }

    /// Each Linux x86_64 build picks its own archive: a musl build that installed the glibc
    /// one would replace itself with a binary that does not start where it runs.
    #[test]
    fn a_musl_build_updates_to_the_musl_archive() {
        let release = vec![
            "unpdf-linux-x86_64-v0.9.0.tar.gz".to_string(),
            "unpdf-linux-x86_64-musl-v0.9.0.tar.gz".to_string(),
            "libunpdf-linux-x86_64-musl-v0.9.0.tar.gz".to_string(),
        ];
        let musl = asset_patterns("unpdf", &platform_for("linux", "x86_64", true), "0.9.0");
        assert_eq!(
            find_matching_asset(&release, &musl).as_deref(),
            Some("unpdf-linux-x86_64-musl-v0.9.0.tar.gz")
        );
        let gnu = asset_patterns("unpdf", &platform_for("linux", "x86_64", false), "0.9.0");
        assert_eq!(
            find_matching_asset(&release, &gnu).as_deref(),
            Some("unpdf-linux-x86_64-v0.9.0.tar.gz")
        );
    }

    #[test]
    fn a_musl_build_never_falls_back_to_the_glibc_archive() {
        let release = vec!["unpdf-linux-x86_64-v0.9.0.tar.gz".to_string()];
        let musl = asset_patterns("unpdf", &platform_for("linux", "x86_64", true), "0.9.0");
        assert_eq!(find_matching_asset(&release, &musl), None);
    }

    #[test]
    fn each_released_platform_names_its_own_archive() {
        for (os, arch, musl, expected) in [
            (
                "windows",
                "x86_64",
                false,
                "undoc-windows-x86_64-v0.9.0.zip",
            ),
            ("linux", "x86_64", false, "undoc-linux-x86_64-v0.9.0.tar.gz"),
            (
                "linux",
                "x86_64",
                true,
                "undoc-linux-x86_64-musl-v0.9.0.tar.gz",
            ),
            ("macos", "x86_64", false, "undoc-macos-x86_64-v0.9.0.tar.gz"),
            (
                "macos",
                "aarch64",
                false,
                "undoc-macos-aarch64-v0.9.0.tar.gz",
            ),
        ] {
            let patterns = asset_patterns("undoc", &platform_for(os, arch, musl), "0.9.0");
            assert_eq!(patterns[0], expected, "{os} {arch} musl={musl}");
        }
    }

    #[test]
    fn a_library_only_release_matches_nothing() {
        let patterns = asset_patterns("unpdf", &windows_x86_64(), "0.9.0");
        let release = vec!["libunpdf-windows-x86_64-v0.9.0.zip".to_string()];
        assert_eq!(find_matching_asset(&release, &patterns), None);
    }

    #[test]
    fn naming_variants_resolve_in_preference_order() {
        let patterns = asset_patterns("unpdf", &windows_x86_64(), "0.9.0");
        let triple_only = vec!["unpdf-x86_64-pc-windows-msvc-0.9.0.zip".to_string()];
        assert_eq!(
            find_matching_asset(&triple_only, &patterns).as_deref(),
            Some("unpdf-x86_64-pc-windows-msvc-0.9.0.zip")
        );
        let several = vec![
            "unpdf-x86_64-pc-windows-msvc-v0.9.0.zip".to_string(),
            "unpdf-windows-x86_64-v0.9.0.zip".to_string(),
        ];
        assert_eq!(
            find_matching_asset(&several, &patterns).as_deref(),
            Some("unpdf-windows-x86_64-v0.9.0.zip")
        );
    }

    #[test]
    fn a_v_prefixed_tag_produces_the_same_patterns_as_a_bare_version() {
        let platform = windows_x86_64();
        assert_eq!(
            asset_patterns("unpdf", &platform, "v0.9.0"),
            asset_patterns("unpdf", &platform, "0.9.0")
        );
    }

    #[test]
    fn another_platforms_or_versions_archive_is_not_matched() {
        let patterns = asset_patterns("unpdf", &windows_x86_64(), "0.9.0");
        let release = vec![
            "unpdf-linux-x86_64-v0.9.0.tar.gz".to_string(),
            "unpdf-macos-aarch64-v0.9.0.tar.gz".to_string(),
            "unpdf-windows-x86_64-v0.8.0.zip".to_string(),
        ];
        assert_eq!(find_matching_asset(&release, &patterns), None);
    }
}
