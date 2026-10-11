use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::Subcommand;

use crate::api::messages::MessageRef;
use crate::api::{self, GraphClient};
use crate::error::{Result, TeamsError};
use crate::models::attachment_inventory::{build_inventory, AttachmentItem, ItemKind};
use crate::output::{self, OutputFormat};

use super::message::resolve_message_ref;

#[derive(Debug, Subcommand)]
pub enum AttachmentsCommand {
    /// List attachments and inline images in a message
    List {
        /// Team ID (for channel messages)
        #[arg(long)]
        team: Option<String>,
        /// Channel ID (for channel messages)
        #[arg(long)]
        channel: Option<String>,
        /// Chat ID (for chat messages)
        #[arg(long)]
        chat: Option<String>,
        /// Reply ID (for a reply within a channel thread)
        #[arg(long)]
        reply: Option<String>,
        /// Message ID
        #[arg(required_unless_present = "message", conflicts_with = "message")]
        message_id: Option<String>,
        /// Message ID
        #[arg(
            long = "message",
            alias = "message-id",
            required_unless_present = "message_id",
            conflicts_with = "message_id"
        )]
        message: Option<String>,
    },
    /// Download attachments and inline images from a message
    Download {
        /// Team ID (for channel messages)
        #[arg(long)]
        team: Option<String>,
        /// Channel ID (for channel messages)
        #[arg(long)]
        channel: Option<String>,
        /// Chat ID (for chat messages)
        #[arg(long)]
        chat: Option<String>,
        /// Reply ID (for a reply within a channel thread)
        #[arg(long)]
        reply: Option<String>,
        /// Message ID
        #[arg(required_unless_present = "message", conflicts_with = "message")]
        message_id: Option<String>,
        /// Message ID
        #[arg(
            long = "message",
            alias = "message-id",
            required_unless_present = "message_id",
            conflicts_with = "message_id"
        )]
        message: Option<String>,
        /// Download only the item with this index (from `attachments list`)
        #[arg(long)]
        index: Option<usize>,
        /// Output directory for downloaded files
        #[arg(long, conflicts_with = "path")]
        dir: Option<String>,
        /// Exact output file path for a single item; use '-' for stdout
        #[arg(long, requires = "index")]
        path: Option<String>,
    },
}

pub async fn run(
    cmd: AttachmentsCommand,
    client: &GraphClient,
    format: OutputFormat,
) -> Result<()> {
    match cmd {
        AttachmentsCommand::List {
            team,
            channel,
            chat,
            reply,
            message_id,
            message,
        } => {
            let start = Instant::now();
            let message_id =
                super::message::resolve_id(message_id, message, "--message or <MESSAGE_ID>")?;
            let message_ref = resolve_message_ref(team, channel, chat, reply, message_id.clone())?;
            let items = fetch_inventory(client, &message_ref).await?;
            let result = serde_json::json!({
                "message_id": message_id,
                "items": items,
            });
            output::print_success(format, &result, start);
            Ok(())
        }

        AttachmentsCommand::Download {
            team,
            channel,
            chat,
            reply,
            message_id,
            message,
            index,
            dir,
            path,
        } => {
            let start = Instant::now();
            let message_id =
                super::message::resolve_id(message_id, message, "--message or <MESSAGE_ID>")?;
            let message_ref = resolve_message_ref(team, channel, chat, reply, message_id.clone())?;
            let items = fetch_inventory(client, &message_ref).await?;
            let selected = select_items(&items, index)?;

            if path.as_deref() == Some("-") {
                let item = selected[0];
                let (bytes, _) = fetch_item_bytes(client, &message_ref, item).await?;
                output::write_stdout(&bytes);
                return Ok(());
            }

            let dir = PathBuf::from(dir.unwrap_or_else(|| ".".to_string()));
            let mut results = Vec::new();
            let mut first_error: Option<TeamsError> = None;
            for item in &selected {
                match download_item(
                    client,
                    &message_ref,
                    &message_id,
                    item,
                    &dir,
                    path.as_deref(),
                )
                .await
                {
                    Ok(saved) => results.push(saved),
                    Err(e) => {
                        results.push(serde_json::json!({
                            "index": item.index,
                            "kind": item.kind,
                            "error": e.to_string(),
                        }));
                        first_error.get_or_insert(e);
                    }
                }
            }

            if let Some(err) = first_error {
                let saved: Vec<&str> = results
                    .iter()
                    .filter_map(|r| r.get("path").and_then(|p| p.as_str()))
                    .collect();
                let saved_note = if saved.is_empty() {
                    String::new()
                } else {
                    format!(
                        " Successfully downloaded before the failure: {}.",
                        saved.join(", ")
                    )
                };
                return Err(augment_error(err, &saved_note));
            }

            let result = serde_json::json!({
                "message_id": message_id,
                "downloaded": results,
            });
            output::print_success(format, &result, start);
            Ok(())
        }
    }
}

