//! bee's developer tasks.
//!
//! Today that is one thing: a visual regression pipeline for the TUI. `viz-web` compiles bee's
//! visual constructs to WebAssembly with [Ratzilla](https://github.com/ratatui/ratzilla), the
//! official ratatui WASM backend; a headless Chrome renders them; Puppeteer screenshots them at
//! fixed time offsets; the PNGs are diffed against baselines committed under `xtask/baselines/`.
//!
//! Nothing here is part of bee's build. This crate is excluded from the workspace, `viz-web` is
//! excluded from *this* crate's workspace, and neither is reachable from any shipping crate — so
//! `cargo build`, `cargo test` and `cargo clippy --workspace --all-targets` are unchanged by its
//! existence. See `xtask/README.md`.

mod serve;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "cargo xtask", about = "bee's developer tasks", version)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

// The shared `Viz` prefix is the point, not an accident: clap derives the subcommand names from
// these variants, and `viz-serve`/`viz-snapshot`/`viz-update` are the commands. They are also a
// family — whatever `cargo xtask` grows next will not be a `viz-` command, and the prefix is what
// will keep the two groups apart.
#[allow(clippy::enum_variant_names)]
#[derive(Subcommand)]
enum Cmd {
    /// Build the visual scenes and open them in a browser, playing in real time.
    VizServe {
        /// Port for `trunk serve`.
        #[arg(long, default_value_t = 8080)]
        port: u16,
        /// Build and serve without opening a browser.
        #[arg(long)]
        no_open: bool,
    },
    /// Screenshot every scene and compare against the committed baselines.
    VizSnapshot {
        /// Restrict to one scene (repeatable). Omit to run all of them.
        #[arg(long = "scene")]
        scenes: Vec<String>,
        /// Reuse an existing `dist/` instead of rebuilding.
        #[arg(long)]
        no_build: bool,
    },
    /// Screenshot every scene and save the results as the new baselines.
    VizUpdate {
        #[arg(long = "scene")]
        scenes: Vec<String>,
        #[arg(long)]
        no_build: bool,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    match Cli::parse().command {
        Cmd::VizServe { port, no_open } => viz_serve(port, no_open),
        Cmd::VizSnapshot { scenes, no_build } => snapshot("compare", &scenes, no_build),
        Cmd::VizUpdate { scenes, no_build } => snapshot("update", &scenes, no_build),
    }
}

// --- Paths ------------------------------------------------------------------------------------
//
// Anchored to this crate's manifest directory, not the process's cwd, so `cargo xtask` behaves the
// same from anywhere in the repo.

fn xtask_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn viz_web_dir() -> PathBuf {
    xtask_dir().join("viz-web")
}

fn dist_dir() -> PathBuf {
    viz_web_dir().join("dist")
}

fn puppeteer_dir() -> PathBuf {
    xtask_dir().join("puppeteer")
}

// --- Subcommands ------------------------------------------------------------------------------

fn viz_serve(port: u16, no_open: bool) -> Result<ExitCode> {
    require_trunk()?;
    require_wasm_target()?;

    let url = format!("http://localhost:{port}");
    println!("serving bee's visual scenes at {url} (Ctrl-C to stop)");

    let mut child = Command::new("trunk")
        .args(["serve", "--port", &port.to_string()])
        .current_dir(viz_web_dir())
        .spawn()
        .context("could not start `trunk serve`")?;

    if !no_open {
        // Best-effort: a browser that will not open is not a reason to tear the server down, and
        // the URL is already on stdout.
        open_browser(&url);
    }

    let status = child.wait().context("`trunk serve` failed")?;
    Ok(if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn snapshot(mode: &str, scenes: &[String], no_build: bool) -> Result<ExitCode> {
    require_node()?;
    require_node_modules()?;

    if no_build {
        if !dist_dir().join("index.html").is_file() {
            bail!(
                "--no-build was given but {} does not exist — run without it first",
                dist_dir().join("index.html").display()
            );
        }
    } else {
        require_trunk()?;
        require_wasm_target()?;
        build_wasm()?;
    }

    let server = serve::serve(&dist_dir())?;
    println!("serving {} at {}", dist_dir().display(), server.base_url());

    let mut cmd = Command::new("node");
    cmd.arg("snapshot.mjs")
        .args(["--mode", mode])
        .args(["--base-url", &server.base_url()])
        .args(["--baseline-dir", &xtask_dir().join("baselines").to_string_lossy()])
        .args(["--out-dir", &xtask_dir().join("shots").to_string_lossy()])
        .current_dir(puppeteer_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    for scene in scenes {
        cmd.args(["--scene", scene]);
    }

    let out = cmd.output().context("could not run snapshot.mjs")?;
    let report = String::from_utf8_lossy(&out.stdout);
    let code = out.status.code().unwrap_or(1);

    // The driver's JSON is the machine-readable record; what follows is the human summary. Parsing
    // it properly would mean a serde_json dependency for a handful of fields, so the report is
    // echoed in full whenever anything went wrong and summarized otherwise.
    match code {
        0 => {
            let n = report.matches(r#""status": "ok""#).count()
                + report.matches(r#""status": "written""#).count();
            println!(
                "{} {n} screenshots",
                if mode == "update" { "wrote" } else { "matched" }
            );
            Ok(ExitCode::SUCCESS)
        }
        1 => {
            println!("{report}");
            println!(
                "\nvisual drift — the report above lists each changed scene, with actual/diff PNGs\n\
                 under {}. If the change is intended: cargo xtask viz-update",
                xtask_dir().join("shots").display()
            );
            Ok(ExitCode::FAILURE)
        }
        _ => {
            println!("{report}");
            bail!("snapshot.mjs failed (exit {code})");
        }
    }
}

fn build_wasm() -> Result<()> {
    println!("building the WASM scenes (trunk build --release)");
    let status = Command::new("trunk")
        .args(["build", "--release"])
        .current_dir(viz_web_dir())
        .status()
        .context("could not run `trunk build`")?;
    if !status.success() {
        bail!("`trunk build --release` failed");
    }
    Ok(())
}

// --- Prerequisites ----------------------------------------------------------------------------
//
// trunk, the wasm32 target and Node are external tools, deliberately: making them Cargo
// dependencies would drag a toolchain into the repo for a job the developer's machine already does.
// The cost of that choice is that their absence must produce an instruction rather than a
// backtrace, which is what these checks are.

fn on_path(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn require_trunk() -> Result<()> {
    if on_path("trunk") {
        return Ok(());
    }
    bail!("trunk not found — install with: cargo install --locked trunk");
}

fn require_node() -> Result<()> {
    if on_path("node") {
        return Ok(());
    }
    bail!("node not found — install Node.js 18 or newer (https://nodejs.org)");
}

fn require_wasm_target() -> Result<()> {
    let out = Command::new("rustc")
        .args(["--print", "target-list"])
        .output();
    // If rustc cannot be asked, let trunk produce the real error rather than inventing one here.
    let Ok(out) = out else { return Ok(()) };
    if !out.status.success() {
        return Ok(());
    }
    // `--print target-list` lists every *known* target, not the installed ones, so probe the
    // sysroot instead: that is where `rustup target add` puts the std artifacts.
    let sysroot = Command::new("rustc").args(["--print", "sysroot"]).output();
    let Ok(sysroot) = sysroot else { return Ok(()) };
    let sysroot = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();
    let installed = Path::new(&sysroot)
        .join("lib/rustlib/wasm32-unknown-unknown")
        .is_dir();
    if installed {
        return Ok(());
    }
    bail!("the wasm32-unknown-unknown target is not installed — add it with: rustup target add wasm32-unknown-unknown");
}

fn require_node_modules() -> Result<()> {
    if puppeteer_dir().join("node_modules").is_dir() {
        return Ok(());
    }
    bail!(
        "Puppeteer is not installed — install it with: (cd {} && npm install)",
        puppeteer_dir().display()
    );
}

fn open_browser(url: &str) {
    // No `open` crate: three candidate commands cover Linux, macOS and WSL, and a dev tool should
    // not take a dependency to run one of them.
    for (program, args) in [
        ("xdg-open", vec![url]),
        ("open", vec![url]),
        ("wslview", vec![url]),
    ] {
        let ok = Command::new(program)
            .args(&args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return;
        }
    }
    eprintln!("could not open a browser — visit {url} yourself");
}
