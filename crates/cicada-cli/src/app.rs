//! `cicada app` (v0.1 wave 4, docs/17 Track L1): `serve` plus the app
//! window. The server half IS [`crate::serve`] — `app` takes exactly
//! `serve`'s arguments and resolves them through the same function
//! ([`crate::serve::serve_with`]), so whatever `serve` does with its path
//! argument, `app` does too. What this module adds is the browser: a
//! Chromium-based one in `--app=<url>` mode when the machine has one (a
//! dedicated window without tabs or an address bar — the app window U2
//! asked for), else the default browser on the plain URL; `--no-browser`
//! opens nothing. The URL is printed either way, Ctrl-C stops the server,
//! and the terminal the command runs in is the server console.
//!
//! Discovery is split in two on purpose: [`probe`] looks at the machine
//! (the registry, the usual install directories, `/Applications`) and
//! produces an [`Environment`]; [`choose`] is a pure function from that
//! environment and the URL to the [`Launch`] command, and the unit tests
//! hold it to the contract's table. OS-specific behaviour is DATA the one
//! code path reads ([`HostOs`] from `std::env::consts::OS`), never a
//! `cfg`-gated body — every arm compiles on every OS (AGENTS.md working
//! rule).
//!
//! The window needs something to load: `app` REFUSES, before the server
//! binds, when the binary has no SPA to serve — no `--web-dir` and no
//! embedded build ([`spa_source`]). `cicada serve` is the API-only shape;
//! an app window onto the server's "API only" page is the one thing this
//! command must never open (the first review's finding, 2026-08-24).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::serve::{ServeArgs, serve_with};

/// Where the app window's SPA comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spa {
    /// A built `web/dist` on disk, given as `--web-dir` (its `index.html`
    /// exists). The server prefers it over an embedded SPA, and so does
    /// this rule.
    WebDir(PathBuf),
    /// Baked into the binary (`--features embed`, the release shape).
    Embedded,
}

/// Why `cicada app` has nothing to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoSpa {
    /// `--web-dir` names a directory without an `index.html` — the server
    /// would serve the directory with a 404 where the app should be.
    MissingIndex(PathBuf),
    /// No `--web-dir`, and this build embeds no SPA: the server would
    /// answer `/` with its "API only" page.
    Nothing,
}

impl std::fmt::Display for NoSpa {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingIndex(dir) => write!(
                f,
                "--web-dir {} has no index.html — the app window would load a 404; \
                 run `cd web && npm run build` first",
                dir.display()
            ),
            Self::Nothing => write!(
                f,
                "cicada app has nothing to open: this build embeds no SPA and no --web-dir was given. \
                 Pass --web-dir web/dist (after `cd web && npm run build`) or build with \
                 `cargo build -p cicada-cli --features embed`; `cicada serve` is the API-only shape"
            ),
        }
    }
}

impl std::error::Error for NoSpa {}

/// The rule for what the window loads — a pure function of the arguments,
/// the build (`embedded` = `cfg!(feature = "embed")`) and the disk
/// (`is_file`, injected so the tests need no files). `--web-dir` wins when
/// given and must carry an `index.html` (the server's own preference
/// order); otherwise the embedded SPA; else there is nothing, and
/// `cicada app` refuses before binding anything.
///
/// # Errors
///
/// [`NoSpa`] — the reason, worded for the console.
pub fn spa_source(
    web_dir: Option<&Path>,
    embedded: bool,
    is_file: impl Fn(&Path) -> bool,
) -> Result<Spa, NoSpa> {
    match web_dir {
        Some(dir) if is_file(&dir.join("index.html")) => Ok(Spa::WebDir(dir.to_owned())),
        Some(dir) => Err(NoSpa::MissingIndex(dir.to_owned())),
        None if embedded => Ok(Spa::Embedded),
        None => Err(NoSpa::Nothing),
    }
}

/// Arguments of `cicada app`.
pub struct AppArgs {
    /// Exactly `cicada serve`'s arguments — resolved by `serve`'s code.
    pub serve: ServeArgs,
    /// Print the URL and open nothing.
    pub no_browser: bool,
}

