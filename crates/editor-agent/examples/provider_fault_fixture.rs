//! Standard-library provider fault helper, intended for an editor-agent test
//! example. It has no credentials, configuration discovery or editor access.
//!
//! Every process arms its own hard watchdog before pipe I/O or child spawning.
//! Fallback termination exits with 124, including when stdout/stderr is blocked.
//! The parent-with-child mode intentionally leaves one finite-lived descendant
//! in the inherited process group/job so the real Process adapter can own it.

use std::{
    env,
    error::Error,
    ffi::OsString,
    fmt,
    io::{self, Write},
    process::{self, Command, Stdio},
    thread,
    time::Duration,
};

const DEFAULT_TTL_MS: u64 = 5_000;
const MAX_TTL_MS: u64 = 10_000;
const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const FLOOD_FRAME: &[u8] = b"{\"event\":1}\n";

#[derive(Clone, Copy)]
enum Mode {
    NeverReadStdin,
    ParentWithChild,
    PipeHolder,
    SplitLine,
    InvalidLine,
    OversizedLine,
    StdoutFlood,
    StderrFlood,
    StderrFloodHold,
}

struct Options {
    mode: Mode,
    ttl_ms: u64,
    bytes: usize,
}

#[derive(Debug)]
enum FixtureError {
    Arguments(&'static str),
    Io(io::Error),
}

impl fmt::Display for FixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Arguments(message) => formatter.write_str(message),
            Self::Io(error) => write!(formatter, "fixture I/O failed: {error}"),
        }
    }
}

impl Error for FixtureError {}

impl From<io::Error> for FixtureError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl Options {
    fn parse(mut arguments: impl Iterator<Item = OsString>) -> Result<Self, FixtureError> {
        let mode = arguments
            .next()
            .ok_or(FixtureError::Arguments("fixture mode is required"))?;
        let mode = match mode.to_str() {
            Some("never-read-stdin") => Mode::NeverReadStdin,
            Some("parent-with-child") => Mode::ParentWithChild,
            Some("pipe-holder") => Mode::PipeHolder,
            Some("split-line") => Mode::SplitLine,
            Some("invalid-line") => Mode::InvalidLine,
            Some("oversized-line") => Mode::OversizedLine,
            Some("stdout-flood") => Mode::StdoutFlood,
            Some("stderr-flood") => Mode::StderrFlood,
            Some("stderr-flood-hold") => Mode::StderrFloodHold,
            _ => return Err(FixtureError::Arguments("unknown fixture mode")),
        };
        let mut options = Self {
            mode,
            ttl_ms: DEFAULT_TTL_MS,
            bytes: match mode {
                Mode::OversizedLine => 2_200_000,
                Mode::StderrFlood | Mode::StderrFloodHold => 280_000,
                _ => 4 * 1024 * 1024,
            },
        };
        let mut seen_ttl = false;
        let mut seen_bytes = false;
        for _ in 0..2 {
            let Some(flag) = arguments.next() else {
                break;
            };
            let value = arguments.next().ok_or(FixtureError::Arguments(
                "fixture option requires a numeric value",
            ))?;
            let number = value
                .to_str()
                .and_then(|value| value.parse::<u64>().ok())
                .ok_or(FixtureError::Arguments("invalid fixture numeric option"))?;
            match flag.to_str() {
                Some("--ttl-ms") if !seen_ttl => {
                    seen_ttl = true;
                    if !(100..=MAX_TTL_MS).contains(&number) {
                        return Err(FixtureError::Arguments(
                            "fixture TTL must be 100–10000 milliseconds",
                        ));
                    }
                    options.ttl_ms = number;
                }
                Some("--bytes") if !seen_bytes => {
                    seen_bytes = true;
                    if !(1..=MAX_OUTPUT_BYTES as u64).contains(&number)
                        || !matches!(
                            mode,
                            Mode::OversizedLine
                                | Mode::StdoutFlood
                                | Mode::StderrFlood
                                | Mode::StderrFloodHold
                        )
                    {
                        return Err(FixtureError::Arguments(
                            "fixture byte budget must be 1–16777216 for an output mode",
                        ));
                    }
                    options.bytes = number as usize;
                }
                _ => {
                    return Err(FixtureError::Arguments(
                        "unknown or repeated fixture option",
                    ));
                }
            }
        }
        if arguments.next().is_some() {
            return Err(FixtureError::Arguments("too many fixture options"));
        }
        Ok(options)
    }
}

