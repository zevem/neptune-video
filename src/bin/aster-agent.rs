//! Newline-delimited agent API for a headless project or a live desktop session.
use aster_editor::headless::{configure_save_as, execute, open_headless};
use editor_agent::{AgentRequest, AgentResponse, SessionDescriptor};
use editor_core::Editor;
use std::{
    io::{self, BufRead, Read, Write},
    path::PathBuf,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("aster-agent: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut project_path = None;
    let mut connect_path = None;
    let mut create = false;
    let mut recover = false;
    let mut save_as = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--project" => {
                project_path = Some(PathBuf::from(args.next().ok_or("--project needs a path")?))
            }
            "--connect" => {
                connect_path = Some(PathBuf::from(
                    args.next()
                        .ok_or("--connect needs a session descriptor path")?,
                ))
            }
            "--create" => create = true,
            "--recover" => recover = true,
            "--save-as" => {
                save_as = Some(PathBuf::from(args.next().ok_or("--save-as needs a path")?));
            }
            "--help" | "-h" => {
                println!(
                    "aster-agent [--project FILE [--create|--recover] [--save-as NEWPATH]] | [--connect SESSION.json]\nReads AgentRequest JSON lines on stdin and writes AgentResponse lines on stdout.\nMutations require expected_revision, expected_project_id and expected_context_id from inspect. With no arguments, use an ephemeral headless session.\n--create and --save-as refuse existing files. Legacy/unbound recovery is inspectable but requires --save-as before durable edits. A failed project save restores the prior editing state and renews context; inspect again before retrying.\nRender jobs (export_capabilities, queue_export, render_jobs and the job operations) run in the desktop editor; use --connect for them."
                );
                return Ok(());
            }
            _ => return Err(format!("Unknown argument {arg}").into()),
        }
    }
    if project_path.is_some() && connect_path.is_some() {
        return Err("Choose either a headless project or a live connection".into());
    }
    if create && recover {
        return Err("Choose either --create or --recover".into());
    }
    if (create || recover || save_as.is_some()) && project_path.is_none() {
        return Err("--create, --recover and --save-as require --project".into());
    }
    if create && save_as.is_some() {
        return Err("Choose either --create or --save-as".into());
    }
    let descriptor: Option<SessionDescriptor> = connect_path
        .map(|p| std::fs::read(p).map(|bytes| serde_json::from_slice(&bytes)))
        .transpose()?
        .transpose()?;
    let (mut editor, mut document) = if let Some(path) = &project_path {
        let (editor, document) = open_headless(path, create, recover)?;
        (editor, Some(document))
    } else {
        (Editor::default(), None)
    };
    if let (Some(path), Some(document)) = (save_as, document.as_mut())
        && let Some(error) = configure_save_as(&mut editor, document, &path)?
    {
        eprintln!("aster-agent: Project saved; session recovery failed: {error}");
    }
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    loop {
        let mut line = String::new();
        // BufRead::read_line alone permits unbounded allocation before checking.
        let count = std::io::Read::by_ref(&mut input)
            .take(editor_agent::MAX_REQUEST_BYTES + 1)
            .read_line(&mut line)?;
        if count == 0 {
            break;
        }
        if count as u64 > editor_agent::MAX_REQUEST_BYTES || !line.ends_with('\n') {
            return Err("Request exceeds 1 MiB or lacks final newline".into());
        }
        let request = serde_json::from_str::<AgentRequest>(&line);
        let response = match request {
            Err(error) => AgentResponse::error(
                String::new(),
                editor.revision(),
                "protocol",
                error.to_string(),
            ),
            Ok(request) => {
                if let Some(descriptor) = &descriptor {
                    editor_agent::connect(descriptor, request)?
                } else {
                    execute(&mut editor, &mut document, request)?
                }
            }
        };
        serde_json::to_writer(&mut output, &response)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
    Ok(())
}
