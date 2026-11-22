//! `ops-relay` — tails a line-oriented log and relays new lines as Telegram
//! alerts through [`alertkit::TelegramSink`].
//!
//! Each log line becomes one [`alertkit::Alert`]. A persisted offset file
//! records how many lines were already relayed; the process relays every
//! line beyond that offset (empty lines are skipped, but still advance the
//! offset), then writes the new line count back.
//!
//! The bot token is read from a file at runtime — never pass tokens on the
//! command line. A missing or empty token file means the relay is not
//! armed: the process exits 0 quietly without sending anything. Exit is
//! non-zero only for bad arguments or hard I/O failures.

use std::path::PathBuf;
use std::process::ExitCode;

use alertkit::{Alert, TelegramSink};

const USAGE: &str =
    "usage: ops-relay --token-file <path> --chat-id <id> --log <path> --offset <path>";

/// Parsed command-line arguments.
struct Args {
    /// File holding the Telegram bot token (read at runtime, trimmed).
    token_file: PathBuf,
    /// Telegram chat id to deliver alerts to.
    chat_id: String,
    /// Line-oriented log to tail.
    log: PathBuf,
    /// File persisting the number of already-relayed lines.
    offset: PathBuf,
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{USAGE}\nops-relay: {message}");
            return ExitCode::from(2);
        }
    };

    let token = match std::fs::read_to_string(&args.token_file) {
        Ok(token) => token.trim().to_string(),
        // Feature not armed — quiet success.
        Err(_) => return ExitCode::SUCCESS,
    };
    if token.is_empty() {
        return ExitCode::SUCCESS;
    }

    // A missing or unreadable log is treated as empty: nothing to relay.
    let log_content = std::fs::read_to_string(&args.log).unwrap_or_default();
    let prior_offset = std::fs::read_to_string(&args.offset)
        .ok()
        .and_then(|content| parse_offset(&content))
        .unwrap_or(0);

    let pending = pending_relay_lines(&log_content, prior_offset);
    let total = total_lines(&log_content);

    if !pending.is_empty() {
        let sink = TelegramSink::new(token, args.chat_id.clone());
        for line in &pending {
            sink.block_on_send(&alert_for(line));
        }
    }

    // Persist the offset so the next run relays only fresh lines.
    if let Some(parent) = args.offset.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                eprintln!("ops-relay: offset dir {}: {error}", parent.display());
                return ExitCode::FAILURE;
            }
        }
    }
    match std::fs::write(&args.offset, format!("{total}\n")) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ops-relay: offset write {}: {error}", args.offset.display());
            ExitCode::FAILURE
        }
    }
}

/// Parses `--flag value` pairs; every flag is required exactly once.
fn parse_args<I>(mut args: I) -> Result<Args, String>
where
    I: Iterator<Item = String>,
{
    let (mut token_file, mut chat_id, mut log, mut offset) = (None, None, None, None);
    while let Some(arg) = args.next() {
        let slot: &mut Option<String> = match arg.as_str() {
            "--token-file" => &mut token_file,
            "--chat-id" => &mut chat_id,
            "--log" => &mut log,
            "--offset" => &mut offset,
            other => return Err(format!("unrecognized argument `{other}`")),
        };
        if slot.is_some() {
            return Err(format!("`{arg}` given more than once"));
        }
        *slot = Some(
            args.next()
                .ok_or_else(|| format!("`{arg}` requires a value"))?,
        );
    }
    let chat_id = chat_id
        .filter(|id| !id.is_empty())
        .ok_or("missing --chat-id")?;
    Ok(Args {
        token_file: token_file.ok_or("missing --token-file")?.into(),
        chat_id,
        log: log.ok_or("missing --log")?.into(),
        offset: offset.ok_or("missing --offset")?.into(),
    })
}

/// Parses the persisted offset; unreadable or malformed means 0.
fn parse_offset(content: &str) -> Option<u64> {
    content.trim().parse().ok()
}

/// Lines at or beyond `offset`, with blank lines skipped.
///
/// Blank lines still advance the offset (it counts file lines, not alerts),
/// so arithmetic stays stable across rotations and re-runs.
fn pending_relay_lines(content: &str, offset: u64) -> Vec<&str> {
    content
        .lines()
        .enumerate()
        .filter(|(index, _line)| u64::try_from(*index).is_ok_and(|index| index >= offset))
        .map(|(_, line)| line)
        .filter(|line| !line.trim().is_empty())
        .collect()
}

/// Total line count, as written back to the offset file.
fn total_lines(content: &str) -> u64 {
    u64::try_from(content.lines().count()).unwrap_or(u64::MAX)
}

/// Alert envelope for one relayed log line.
fn alert_for(line: &str) -> Alert {
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(0));
    Alert {
        rule_id: "ops-relay".to_string(),
        rule_name: "ops relay".to_string(),
        message: line.to_string(),
        entity: "ops-relay".to_string(),
        ts_ms,
    }
}

/// Synchronous send helper: runs one fire-and-forget delivery on a throwaway
/// single-threaded runtime, keeping `main` free of `Future` plumbing.
trait BlockOnSend {
    /// Delivers `alert`, blocking the caller until the send settles.
    fn block_on_send(&self, alert: &Alert);
}

impl BlockOnSend for TelegramSink {
    fn block_on_send(&self, alert: &Alert) {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime.block_on(self.send(alert)),
            Err(error) => eprintln!("ops-relay: runtime: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Unique per-process temp path so parallel test runs never collide.
    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ops-relay-{}-{tag}", std::process::id()))
    }

    #[test]
    fn offset_arithmetic_on_temp_log() -> Result<(), Box<dyn std::error::Error>> {
        let path = temp_path("offset");
        fs::write(&path, "alpha\nbeta\ngamma\n")?;
        let content = fs::read_to_string(&path)?;

        let pending = pending_relay_lines(&content, 2);
        assert_eq!(pending, ["gamma"]);
        assert_eq!(total_lines(&content), 3);

        // Offset past the end (log rotated): nothing relayed, offset resets.
        assert!(pending_relay_lines(&content, 99).is_empty());

        let _ = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn empty_lines_are_skipped_but_counted() -> Result<(), Box<dyn std::error::Error>> {
        let path = temp_path("blanks");
        fs::write(&path, "one\n\n   \ntwo\n")?;
        let content = fs::read_to_string(&path)?;

        let pending = pending_relay_lines(&content, 0);
        assert_eq!(pending, ["one", "two"]);
        // Blank lines count toward the persisted offset.
        assert_eq!(total_lines(&content), 4);

        // Mid-file offset still skips leading blanks correctly.
        assert_eq!(pending_relay_lines(&content, 3), ["two"]);

        let _ = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn offset_parse_falls_back_to_zero() {
        assert_eq!(parse_offset(""), None);
        assert_eq!(parse_offset("not-a-number"), None);
        assert_eq!(parse_offset("7\n"), Some(7));
    }

    #[test]
    fn missing_args_are_rejected() {
        let full = [
            "--token-file",
            "/tmp/t",
            "--chat-id",
            "42",
            "--log",
            "/tmp/l",
            "--offset",
            "/tmp/o",
        ];
        assert!(parse_args(full.into_iter().map(String::from)).is_ok());
        assert!(parse_args(full.into_iter().take(6).map(String::from)).is_err());
        assert!(parse_args(["--token-file"].into_iter().map(String::from)).is_err());
        assert!(parse_args(["--bogus", "x"].into_iter().map(String::from)).is_err());
        assert!(parse_args(["--chat-id", ""].into_iter().map(String::from)).is_err());
    }
}
