//! `aster-mcp`: Aster's editing engine for MCP clients (Claude Code, Codex
//! and others), over stdio or authenticated Streamable HTTP on loopback.
//!
//! The server edits one project with the editor's own commands: in an
//! editor of its own that saves the file after every edit (Local), or in
//! the Aster window that has the project open (Desktop), where every edit
//! appears at once and the person can undo it.
mod errors;
mod frames;
mod http;
mod jobs;
mod paths;
mod server;
mod time;
mod tools;
mod workspace;

use paths::Roots;
use server::{AsterServer, Shared};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use workspace::{Authority, Workspace};

const USAGE: &str = "\
aster-mcp: Aster's video editor as an MCP server

Usage: aster-mcp [options]

Options:
  --root DIR              A folder the server may read media from and write
                          projects, exports and pictures to; repeat for more.
                          Also ASTER_MCP_ROOTS (separated as PATH is). By
                          default the current folder, unless it is a home,
                          drive or system folder.
  --http [ADDR]           Serve Streamable HTTP on a loopback address
                          (127.0.0.1:47420 by default) instead of stdio. Every
                          request needs `Authorization: Bearer TOKEN`: the
                          token is ASTER_MCP_TOKEN, or a random one; both
                          are written with the address to a private
                          connection file, mcp/PID.json beside the settings.
  --desktop-settings PATH The settings file of the Aster windows to work
                          with (sessions, presets and models beside it).
  --sessions DIR          Where Aster windows publish their live sessions.
  --models DIR            Where speech models are kept (ASTER_MODELS_DIR).
  --aster PATH            The Aster program `desktop {op: open}` launches;
                          the `aster` beside this program by default.
  --help, --version
";

/// What the command line asked for.
#[derive(Debug, Default)]
struct Options {
    roots: Vec<PathBuf>,
    http: Option<String>,
    desktop_settings: Option<PathBuf>,
    sessions: Option<PathBuf>,
    models: Option<PathBuf>,
    aster: Option<PathBuf>,
}

fn parse(args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let mut options = Options::default();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--help" | "-h" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "--version" | "-V" => {
                println!("aster-mcp {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--root" => options.roots.push(PathBuf::from(value("--root")?)),
            "--desktop-settings" => {
                options.desktop_settings = Some(PathBuf::from(value("--desktop-settings")?))
            }
            "--sessions" => options.sessions = Some(PathBuf::from(value("--sessions")?)),
            "--models" => options.models = Some(PathBuf::from(value("--models")?)),
            "--aster" => options.aster = Some(PathBuf::from(value("--aster")?)),
            "--http" => {
                let address = match args.peek() {
                    Some(next) if !next.starts_with("--") => args.next().unwrap_or_default(),
                    _ => http::DEFAULT_ADDRESS.to_owned(),
                };
                options.http = Some(address);
            }
            other => return Err(format!("Unknown argument {other}\n\n{USAGE}")),
        }
    }
    Ok(Some(options))
}