/// The host operating system, as the decision reads it. Built from
/// `std::env::consts::OS` so the decision stays a plain function of data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    /// `windows`.
    Windows,
    /// `macos`.
    MacOs,
    /// `linux`.
    Linux,
    /// Anything else: no browser is opened, the URL is printed.
    Other,
}

impl HostOs {
    /// From `std::env::consts::OS`'s spelling.
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name {
            "windows" => Self::Windows,
            "macos" => Self::MacOs,
            "linux" => Self::Linux,
            _ => Self::Other,
        }
    }

    /// This process's host.
    #[must_use]
    pub fn current() -> Self {
        Self::from_name(std::env::consts::OS)
    }

    /// The name for messages.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::MacOs => "macOS",
            Self::Linux => "Linux",
            Self::Other => std::env::consts::OS,
        }
    }
}

/// The Chromium-based browsers whose `--app` mode makes the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browser {
    /// Microsoft Edge (`msedge.exe`; `Microsoft Edge.app`).
    Edge,
    /// Google Chrome (`chrome.exe`; `Google Chrome.app`).
    Chrome,
}

impl Browser {
    /// The product name — also the application name macOS' `open -a` takes.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Edge => "Microsoft Edge",
            Self::Chrome => "Google Chrome",
        }
    }

    /// The Windows executable's file name — the registry's App Paths key.
    #[must_use]
    pub fn windows_exe(self) -> &'static str {
        match self {
            Self::Edge => "msedge.exe",
            Self::Chrome => "chrome.exe",
        }
    }

    /// Where the Windows installer puts it under a Program Files root.
    fn windows_relative(self) -> &'static str {
        match self {
            Self::Edge => r"Microsoft\Edge\Application\msedge.exe",
            Self::Chrome => r"Google\Chrome\Application\chrome.exe",
        }
    }
}

/// Where the probe found a browser — for the console line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The registry's `App Paths` key (Windows).
    AppPaths,
    /// A usual install directory under Program Files / `%LOCALAPPDATA%`.
    ProgramFiles,
    /// `/Applications` or `~/Applications` (macOS).
    Applications,
}

/// A browser the probe found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// Which one.
    pub browser: Browser,
    /// The executable (Windows) or the `.app` bundle (macOS).
    pub path: PathBuf,
    /// Where it was found.
    pub source: Source,
}

/// Everything [`choose`] reads: the machine, probed once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    /// The host.
    pub os: HostOs,
    /// Every Chromium-based browser the probe found, in probe order; the
    /// decision orders them by the contract's preference, not by this.
    pub installed: Vec<Installed>,
}

/// How the URL is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// A dedicated window in this browser (`--app=<url>`).
    AppWindow(Browser),
    /// The system's default browser, on the plain URL.
    DefaultBrowser,
}

/// The command that opens the app: built by [`choose`], run by [`open`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// The program (an absolute executable, or a name on PATH).
    pub program: PathBuf,
    /// Its arguments.
    pub args: Vec<String>,
    /// What kind of opening this is.
    pub mode: Mode,
}

impl Launch {
    /// The console line printed when it starts.
    #[must_use]
    pub fn describe(&self) -> String {
        match &self.mode {
            Mode::AppWindow(browser) => format!(
                "opening an app window in {} ({})",
                browser.name(),
                self.program.display()
            ),
            Mode::DefaultBrowser => format!(
                "opening the default browser ({} {})",
                self.program.display(),
                self.args.join(" ")
            ),
        }
    }
}

/// The contract's preference per OS: Windows Edge then Chrome; macOS
/// Chrome then Edge.
fn preference(os: HostOs) -> &'static [Browser] {
    match os {
        HostOs::Windows => &[Browser::Edge, Browser::Chrome],
        HostOs::MacOs => &[Browser::Chrome, Browser::Edge],
        HostOs::Linux | HostOs::Other => &[],
    }
}

