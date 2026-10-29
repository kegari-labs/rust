//! Bounded operator diagnostics from the existing redacted rotating sink.
//! These records describe activity; they are never workflow or recovery authority.
use super::*;
use serde_json::Value;
use std::io::{Seek, SeekFrom};

const TAIL_BYTES: u64 = 128 * 1024;
const RECENT_EVENTS: usize = 32;
const RESPONSE_BYTES: usize = 64 * 1024;

impl LogSink {
    /// The authenticated status route supplies its actual runtime identity.
    /// Read failure is data so an unavailable log never hides runtime status.
    pub fn recent(&self, runtime: Uuid) -> Value {
        match self.read_recent(runtime) {
            Ok(value) => value,
            Err(error) => json!({"available":false,"events":[],"truncated":false,
                "error":error.code,"next_action":"Inspect the runtime's protected rotating logs; status and scheduling do not depend on diagnostic reads."}),
        }
    }

    fn read_recent(&self, runtime: Uuid) -> Result<Value> {
        if runtime.is_nil() {
            return Err(Error::invalid("log read requires the registered runtime"));
        }
        let secrets = self
            .redactions
            .lock()
            .map_err(|_| Error::new("log_unavailable", "redaction state unavailable"))?;
        // Match writer lock ordering. A busy writer produces an explicit
        // unavailable tail rather than delaying an operator status request.
        let _lock = self.directory.section_lock("log.lock")?;
        let directory = PrivateDir::open(self.directory.path())?;
        let mut events = Vec::new();
        let mut response_bytes = 0usize;
        let mut truncated = false;
        'files: for index in 0..LOG_FILES {
            let leaf = if index == 0 {
                "events.jsonl".into()
            } else {
                format!("events.{index}.jsonl")
            };
            let mut file = match OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
                .open(directory.path().join(&leaf))
            {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(Error::io("open retained diagnostic log", error)),
            };
            validate_file(&file, true)?;
            // The oldest retained rotation marks the storage retention boundary.
            truncated |= index == LOG_FILES - 1;
            let length = file
                .metadata()
                .map_err(|error| Error::io("inspect diagnostic log", error))?
                .len();
            if length > LOG_BYTES {
                return Err(Error::new(
                    "log_invalid",
                    "retained log exceeds the rotating-file bound",
                ));
            }
            let start = length.saturating_sub(TAIL_BYTES);
            file.seek(SeekFrom::Start(start))
                .map_err(|error| Error::io("seek diagnostic tail", error))?;
            let mut bytes = Vec::new();
            file.take(TAIL_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| Error::io("read diagnostic tail", error))?;
            if bytes.len() as u64 > TAIL_BYTES || !bytes.is_empty() && bytes.last() != Some(&b'\n')
            {
                return Err(Error::new(
                    "log_incomplete",
                    "diagnostic tail is not a complete bounded record sequence",
                ));
            }
            let mut body = bytes.as_slice();
            if start > 0 {
                let cut = body.iter().position(|byte| *byte == b'\n').ok_or_else(|| {
                    Error::new("log_incomplete", "diagnostic record exceeds the tail bound")
                })?;
                body = &body[cut + 1..];
                truncated = true;
            }
            for line in body
                .split(|byte| *byte == b'\n')
                .rev()
                .filter(|line| !line.is_empty())
            {
                if events.len() == RECENT_EVENTS {
                    truncated = true;
                    break 'files;
                }
                let mut value: Value = serde_json::from_slice(line).map_err(|_| {
                    Error::new(
                        "log_invalid",
                        "retained diagnostic is not a structured record",
                    )
                })?;
                let object = value.as_object_mut().ok_or_else(|| {
                    Error::new("log_invalid", "retained diagnostic is not an object")
                })?;
                let at: DateTime<Utc> =
                    serde_json::from_value(object.remove("at").ok_or_else(|| {
                        Error::new("log_invalid", "retained diagnostic lacks its timestamp")
                    })?)
                    .map_err(|_| {
                        Error::new(
                            "log_invalid",
                            "retained diagnostic has an invalid timestamp",
                        )
                    })?;
                let event: LogEvent = serde_json::from_value(value).map_err(|_| {
                    Error::new(
                        "log_invalid",
                        "retained diagnostic contains unsupported fields",
                    )
                })?;
                if event.runtime_id != runtime {
                    return Err(Error::new(
                        "log_identity",
                        "diagnostic belongs to another runtime",
                    ));
                }
                let mut value = serde_json::to_value(event)
                    .map_err(|_| Error::invalid("diagnostic encoding failed"))?;
                // Reapply current secret redaction to older records, retaining
                // the exact allowlisted field set rather than arbitrary JSON.
                for (field, maximum) in [
                    ("work", 256),
                    ("revision", 256),
                    ("event", 128),
                    ("outcome", 128),
                    ("reason", 1024),
                ] {
                    if let Some(text) = value[field].as_str() {
                        value[field] = json!(redact(text, &secrets, maximum));
                    }
                }
                value["at"] = json!(at);
                let size = serde_json::to_vec(&value)
                    .map_err(|_| Error::invalid("diagnostic encoding failed"))?
                    .len();
                if response_bytes + size > RESPONSE_BYTES - 1024 {
                    truncated = true;
                    break 'files;
                }
                response_bytes += size;
                events.push(value);
            }
            // If a file tail itself was cut, older rotations cannot fill that
            // missing interval. Return an honest contiguous recent window.
            if start > 0 {
                break;
            }
        }
        events.reverse();
        Ok(
            json!({"available":true,"events":events,"truncated":truncated,
            "maximum_events":RECENT_EVENTS,"retention":"rotating_diagnostics_only"}),
        )
    }
}