/// Fetch the message and its hosted contents, returning the unified inventory.
pub async fn fetch_inventory(
    client: &GraphClient,
    message_ref: &MessageRef,
) -> Result<Vec<AttachmentItem>> {
    let msg = api::messages::get_message(client, message_ref).await?;
    let hosted = api::messages::list_hosted_contents(client, message_ref).await?;
    Ok(build_inventory(&msg, &hosted))
}

fn select_items(items: &[AttachmentItem], index: Option<usize>) -> Result<Vec<&AttachmentItem>> {
    match index {
        Some(i) => {
            let item = items.iter().find(|it| it.index == i).ok_or_else(|| {
                TeamsError::InvalidInput(format!(
                    "No attachment item with index {i}; the message has {} item(s). \
                     Run `teams message attachments list` to see them.",
                    items.len()
                ))
            })?;
            if !item.downloadable {
                return Err(TeamsError::InvalidInput(format!(
                    "Item {i} ({:?}) is not downloadable; its content is inline in `message get` output.",
                    item.kind
                )));
            }
            Ok(vec![item])
        }
        None => {
            let downloadable: Vec<&AttachmentItem> =
                items.iter().filter(|it| it.downloadable).collect();
            if downloadable.is_empty() {
                return Err(TeamsError::NotFound(
                    "Message has no downloadable attachments or inline images".into(),
                ));
            }
            Ok(downloadable)
        }
    }
}

async fn fetch_item_bytes(
    client: &GraphClient,
    message_ref: &MessageRef,
    item: &AttachmentItem,
) -> Result<(Vec<u8>, Option<String>)> {
    match item.kind {
        ItemKind::HostedContent => {
            let id = item.hosted_content_id.as_deref().ok_or_else(|| {
                TeamsError::InvalidInput("Hosted content item is missing its ID".into())
            })?;
            api::messages::get_hosted_content_bytes(client, message_ref, id).await
        }
        ItemKind::CodeSnippet => {
            let url = item.code_snippet_url.as_deref().ok_or_else(|| {
                TeamsError::InvalidInput("Code snippet item is missing codeSnippetUrl".into())
            })?;
            let id = code_snippet_hosted_content_id(url).ok_or_else(|| {
                TeamsError::InvalidInput(format!(
                    "Code snippet codeSnippetUrl is not a Graph hostedContents URL, \
                     refusing to fetch it: {url}"
                ))
            })?;
            api::messages::get_hosted_content_bytes(client, message_ref, &id).await
        }
        ItemKind::FileReference => {
            let url = item.content_url.as_deref().ok_or_else(|| {
                TeamsError::InvalidInput("File attachment is missing contentUrl".into())
            })?;
            api::files::download_shared_item(client, url).await
        }
        _ => Err(TeamsError::InvalidInput(format!(
            "Item {} ({:?}) is not downloadable",
            item.index, item.kind
        ))),
    }
}

/// Extract the hosted-content ID from a `codeSnippetUrl`. The URL is message
/// content an arbitrary sender controls, so it is never fetched directly —
/// doing so would send our Graph bearer token wherever it points. Only the ID
/// is taken, percent-decoded, and the request URL is rebuilt from the message
/// reference we already resolved.
fn code_snippet_hosted_content_id(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if parsed.scheme() != "https" || parsed.host_str() != Some("graph.microsoft.com") {
        return None;
    }
    let mut segments = parsed
        .path_segments()?
        .skip_while(|s| *s != "hostedContents");
    segments.next()?;
    let id = segments.next().filter(|s| !s.is_empty())?;
    if segments.next() != Some("$value") || segments.next().is_some() {
        return None;
    }
    urlencoding::decode(id).ok().map(|s| s.into_owned())
}

async fn download_item(
    client: &GraphClient,
    message_ref: &MessageRef,
    message_id: &str,
    item: &AttachmentItem,
    dir: &Path,
    explicit_path: Option<&str>,
) -> Result<serde_json::Value> {
    let (bytes, content_type) = fetch_item_bytes(client, message_ref, item).await?;

    let target = match explicit_path {
        Some(p) => PathBuf::from(p),
        None => unique_path(dir.join(default_filename(message_id, item, content_type.as_deref()))),
    };

    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| {
                TeamsError::InvalidInput(format!(
                    "Failed to create directory '{}': {e}",
                    parent.display()
                ))
            })?;
        }
    }
    write_atomically(&target, &bytes)?;

    Ok(serde_json::json!({
        "index": item.index,
        "kind": item.kind,
        "path": target.to_string_lossy(),
        "size": bytes.len(),
        "content_type": content_type,
    }))
}