/// The decision — a pure function of the probed environment and the URL.
///
/// Windows: the first of Edge, Chrome the probe found, run directly with
/// `--app=<url>`; none → `rundll32 url.dll,FileProtocolHandler <url>` (the
/// default browser; unlike `cmd /c start` it takes the URL's `&` as is).
/// macOS: the first of Chrome, Edge found as an application bundle, through
/// `open -na "<name>" --args --app=<url>`; none → `open <url>`. Linux:
/// `xdg-open <url>` (the default browser). Any other OS: `None` — the URL
/// is printed and nothing is opened.
#[must_use]
pub fn choose(env: &Environment, url: &str) -> Option<Launch> {
    let preferred = preference(env.os)
        .iter()
        .find_map(|wanted| env.installed.iter().find(|found| found.browser == *wanted));
    let launch = match (env.os, preferred) {
        (HostOs::Windows, Some(found)) => Launch {
            program: found.path.clone(),
            args: vec![format!("--app={url}")],
            mode: Mode::AppWindow(found.browser),
        },
        (HostOs::Windows, None) => Launch {
            program: PathBuf::from("rundll32"),
            args: vec!["url.dll,FileProtocolHandler".to_owned(), url.to_owned()],
            mode: Mode::DefaultBrowser,
        },
        (HostOs::MacOs, Some(found)) => Launch {
            program: PathBuf::from("open"),
            args: vec![
                "-na".to_owned(),
                found.browser.name().to_owned(),
                "--args".to_owned(),
                format!("--app={url}"),
            ],
            mode: Mode::AppWindow(found.browser),
        },
        (HostOs::MacOs, None) => Launch {
            program: PathBuf::from("open"),
            args: vec![url.to_owned()],
            mode: Mode::DefaultBrowser,
        },
        (HostOs::Linux, _) => Launch {
            program: PathBuf::from("xdg-open"),
            args: vec![url.to_owned()],
            mode: Mode::DefaultBrowser,
        },
        (HostOs::Other, _) => return None,
    };
    Some(launch)
}

/// The `(Default)` value of a `reg query <key> /ve` listing, or `None` when
/// the output carries none (the key is missing — `reg` prints an `ERROR:`
/// line and exits 1). Keyed on the `REG_SZ` / `REG_EXPAND_SZ` type token,
/// never on the `(Default)` label, which `reg` localises.
#[must_use]
pub fn parse_reg_query_default(output: &str) -> Option<PathBuf> {
    output.lines().find_map(|line| {
        let (_, value) = ["REG_EXPAND_SZ", "REG_SZ"]
            .iter()
            .find_map(|token| line.split_once(token))?;
        let value = value.trim();
        (!value.is_empty()).then(|| PathBuf::from(value))
    })
}

