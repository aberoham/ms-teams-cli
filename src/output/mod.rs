pub mod json;
pub mod plain;
pub mod progress;
pub mod table;

use crate::error::TeamsError;
use crate::models::common::{Envelope, Metadata, Page};
use serde::Serialize;
use std::io::{self, IsTerminal, Write};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Json,
    Human,
    Plain,
}

impl OutputFormat {
    pub fn detect(flag: Option<&str>) -> Self {
        match flag {
            Some("json") => Self::Json,
            Some("human") | Some("table") => Self::Human,
            Some("plain") | Some("text") => Self::Plain,
            _ => {
                if std::io::stdout().is_terminal() {
                    Self::Human
                } else {
                    Self::Json
                }
            }
        }
    }
}

/// Write bytes to stdout, treating a closed pipe as normal early termination.
pub fn write_stdout(bytes: &[u8]) {
    handle_stdout_result(try_write_stdout(bytes));
}

/// Write a line to stdout, treating a closed pipe as normal early termination.
pub fn write_stdout_line(line: &str) {
    handle_stdout_result(try_write_stdout_line(line));
}

fn try_write_stdout(bytes: &[u8]) -> io::Result<()> {
    io::stdout().lock().write_all(bytes)
}

pub fn try_write_stdout_line(line: &str) -> io::Result<()> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    stdout.write_all(line.as_bytes())?;
    stdout.write_all(b"\n")
}

fn handle_stdout_result(result: io::Result<()>) {
    if let Err(err) = result {
        if err.kind() == io::ErrorKind::BrokenPipe {
            std::process::exit(0);
        }

        eprintln!("Error: Failed to write to stdout: {err}");
        std::process::exit(1);
    }
}

pub fn print_success<T: Serialize>(format: OutputFormat, data: &T, start: Instant) {
    let metadata = Metadata::new().with_duration(start.elapsed().as_millis() as u64);
    match format {
        OutputFormat::Json => {
            let envelope = Envelope::success(data, metadata);
            json::print(&envelope);
        }
        OutputFormat::Human => {
            let output = serde_json::to_string_pretty(data).unwrap_or_default();
            write_stdout_line(&output);
        }
        OutputFormat::Plain => {
            plain::print_object(data);
        }
    }
}

pub fn print_success_list<T: Serialize>(format: OutputFormat, data: &[T], start: Instant) {
    let metadata = Metadata::new().with_duration(start.elapsed().as_millis() as u64);
    match format {
        OutputFormat::Json => {
            let envelope = Envelope::success(data, metadata);
            json::print(&envelope);
        }
        OutputFormat::Human => {
            let output = serde_json::to_string_pretty(data).unwrap_or_default();
            write_stdout_line(&output);
        }
        OutputFormat::Plain => {
            plain::print_list(data);
        }
    }
}

/// Print a paged list result. JSON metadata carries `has_more` and
/// `next_link`; other formats note truncation on stderr.
pub fn print_success_page<T: Serialize>(format: OutputFormat, page: &Page<T>, start: Instant) {
    match format {
        OutputFormat::Json => {
            json::print(&page_envelope(page, start.elapsed().as_millis() as u64));
        }
        OutputFormat::Human | OutputFormat::Plain => {
            print_success_list(format, &page.items, start);
            print_more_hint(page);
        }
    }
}

fn page_envelope<T: Serialize>(page: &Page<T>, duration_ms: u64) -> Envelope<&[T]> {
    let metadata = Metadata::new()
        .with_duration(duration_ms)
        .with_page(page.next_link.as_deref());
    Envelope::success(page.items.as_slice(), metadata)
}

/// Stderr footer for a truncated list; for table output.
pub fn print_more_hint<T>(page: &Page<T>) {
    if page.next_link.is_some() {
        eprintln!("{MORE_RESULTS_HINT}");
    }
}

const MORE_RESULTS_HINT: &str = "More results available; use --all-pages";

pub fn print_error(format: OutputFormat, err: &TeamsError) {
    let metadata = Metadata::new();
    match format {
        OutputFormat::Json => {
            let envelope = Envelope::<()>::error(err.error_code(), err.to_string(), metadata);
            let output = json::serialize(&envelope);
            if let Err(write_err) = try_write_stdout_line(&output) {
                if write_err.kind() != io::ErrorKind::BrokenPipe {
                    eprintln!("Error: Failed to write error response to stdout: {write_err}");
                }
            }
        }
        OutputFormat::Human | OutputFormat::Plain => {
            eprintln!("Error: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<serde_json::Value> {
        vec![
            serde_json::json!({ "id": "a" }),
            serde_json::json!({ "id": "b" }),
        ]
    }

    fn render(next_link: Option<&str>) -> serde_json::Value {
        let page = Page {
            items: items(),
            next_link: next_link.map(str::to_string),
        };
        serde_json::to_value(page_envelope(&page, 1)).unwrap()
    }

    #[test]
    fn truncated_page_reports_has_more_and_next_link() {
        let json = render(Some(
            "https://graph.microsoft.com/v1.0/me/chats?$skiptoken=x",
        ));
        assert_eq!(json["metadata"]["has_more"], true);
        assert_eq!(
            json["metadata"]["next_link"],
            "https://graph.microsoft.com/v1.0/me/chats?$skiptoken=x"
        );
    }

    #[test]
    fn complete_page_reports_no_more_and_omits_next_link() {
        let json = render(None);
        assert_eq!(json["metadata"]["has_more"], false);
        assert!(json["metadata"].get("next_link").is_none());
    }

    #[test]
    fn page_data_matches_plain_list_envelope() {
        let list = serde_json::to_value(Envelope::success(&items(), Metadata::new())).unwrap();
        assert_eq!(render(Some("https://next"))["data"], list["data"]);
        assert_eq!(render(None)["data"], list["data"]);
    }
}