fn default_filename(message_id: &str, item: &AttachmentItem, content_type: Option<&str>) -> String {
    match item.kind {
        ItemKind::FileReference => item
            .name
            .as_deref()
            .map(sanitize_filename)
            .unwrap_or_else(|| format!("msg-{message_id}-{}.bin", item.index)),
        ItemKind::CodeSnippet => format!("msg-{message_id}-{}.txt", item.index),
        _ => format!(
            "msg-{message_id}-{}.{}",
            item.index,
            extension_for(content_type)
        ),
    }
}

fn extension_for(content_type: Option<&str>) -> &'static str {
    match content_type {
        Some("image/png") => "png",
        Some("image/jpeg") => "jpg",
        Some("image/gif") => "gif",
        Some("image/webp") => "webp",
        Some("image/bmp") => "bmp",
        Some("image/svg+xml") => "svg",
        Some("text/plain") => "txt",
        Some(ct) => mime_guess::get_mime_extensions_str(ct)
            .and_then(|exts| exts.first())
            .copied()
            .unwrap_or("bin"),
        None => "bin",
    }
}

/// Strip path separators and traversal from an attachment-supplied filename so
/// it cannot escape the output directory.
fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '\0' => '_',
            other => other,
        })
        .collect();
    let cleaned = cleaned.trim_start_matches(['.', ' ']).trim_end();
    if cleaned.is_empty() {
        "attachment.bin".to_string()
    } else {
        cleaned.to_string()
    }
}

/// Avoid clobbering existing files by suffixing `-1`, `-2`, … before the extension.
fn unique_path(candidate: PathBuf) -> PathBuf {
    if !candidate.exists() {
        return candidate;
    }
    let stem = candidate
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "attachment".to_string());
    let ext = candidate
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let parent = candidate.parent().unwrap_or(Path::new("."));
    (1..)
        .map(|n| parent.join(format!("{stem}-{n}{ext}")))
        .find(|p| !p.exists())
        .expect("unbounded suffix search always terminates")
}

/// Write to a uniquely named, exclusively created temp file beside `target`,
/// then rename over it. A failed download leaves nothing behind, and an
/// existing file or symlink at a temp path is never followed or clobbered.
pub(crate) fn write_atomically(target: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;

    let fail = |e: std::io::Error| {
        TeamsError::InvalidInput(format!("Failed to write '{}': {e}", target.display()))
    };
    if target.file_name().is_none() {
        return Err(TeamsError::InvalidInput(
            "Target path has no filename".into(),
        ));
    }
    let dir = match target.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let mut builder = tempfile::Builder::new();
    builder.prefix(".teams-download-").suffix(".part");
    // Keep an existing target's mode exactly; otherwise 0666 less umask, as a
    // plain create would, not tempfile's 0600.
    #[cfg(unix)]
    let existing = std::fs::metadata(target).ok().map(|m| m.permissions());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = existing.as_ref().map_or(0o666, |p| p.mode() & 0o777);
        builder.permissions(std::fs::Permissions::from_mode(mode));
    }
    let mut tmp = builder.tempfile_in(dir).map_err(fail)?;
    #[cfg(unix)]
    if let Some(perms) = existing {
        tmp.as_file().set_permissions(perms).map_err(fail)?;
    }
    tmp.write_all(bytes).map_err(fail)?;
    tmp.persist(target).map_err(|e| fail(e.error))?;
    Ok(())
}