fn arm_watchdog(ttl_ms: u64) -> io::Result<()> {
    thread::Builder::new()
        .name("neptune-video-fixture-ttl".into())
        .spawn(move || {
            thread::sleep(Duration::from_millis(ttl_ms));
            // A blocked pipe writer cannot run a normal deadline check. This
            // watchdog terminates this helper only, never its caller or group.
            process::exit(124);
        })?;
    Ok(())
}

fn park_until_watchdog() -> ! {
    loop {
        thread::park();
    }
}

fn parent_with_child(options: &Options) -> Result<(), FixtureError> {
    let mut child = Command::new(env::current_exe()?)
        .args(["pipe-holder", "--ttl-ms"])
        .arg(options.ttl_ms.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;
    let reported = writeln!(
        io::stdout().lock(),
        "{{\"event\":\"descendant\",\"pid\":{},\"parent_pid\":{},\"ttl_ms\":{}}}",
        child.id(),
        process::id(),
        options.ttl_ms
    )
    .and_then(|()| io::stdout().flush());
    if let Err(error) = reported {
        // Reporting failed: retain no hidden descendant for the test to find.
        let _ = child.kill();
        let _ = child.wait();
        return Err(error.into());
    }
    // Dropping std::process::Child does not terminate it. The descendant remains
    // inside the inherited job/group and retains both output pipes, with its
    // own finite watchdog as a fallback if the owning test fails.
    drop(child);
    Ok(())
}

fn write_bytes(writer: &mut impl Write, count: usize) -> io::Result<()> {
    let buffer = [b'x'; 8192];
    let mut remaining = count;
    while remaining != 0 {
        let length = remaining.min(buffer.len());
        writer.write_all(&buffer[..length])?;
        remaining -= length;
    }
    writer.flush()
}

fn run(options: &Options) -> Result<(), FixtureError> {
    match options.mode {
        Mode::NeverReadStdin => {
            writeln!(
                io::stdout().lock(),
                "{{\"event\":\"never_read_stdin\",\"pid\":{},\"ttl_ms\":{}}}",
                process::id(),
                options.ttl_ms
            )?;
            io::stdout().flush()?;
            park_until_watchdog();
        }
        Mode::ParentWithChild => parent_with_child(options)?,
        Mode::PipeHolder => park_until_watchdog(),
        Mode::SplitLine => {
            let mut stdout = io::stdout().lock();
            stdout.write_all(b"  \n{\"value\":")?;
            stdout.flush()?;
            thread::sleep(Duration::from_millis(20));
            stdout.write_all(b"7}\n")?;
            stdout.flush()?;
        }
        Mode::InvalidLine => {
            let mut stdout = io::stdout().lock();
            stdout.write_all(b"not JSON\n")?;
            stdout.flush()?;
        }
        Mode::OversizedLine => {
            let mut stdout = io::stdout().lock();
            write_bytes(&mut stdout, options.bytes)?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
        }
        Mode::StdoutFlood => {
            // Whole NDJSON frames; round down to stay within the byte budget.
            // Even one frame is small and no one frame exceeds its own bound.
            let mut stdout = io::stdout().lock();
            for _ in 0..options.bytes / FLOOD_FRAME.len() {
                stdout.write_all(FLOOD_FRAME)?;
            }
            stdout.flush()?;
        }
        Mode::StderrFlood => write_bytes(&mut io::stderr().lock(), options.bytes)?,
        Mode::StderrFloodHold => {
            write_bytes(&mut io::stderr().lock(), options.bytes)?;
            // Keep stdout open until the adapter classifies the stderr budget;
            // early EOF must not race its error-reader's limit observation.
            park_until_watchdog();
        }
    }
    Ok(())
}

fn main() {
    let options = Options::parse(env::args_os().skip(1));
    let ttl_ms = options
        .as_ref()
        .map_or(DEFAULT_TTL_MS, |options| options.ttl_ms);
    if arm_watchdog(ttl_ms).is_err() {
        process::exit(3);
    }
    if let Err(error) = options.and_then(|options| run(&options)) {
        eprintln!("{error}");
        process::exit(2);
    }
}
