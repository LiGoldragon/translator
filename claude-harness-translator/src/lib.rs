//! The Claude Harness Nexus's translator: the one place that holds Claude
//! Code's words. It reads a hook's payload into a Hook, and follows a
//! transcript for its Nexus, reading each new row.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Seek, SeekFrom};
use std::io::Write;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::process::ExitCode;

use claude_harness_process::*;
use harness::{BackgroundWork, Description, EndReason, FilePath, Response, Session, Tokens, WorkId, WorkKind};
use rustix::event::{PollFd, PollFlags};
use rustix::fs::inotify;
use serde_json::Value;
use transport::{Framed, Place, Placed};

#[derive(Debug, PartialEq, Eq)]
pub enum Untranslated {
    Unreadable,
    UnknownEvent(String),
    MissingField(&'static str),
}

/// What reads the harness's own words into a type of the Nexus.
pub trait Translatable: Sized {
    fn translate(text: &str) -> Result<Self, Untranslated>;
}

fn text(value: &Value, field: &'static str) -> Result<String, Untranslated> {
    value.get(field).and_then(Value::as_str).map(str::to_owned).ok_or(Untranslated::MissingField(field))
}

/// A task of a kind not known is left out, and the rest of its Stop is read.
fn work(task: &Value) -> Result<BackgroundWork, Untranslated> {
    let work_kind = match text(task, "type")?.as_str() {
        "shell" => WorkKind::Shell,
        "subagent" => WorkKind::Subagent,
        "monitor" => WorkKind::Monitor,
        "workflow" => WorkKind::Workflow,
        other => return Err(Untranslated::UnknownEvent(format!("background task type {other}"))),
    };
    Ok(BackgroundWork { work_id: WorkId(text(task, "id")?), work_kind, description: Description(text(task, "description").unwrap_or_default()) })
}

impl Translatable for Hook {
    fn translate(payload: &str) -> Result<Hook, Untranslated> {
        let value: Value = serde_json::from_str(payload).map_err(|_| Untranslated::Unreadable)?;
        let session = || text(&value, "session_id").map(Session);
        Ok(match text(&value, "hook_event_name")?.as_str() {
            // A start on resume or on compact is not yet read: its mapping waits on a ruling.
            "SessionStart" => match text(&value, "source").ok().as_deref() {
                Some(source @ ("resume" | "compact")) => return Err(Untranslated::UnknownEvent(format!("SessionStart from {source}"))),
                _ => Hook::SessionStart(SessionStart { session: session()?, file_path: FilePath(text(&value, "transcript_path")?) }),
            },
            "UserPromptSubmit" => Hook::UserPromptSubmit(session()?),
            "PermissionRequest" => Hook::PermissionRequest(session()?),
            "PostToolUse" => Hook::PostToolUse(session()?),
            "Stop" => Hook::Stop(Stop {
                session: session()?,
                response_option: text(&value, "last_assistant_message").ok().map(Response),
                background_work_vector: value
                    .get("background_tasks")
                    .and_then(Value::as_array)
                    .map(|tasks| {
                        tasks
                            .iter()
                            .filter_map(|task| work(task).inspect_err(|untranslated| eprintln!("{untranslated:?}")).ok())
                            .collect()
                    })
                    .unwrap_or_default(),
            }),
            "SessionEnd" => Hook::SessionEnd(SessionEnd {
                session: session()?,
                end_reason: match text(&value, "reason")?.as_str() {
                    "clear" => EndReason::Cleared,
                    _ => EndReason::Exited,
                },
            }),
            other => return Err(Untranslated::UnknownEvent(other.to_owned())),
        })
    }
}

impl Translatable for Row {
    /// Of the main thread: an assistant row tells the context's size, the user
    /// row that carries turnPosition starts a turn, and the stop_hook_summary
    /// row that did not keep the turn going ends it. Every other row is Other.
    fn translate(line: &str) -> Result<Row, Untranslated> {
        let value: Value = serde_json::from_str(line).map_err(|_| Untranslated::Unreadable)?;
        let main_thread = value.get("isSidechain").and_then(Value::as_bool) != Some(true);
        let usage = value.get("message").and_then(|message| message.get("usage"));
        let summary = value.get("subtype").and_then(Value::as_str) == Some("stop_hook_summary")
            && value.get("preventedContinuation").and_then(Value::as_bool) == Some(false);
        Ok(match (value.get("type").and_then(Value::as_str), main_thread, usage) {
            (Some("user"), true, _) if value.get("turnPosition").is_some_and(|position| !position.is_null()) => Row::TurnStarted,
            (Some("system"), true, _) if summary => Row::TurnEnded,
            (Some("assistant"), true, Some(usage)) => {
                let tokens = ["input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"]
                    .iter()
                    .filter_map(|field| usage.get(field).and_then(Value::as_i64))
                    .sum();
                Row::Assistant(Tokens(tokens))
            }
            _ => Row::Other,
        })
    }
}

/// Reads the whole lines after the offset, at most the row count of them.
fn rows_after(path: &str, offset: &Offset, row_count: &RowCount) -> io::Result<Following> {
    let mut file = BufReader::new(File::open(path)?);
    file.seek(SeekFrom::Start(offset.0.max(0) as u64))?;
    let mut rows = Following { row_vector: Vec::new(), offset: offset.clone() };
    let mut line = Vec::new();
    while (rows.row_vector.len() as i64) < row_count.0 {
        line.clear();
        let read = file.read_until(b'\n', &mut line)?;
        if read == 0 || line.last() != Some(&b'\n') {
            break;
        }
        rows.offset.0 += read as i64;
        rows.row_vector.push(Row::translate(&String::from_utf8_lossy(&line)).unwrap_or(Row::Other));
    }
    Ok(rows)
}

/// A lasting emission: an outcome each time the transcript grows, until the
/// Nexus closes the connection. A transcript not yet written is waited for:
/// its nearest existing ancestor is watched until each missing part appears.
/// A path with no ancestor at all ends the connection, and the emission is Unanswered.
pub fn follow(mut stream: UnixStream, follow: Follow) -> io::Result<()> {
    let path = std::path::PathBuf::from(&follow.file_path.0);
    let watching = inotify::init(inotify::CreateFlags::CLOEXEC)?;
    let mut appearing: Option<(std::path::PathBuf, i32)> = None;
    let mut followed = false;
    let mut offset = follow.offset.clone();
    let mut events = [0u8; 4096];
    loop {
        while !followed {
            if inotify::add_watch(&watching, &path, inotify::WatchFlags::MODIFY).is_ok() {
                followed = true;
                if let Some((_, watch)) = appearing.take() {
                    inotify::remove_watch(&watching, watch)?;
                }
                break;
            }
            let Some(nearest) = path.ancestors().skip(1).find(|ancestor| ancestor.is_dir()).map(std::path::Path::to_owned) else {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no ancestor of the transcript exists"));
            };
            if appearing.as_ref().is_some_and(|(directory, _)| *directory == nearest) {
                break;
            }
            if let Some((_, watch)) = appearing.take() {
                inotify::remove_watch(&watching, watch)?;
            }
            let watch = inotify::add_watch(&watching, &nearest, inotify::WatchFlags::CREATE | inotify::WatchFlags::MOVED_TO)?;
            appearing = Some((nearest, watch));
        }
        if followed {
            let rows = rows_after(&follow.file_path.0, &offset, &follow.row_count)?;
            let full = rows.row_vector.len() as i64 >= follow.row_count.0;
            if !rows.row_vector.is_empty() {
                offset = rows.offset.clone();
                rows.write_to(&mut stream)?;
            }
            if full {
                continue;
            }
        }
        let mut waiting = [PollFd::new(&watching, PollFlags::IN), PollFd::new(&stream, PollFlags::IN)];
        rustix::event::poll(&mut waiting, None)?;
        if !waiting[1].revents().is_empty() {
            return Ok(());
        }
        rustix::io::read(&watching, &mut events)?;
    }
}

/// The translator itself as a caller: its process, and the user that owns it.
pub fn itself() -> io::Result<Caller> {
    use std::os::unix::fs::MetadataExt;
    let user_id = std::fs::metadata("/proc/self")?.uid();
    Ok(Caller { process_id: ProcessId(std::process::id().into()), user_id: UserId(user_id.into()) })
}

/// The hook's caller, from the connection's peer credentials.
pub fn caller(stream: &UnixStream) -> io::Result<Caller> {
    let credentials = rustix::net::sockopt::socket_peercred(stream)?;
    Ok(Caller { process_id: ProcessId(credentials.pid.as_raw_nonzero().get().into()), user_id: UserId(credentials.uid.as_raw().into()) })
}

/// The CLI, written once for the ordinary and the meta socket: one datom
/// written to the translator; each reply printed as it arrives, one datom a
/// line, until the translator ends the exchange.
pub fn call(text_socket: Placed) -> ExitCode {
    let mut arguments = std::env::args().skip(1);
    let (Some(datom), None) = (arguments.next(), arguments.next()) else {
        eprintln!("Usage");
        return ExitCode::from(2);
    };
    let place = match Place::from_configuration() {
        Ok(place) => place,
        Err(unplaced) => {
            eprintln!("{unplaced:?}");
            return ExitCode::from(2);
        }
    };
    let Ok(mut stream) = UnixStream::connect(place.path(text_socket)) else {
        eprintln!("TranslatorDown");
        return ExitCode::FAILURE;
    };
    if stream.write_all(datom.as_bytes()).is_err() || stream.shutdown(Shutdown::Write).is_err() {
        return ExitCode::FAILURE;
    }
    let mut out = std::io::stdout();
    for line in BufReader::new(stream).lines().map_while(Result::ok) {
        if writeln!(out, "{line}").and_then(|()| out.flush()).is_err() {
            break;
        }
    }
    ExitCode::SUCCESS
}