/// Runs the server until its client leaves (stdio) or it is stopped
/// (HTTP), then lets go of the project. Answers the process exit code.
pub fn run() -> i32 {
    let options = match parse(std::env::args().skip(1)) {
        Ok(Some(options)) => options,
        Ok(None) => return 0,
        Err(message) => {
            eprintln!("aster-mcp: {message}");
            return 2;
        }
    };
    // Children (FFmpeg, the Aster window) must not inherit the token.
    let token = std::env::var("ASTER_MCP_TOKEN")
        .ok()
        .filter(|token| !token.is_empty());
    // SAFETY: no other thread runs yet.
    unsafe { std::env::remove_var("ASTER_MCP_TOKEN") };
    let mut named = options.roots.clone();
    if let Some(list) = std::env::var_os("ASTER_MCP_ROOTS") {
        named.extend(std::env::split_paths(&list).filter(|path| !path.as_os_str().is_empty()));
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let roots = match Roots::new(named, &cwd) {
        Ok(roots) => Arc::new(roots),
        Err(message) => {
            eprintln!("aster-mcp: {message}");
            return 2;
        }
    };
    // Where the windows and their presets and models are.
    let settings_dir = options
        .desktop_settings
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .or_else(|| {
            crate::settings::Settings::default_path()
                .and_then(|p| p.parent().map(Path::to_path_buf))
        });
    let sessions = options
        .sessions
        .clone()
        .or_else(|| settings_dir.as_ref().map(|dir| dir.join("sessions")));
    let presets = settings_dir.as_ref().map(|dir| dir.join("presets.json"));
    let connections = settings_dir.as_ref().map(|dir| dir.join("mcp"));
    let models = options
        .models
        .clone()
        .or_else(|| std::env::var_os("ASTER_MODELS_DIR").map(PathBuf::from))
        .or_else(|| crate::speech::models_dir(options.desktop_settings.as_deref()));
    let aster = options.aster.clone().or_else(|| {
        let beside = std::env::current_exe()
            .ok()?
            .with_file_name(format!("aster{}", std::env::consts::EXE_SUFFIX));
        beside.is_file().then_some(beside)
    });

    let mut workspace = Workspace::new(roots.clone(), sessions);
    workspace.desktop_program = aster;
    workspace.desktop_settings = options.desktop_settings.clone();
    let shared = Arc::new(Shared {
        authority: Authority::start(workspace),
        jobs: jobs::Jobs::default(),
        frames: frames::Frames::start(),
        roots: roots.clone(),
        models,
        presets,
        watchers: Mutex::new(Vec::new()),
    });

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_name("aster-mcp")
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("aster-mcp: no runtime: {error}");
            return 1;
        }
    };
    let roots_list: Vec<String> = roots
        .list()
        .iter()
        .map(|r| r.display().to_string())
        .collect();
    let code = runtime.block_on(async {
        tokio::spawn(server::watch(shared.clone()));
        match &options.http {
            Some(address) => {
                eprintln!("aster-mcp: roots {}", roots_list.join(", "));
                match http::serve(shared.clone(), address, token, connections.clone()).await {
                    Ok(()) => 0,
                    Err(message) => {
                        eprintln!("aster-mcp: {message}");
                        1
                    }
                }
            }
            None => match stdio(shared.clone()).await {
                Ok(()) => 0,
                Err(message) => {
                    eprintln!("aster-mcp: {message}");
                    1
                }
            },
        }
    });
    // Jobs stop first, so no helper process outlives the server; then the
    // project's file is saved and its lease and recovery copy go.
    shared.jobs.cancel_all(std::time::Duration::from_secs(2));
    let _ = runtime.block_on(shared.authority.run(|workspace| {
        workspace.shut_down();
        Ok(())
    }));
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
    code
}

/// Serves one client over standard input and output. Standard output is
/// the protocol's alone: whatever else would print there goes to standard
/// error.
async fn stdio(shared: Arc<Shared>) -> Result<(), String> {
    use rmcp::ServiceExt;
    let out = protocol_output().map_err(|error| format!("standard output: {error}"))?;
    let service = AsterServer::new(shared)
        .serve((tokio::io::stdin(), out))
        .await
        .map_err(|error| format!("the client did not start a session: {error}"))?;
    // A client may stop its server with a signal instead of closing its
    // input; either way the project is let go of properly.
    let stop = service.cancellation_token();
    tokio::select! {
        _ = service.waiting() => {}
        _ = http::shutdown_signal() => stop.cancel(),
    }
    Ok(())
}

/// Standard output moved to a descriptor of the protocol's own, with
/// descriptor 1 pointing at standard error from now on.
#[cfg(unix)]
fn protocol_output() -> std::io::Result<tokio::fs::File> {
    use std::os::fd::FromRawFd;
    // SAFETY: plain descriptor calls; the new descriptor is owned by the
    // File made from it and nothing else.
    unsafe {
        let private = libc::fcntl(1, libc::F_DUPFD_CLOEXEC, 3);
        if private < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::dup2(2, 1) < 0 {
            let error = std::io::Error::last_os_error();
            libc::close(private);
            return Err(error);
        }
        Ok(tokio::fs::File::from_std(std::fs::File::from_raw_fd(
            private,
        )))
    }
}

#[cfg(not(unix))]
fn protocol_output() -> std::io::Result<tokio::io::Stdout> {
    Ok(tokio::io::stdout())
}