fn augment_error(err: TeamsError, note: &str) -> TeamsError {
    if note.is_empty() {
        return err;
    }
    match err {
        TeamsError::PermissionDenied(m) => TeamsError::PermissionDenied(format!("{m}{note}")),
        TeamsError::NotFound(m) => TeamsError::NotFound(format!("{m}{note}")),
        TeamsError::InvalidInput(m) => TeamsError::InvalidInput(format!("{m}{note}")),
        TeamsError::ApiError { status, message } => TeamsError::ApiError {
            status,
            message: format!("{message}{note}"),
        },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_traversal_and_separators() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize_filename("a/b\\c:d"), "a_b_c_d");
        assert_eq!(sanitize_filename("  .hidden"), "hidden");
        assert_eq!(sanitize_filename(""), "attachment.bin");
        assert_eq!(sanitize_filename("NetskopeLogs.zip"), "NetskopeLogs.zip");
    }

    #[test]
    fn extension_covers_common_image_types() {
        assert_eq!(extension_for(Some("image/png")), "png");
        assert_eq!(extension_for(Some("image/jpeg")), "jpg");
        assert_eq!(extension_for(None), "bin");
        assert_eq!(extension_for(Some("application/x-unknown-thing")), "bin");
    }

    #[test]
    fn unique_path_suffixes_on_collision() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("shot.png");
        std::fs::write(&first, b"x").unwrap();
        let next = unique_path(first.clone());
        assert_eq!(next, dir.path().join("shot-1.png"));
        std::fs::write(&next, b"x").unwrap();
        assert_eq!(unique_path(first), dir.path().join("shot-2.png"));
    }

    #[test]
    fn write_atomically_leaves_no_part_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.png");
        write_atomically(&target, b"bytes").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"bytes");
        assert!(!dir.path().join("out.png.part").exists());

        let no_extension = dir.path().join("out");
        write_atomically(&no_extension, b"more").unwrap();
        assert_eq!(std::fs::read(&no_extension).unwrap(), b"more");
        assert!(!dir.path().join("out.part").exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn write_atomically_ignores_symlink_at_old_part_path() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim");
        std::fs::write(&victim, b"keep").unwrap();
        let part = dir.path().join("out.png.part");
        std::os::unix::fs::symlink(&victim, &part).unwrap();
        let target = dir.path().join("out.png");

        write_atomically(&target, b"payload").unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"payload");
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
        assert!(std::fs::symlink_metadata(&part)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn write_atomically_keeps_default_file_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain");
        std::fs::write(&plain, b"x").unwrap();
        let target = dir.path().join("out.png");

        write_atomically(&target, b"payload").unwrap();

        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&target), mode(&plain));

        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        write_atomically(&target, b"again").unwrap();
        assert_eq!(mode(&target), 0o600);

        // Bits the umask would strip on create survive a replace.
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o664)).unwrap();
        write_atomically(&target, b"third").unwrap();
        assert_eq!(mode(&target), 0o664);
    }

    #[cfg(unix)]
    #[test]
    fn write_atomically_accepts_long_filename() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(format!("{}.bin", "a".repeat(246)));
        write_atomically(&target, b"payload").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"payload");
    }

    #[test]
    fn write_atomically_leaves_stray_part_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("out.png.part");
        std::fs::write(&part, b"stray").unwrap();
        let target = dir.path().join("out.png");

        write_atomically(&target, b"payload").unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"payload");
        assert_eq!(std::fs::read(&part).unwrap(), b"stray");
    }

    #[test]
    fn write_atomically_failure_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        // A directory at the target makes the final rename fail after the write.
        let target = dir.path().join("out.png");
        std::fs::create_dir(&target).unwrap();

        assert!(write_atomically(&target, b"payload").is_err());

        assert!(target.is_dir());
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("out.png")]);
    }

    #[test]
    fn write_atomically_concurrent_writers_leave_one_whole_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.bin");
        let payloads = [vec![b'a'; 4 << 20], vec![b'b'; 4 << 20]];
        let barrier = std::sync::Barrier::new(payloads.len());

        std::thread::scope(|s| {
            let handles: Vec<_> = payloads
                .iter()
                .map(|p| {
                    let (target, barrier) = (&target, &barrier);
                    s.spawn(move || {
                        barrier.wait();
                        write_atomically(target, p)
                    })
                })
                .collect();
            let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
            // Windows can refuse a rename onto a file another rename is replacing
            // ("Access is denied"); one writer may lose there. Elsewhere both win.
            let failed = results.iter().filter(|r| r.is_err()).count();
            assert!(failed == 0 || (cfg!(windows) && failed == 1), "{results:?}");
        });

        // Either way the target is one whole payload, never a mix, and the
        // losing writer's temp file is gone.
        let written = std::fs::read(&target).unwrap();
        assert!(payloads.contains(&written));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn code_snippet_id_extracted_from_graph_url_only() {
        assert_eq!(
            code_snippet_hosted_content_id(
                "https://graph.microsoft.com/v1.0/chats/19:x/messages/m1/hostedContents/aWQ9/$value"
            )
            .as_deref(),
            Some("aWQ9")
        );
        // Percent-encoded IDs are decoded so the rebuilt URL doesn't double-encode.
        assert_eq!(
            code_snippet_hosted_content_id(
                "https://graph.microsoft.com/v1.0/chats/c/messages/m/hostedContents/aWQ%3D/$value"
            )
            .as_deref(),
            Some("aWQ=")
        );
        // A URL pointing anywhere else must be rejected, not fetched.
        assert_eq!(
            code_snippet_hosted_content_id("https://evil.example.com/steal-token"),
            None
        );
        assert_eq!(
            code_snippet_hosted_content_id("https://evil.example.com/hostedContents/x/$value"),
            None
        );
        assert_eq!(
            code_snippet_hosted_content_id(
                "https://evil.example.com/hostedContents/x/$value/extra"
            ),
            None
        );
        assert_eq!(
            code_snippet_hosted_content_id("https://evil.example.com/hostedContents//$value"),
            None
        );
    }
}