/// The usual Windows install locations of a browser, from the environment's
/// Program Files roots (`ProgramFiles`, `ProgramFiles(x86)`, `ProgramW6432`
/// — Edge installs under the x86 root even on 64-bit Windows) and the
/// per-user root (`LOCALAPPDATA`, where Chrome's per-user installer puts
/// it). Pure over `var`; duplicates dropped, order kept.
pub fn windows_usual_paths(browser: Browser, var: impl Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for root in [
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "LOCALAPPDATA",
    ] {
        if let Some(value) = var(root) {
            let path = Path::new(&value).join(browser.windows_relative());
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    paths
}

/// The application bundles macOS installs a browser as: `/Applications`
/// and the user's `~/Applications`.
#[must_use]
pub fn macos_app_bundles(browser: Browser, home: Option<&str>) -> Vec<PathBuf> {
    let bundle = format!("{}.app", browser.name());
    let mut paths = vec![Path::new("/Applications").join(&bundle)];
    if let Some(home) = home {
        paths.push(Path::new(home).join("Applications").join(&bundle));
    }
    paths
}

/// The registry's `App Paths` entry for an executable: `HKLM` then `HKCU`,
/// read through `reg query … /ve` (always present on Windows, no extra
/// dependency); a missing key is simply no entry.
fn registry_app_path(exe: &str) -> Option<PathBuf> {
    ["HKLM", "HKCU"].iter().find_map(|hive| {
        let key = format!(r"{hive}\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{exe}");
        let output = Command::new("reg")
            .args(["query", &key, "/ve"])
            .stdin(Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        parse_reg_query_default(&String::from_utf8_lossy(&output.stdout))
            .filter(|path| path.is_file())
    })
}

/// Look at the machine once. Windows: each browser's App Paths entry, then
/// its usual install directories (only what exists on disk). macOS: the
/// application bundles. Linux and others: nothing to find — the decision
/// uses the default browser.
#[must_use]
pub fn probe(os: HostOs) -> Environment {
    let mut installed = Vec::new();
    match os {
        HostOs::Windows => {
            for browser in [Browser::Edge, Browser::Chrome] {
                if let Some(path) = registry_app_path(browser.windows_exe()) {
                    installed.push(Installed {
                        browser,
                        path,
                        source: Source::AppPaths,
                    });
                }
                for path in windows_usual_paths(browser, |name| std::env::var(name).ok()) {
                    if path.is_file() {
                        installed.push(Installed {
                            browser,
                            path,
                            source: Source::ProgramFiles,
                        });
                    }
                }
            }
        }
        HostOs::MacOs => {
            let home = std::env::var("HOME").ok();
            for browser in [Browser::Chrome, Browser::Edge] {
                for path in macos_app_bundles(browser, home.as_deref()) {
                    if path.is_dir() {
                        installed.push(Installed {
                            browser,
                            path,
                            source: Source::Applications,
                        });
                    }
                }
            }
        }
        HostOs::Linux | HostOs::Other => {}
    }
    Environment { os, installed }
}

/// Start the launch detached: no stdio of ours, not waited for by the
/// server (a thread reaps it). A Chromium launcher that finds its browser
/// already running hands the window over and exits at once; otherwise the
/// process lives as long as the window — either way the server goes on.
///
/// # Errors
///
/// The spawn failure (program missing, not executable).
pub fn open(launch: &Launch) -> std::io::Result<()> {
    let mut child = Command::new(&launch.program)
        .args(&launch.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Run `cicada app`: `serve` with the browser hook.
///
/// The server is the product: a browser that cannot be started is reported
/// loudly on stderr and the server keeps running with its URL on screen —
/// never a silent fallback, never a dead server because a window failed.
///
/// # Errors
///
/// Nothing to open ([`NoSpa`]: no `--web-dir` and no embedded SPA, or a
/// `--web-dir` without an `index.html`) — refused before the server binds;
/// then everything `cicada serve` refuses (bad paths, bind failures, a
/// default pipeline that fails to open).
pub fn app_command(args: &AppArgs) -> anyhow::Result<()> {
    // The server would come up fine without a SPA — and answer `/` with its
    // "API only" page, which is exactly what the window must never show.
    spa_source(
        args.serve.web_dir.as_deref(),
        cfg!(feature = "embed"),
        Path::is_file,
    )?;
    serve_with(&args.serve, "app", |url| {
        if args.no_browser {
            println!("  --no-browser: open the URL above in a browser yourself.");
            return;
        }
        let os = HostOs::current();
        let environment = probe(os);
        match choose(&environment, url) {
            None => println!(
                "  no known way to open a browser on {}; open the URL above yourself.",
                os.name()
            ),
            Some(launch) => match open(&launch) {
                Ok(()) => println!("  {}", launch.describe()),
                Err(error) => eprintln!(
                    "warning: could not start {} ({error}); open the URL above in a browser yourself",
                    launch.program.display()
                ),
            },
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://127.0.0.1:8420/?token=abc&pipeline=02-solids.cic";

    fn found(browser: Browser, path: &str, source: Source) -> Installed {
        Installed {
            browser,
            path: PathBuf::from(path),
            source,
        }
    }

    fn env(os: HostOs, installed: Vec<Installed>) -> Environment {
        Environment { os, installed }
    }

    const EDGE: &str = r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe";
    const CHROME: &str = r"C:\Program Files\Google\Chrome\Application\chrome.exe";

    #[test]
    fn windows_prefers_edge_whatever_the_probe_order() {
        let both = env(
            HostOs::Windows,
            vec![
                found(Browser::Chrome, CHROME, Source::AppPaths),
                found(Browser::Edge, EDGE, Source::ProgramFiles),
            ],
        );
        let launch = choose(&both, URL).unwrap();
        assert_eq!(launch.program, PathBuf::from(EDGE));
        assert_eq!(launch.args, vec![format!("--app={URL}")]);
        assert_eq!(launch.mode, Mode::AppWindow(Browser::Edge));
        assert_eq!(
            launch.describe(),
            format!("opening an app window in Microsoft Edge ({EDGE})")
        );
    }

    #[test]
    fn windows_takes_chrome_when_edge_is_absent() {
        let chrome_only = env(
            HostOs::Windows,
            vec![found(Browser::Chrome, CHROME, Source::ProgramFiles)],
        );
        let launch = choose(&chrome_only, URL).unwrap();
        assert_eq!(launch.program, PathBuf::from(CHROME));
        assert_eq!(launch.args, vec![format!("--app={URL}")]);
        assert_eq!(launch.mode, Mode::AppWindow(Browser::Chrome));
    }

    #[test]
    fn windows_without_a_chromium_browser_opens_the_default_one_on_the_plain_url() {
        let launch = choose(&env(HostOs::Windows, vec![]), URL).unwrap();
        assert_eq!(launch.program, PathBuf::from("rundll32"));
        // The URL is one argument, `&` and all — no shell in between.
        assert_eq!(
            launch.args,
            vec!["url.dll,FileProtocolHandler".to_owned(), URL.to_owned()]
        );
        assert_eq!(launch.mode, Mode::DefaultBrowser);
        assert!(
            launch
                .describe()
                .starts_with("opening the default browser (rundll32 ")
        );
    }

    #[test]
    fn macos_prefers_chrome_then_edge_through_open() {
        let both = env(
            HostOs::MacOs,
            vec![
                found(
                    Browser::Edge,
                    "/Applications/Microsoft Edge.app",
                    Source::Applications,
                ),
                found(
                    Browser::Chrome,
                    "/Applications/Google Chrome.app",
                    Source::Applications,
                ),
            ],
        );
        let launch = choose(&both, URL).unwrap();
        assert_eq!(launch.program, PathBuf::from("open"));
        assert_eq!(
            launch.args,
            vec!["-na", "Google Chrome", "--args", &format!("--app={URL}")]
        );
        assert_eq!(launch.mode, Mode::AppWindow(Browser::Chrome));

        let edge_only = env(
            HostOs::MacOs,
            vec![found(
                Browser::Edge,
                "/Applications/Microsoft Edge.app",
                Source::Applications,
            )],
        );
        let launch = choose(&edge_only, URL).unwrap();
        assert_eq!(
            launch.args,
            vec!["-na", "Microsoft Edge", "--args", &format!("--app={URL}")]
        );
        assert_eq!(launch.mode, Mode::AppWindow(Browser::Edge));
    }

    #[test]
    fn macos_without_a_chromium_browser_opens_the_default_one() {
        let launch = choose(&env(HostOs::MacOs, vec![]), URL).unwrap();
        assert_eq!(launch.program, PathBuf::from("open"));
        assert_eq!(launch.args, vec![URL.to_owned()]);
        assert_eq!(launch.mode, Mode::DefaultBrowser);
    }

    #[test]
    fn linux_is_xdg_open_on_the_plain_url_whatever_was_found() {
        for installed in [
            vec![],
            vec![found(
                Browser::Chrome,
                "/usr/bin/google-chrome",
                Source::ProgramFiles,
            )],
        ] {
            let launch = choose(&env(HostOs::Linux, installed), URL).unwrap();
            assert_eq!(launch.program, PathBuf::from("xdg-open"));
            assert_eq!(launch.args, vec![URL.to_owned()]);
            assert_eq!(launch.mode, Mode::DefaultBrowser);
        }
    }

    #[test]
    fn an_unknown_os_opens_nothing() {
        assert_eq!(choose(&env(HostOs::Other, vec![]), URL), None);
        assert_eq!(HostOs::from_name("freebsd"), HostOs::Other);
        assert_eq!(HostOs::from_name("windows"), HostOs::Windows);
        assert_eq!(HostOs::from_name("macos"), HostOs::MacOs);
        assert_eq!(HostOs::from_name("linux"), HostOs::Linux);
    }

    #[test]
    fn reg_query_output_parses_to_the_default_value() {
        // `reg query "HKLM\...\App Paths\msedge.exe" /ve`, verbatim shape.
        let listing = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\msedge.exe\r\n    (Default)    REG_SZ    C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe\r\n\r\n";
        assert_eq!(parse_reg_query_default(listing), Some(PathBuf::from(EDGE)));
        // A localised label still parses: the type token is the anchor.
        let german = "    (Standard)    REG_SZ    C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe\n";
        assert_eq!(parse_reg_query_default(german), Some(PathBuf::from(CHROME)));
        let expand = "    (Default)    REG_EXPAND_SZ    %ProgramFiles%\\X\\x.exe\n";
        assert_eq!(
            parse_reg_query_default(expand),
            Some(PathBuf::from(r"%ProgramFiles%\X\x.exe"))
        );
        let missing =
            "ERROR: The system was unable to find the specified registry key or value.\r\n";
        assert_eq!(parse_reg_query_default(missing), None);
        assert_eq!(parse_reg_query_default(""), None);
        assert_eq!(
            parse_reg_query_default("    (Default)    REG_SZ    \n"),
            None
        );
    }

    #[test]
    fn windows_usual_paths_follow_the_program_files_roots() {
        let vars = |name: &str| match name {
            "ProgramFiles" | "ProgramW6432" => Some(r"C:\Program Files".to_owned()),
            "ProgramFiles(x86)" => Some(r"C:\Program Files (x86)".to_owned()),
            "LOCALAPPDATA" => Some(r"D:\Profiles\u\AppData\Local".to_owned()),
            _ => None,
        };
        assert_eq!(
            windows_usual_paths(Browser::Edge, vars),
            vec![
                PathBuf::from(r"C:\Program Files\Microsoft\Edge\Application\msedge.exe"),
                PathBuf::from(EDGE),
                PathBuf::from(r"D:\Profiles\u\AppData\Local\Microsoft\Edge\Application\msedge.exe"),
            ]
        );
        assert_eq!(
            windows_usual_paths(Browser::Chrome, vars),
            vec![
                PathBuf::from(CHROME),
                PathBuf::from(r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe"),
                PathBuf::from(r"D:\Profiles\u\AppData\Local\Google\Chrome\Application\chrome.exe"),
            ]
        );
        assert!(windows_usual_paths(Browser::Edge, |_| None).is_empty());
    }

    #[test]
    fn macos_bundles_are_system_and_user_applications() {
        assert_eq!(
            macos_app_bundles(Browser::Chrome, Some("/Volumes/Home/u")),
            vec![
                PathBuf::from("/Applications/Google Chrome.app"),
                PathBuf::from("/Volumes/Home/u/Applications/Google Chrome.app"),
            ]
        );
        assert_eq!(
            macos_app_bundles(Browser::Edge, None),
            vec![PathBuf::from("/Applications/Microsoft Edge.app")]
        );
    }

    #[test]
    fn the_probe_never_invents_a_browser() {
        // On any host, every entry the probe reports exists on disk, and a
        // host without the concept (Linux, others) reports none.
        for entry in &probe(HostOs::current()).installed {
            assert!(entry.path.exists(), "{}", entry.path.display());
        }
        assert!(probe(HostOs::Linux).installed.is_empty());
        assert!(probe(HostOs::Other).installed.is_empty());
    }

    #[test]
    fn the_window_needs_a_spa_web_dir_first_then_the_embedded_one() {
        let dist = PathBuf::from("web/dist");
        let built = |path: &Path| path == Path::new("web/dist/index.html");
        // `--web-dir` with an index.html wins, embedded SPA or not — the
        // server's own preference order.
        assert_eq!(
            spa_source(Some(&dist), false, built),
            Ok(Spa::WebDir(dist.clone()))
        );
        assert_eq!(
            spa_source(Some(&dist), true, built),
            Ok(Spa::WebDir(dist.clone()))
        );
        // A `--web-dir` without one is refused even by an embed build: the
        // server would serve that directory, 404 and all.
        let empty = PathBuf::from("web/empty");
        assert_eq!(
            spa_source(Some(&empty), true, built),
            Err(NoSpa::MissingIndex(empty.clone()))
        );
        let message = NoSpa::MissingIndex(empty).to_string();
        assert!(
            message.contains("web/empty has no index.html") && message.contains("npm run build"),
            "{message}"
        );
        // No `--web-dir`: the embedded SPA, else nothing to open — and the
        // refusal names both ways out and the API-only command.
        assert_eq!(spa_source(None, true, built), Ok(Spa::Embedded));
        assert_eq!(spa_source(None, false, built), Err(NoSpa::Nothing));
        let message = NoSpa::Nothing.to_string();
        for needle in [
            "nothing to open",
            "--web-dir web/dist",
            "--features embed",
            "cicada serve",
        ] {
            assert!(message.contains(needle), "{message}");
        }
    }
}
