use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::Subcommand;

use crate::api::messages::MessageRef;
use crate::api::{self, endpoints, GraphClient};
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
    /// Delete files you attached from your OneDrive (the message itself is kept)
    #[command(
        override_usage = "teams message attachments delete (--team <TEAM> --channel <CHANNEL> [--reply <REPLY>] | --chat <CHAT>) (MESSAGE_ID | --message <MESSAGE>) [--index <INDEX>] --yes"
    )]
    Delete {
        /// Team ID (for channel messages; requires --channel)
        #[arg(long, requires = "channel", conflicts_with = "chat")]
        team: Option<String>,
        /// Channel ID (for channel messages; requires --team)
        #[arg(long, requires = "team", conflicts_with = "chat")]
        channel: Option<String>,
        /// Chat ID (for chat messages)
        #[arg(long, required_unless_present = "team")]
        chat: Option<String>,
        /// Reply ID (for a reply within a channel thread)
        #[arg(long, requires = "team", conflicts_with = "chat")]
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
        /// Delete only the item with this index (from `attachments list`)
        #[arg(long)]
        index: Option<usize>,
        /// Confirm the deletion; without it the command exits with code 2 and sends nothing
        #[arg(long)]
        yes: bool,
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

        AttachmentsCommand::Delete {
            team,
            channel,
            chat,
            reply,
            message_id,
            message,
            index,
            yes: _,
        } => {
            let start = Instant::now();
            let message_id =
                super::message::resolve_id(message_id, message, "--message or <MESSAGE_ID>")?;
            let message_ref = resolve_message_ref(team, channel, chat, reply, message_id.clone())?;
            let items = fetch_inventory(client, &message_ref).await?;
            let selected = select_for_delete(&items, index)?;
            let results = delete_items_at(client, endpoints::GRAPH_V1, &selected).await?;
            let result = serde_json::json!({
                "message_id": message_id,
                "items": results,
            });
            output::print_success(format, &result, start);
            Ok(())
        }
    }
}

/// Like `select_items`, but every kind is selectable: items that are not
/// uploads are reported as skipped rather than refused.
fn select_for_delete(
    items: &[AttachmentItem],
    index: Option<usize>,
) -> Result<Vec<&AttachmentItem>> {
    match index {
        Some(i) => {
            let item = items.iter().find(|it| it.index == i).ok_or_else(|| {
                TeamsError::InvalidInput(format!(
                    "No attachment item with index {i}; the message has {} item(s). \
                     Run `teams message attachments list` to see them.",
                    items.len()
                ))
            })?;
            Ok(vec![item])
        }
        None if items.is_empty() => Err(TeamsError::NotFound(
            "Message has no attachments or inline images".into(),
        )),
        None => Ok(items.iter().collect()),
    }
}

/// Delete each selected file attachment that lives in the caller's own
/// OneDrive. Everything else is skipped with a reason. Stops at the first
/// Graph failure, noting what was already deleted.
async fn delete_items_at(
    client: &GraphClient,
    base: &str,
    items: &[&AttachmentItem],
) -> Result<Vec<serde_json::Value>> {
    let mut my_drive: Option<String> = None;
    let mut results = Vec::new();
    for item in items {
        match delete_item_at(client, base, item, &mut my_drive).await {
            Ok(r) => results.push(r),
            Err(e) => {
                let deleted: Vec<String> = results
                    .iter()
                    .filter(|r| r["deleted"] == true)
                    .map(|r| r["index"].to_string())
                    .collect();
                if deleted.is_empty() {
                    return Err(e);
                }
                let note = format!(
                    " Deleted before the failure: item(s) {}.",
                    deleted.join(", ")
                );
                // Rate-limit and network errors carry no message to extend.
                tracing::warn!("{}", note.trim());
                return Err(match e {
                    TeamsError::ServerError { status, message } => TeamsError::ServerError {
                        status,
                        message: format!("{message}{note}"),
                    },
                    TeamsError::AuthError(m) => TeamsError::AuthError(format!("{m}{note}")),
                    other => augment_error(other, &note),
                });
            }
        }
    }
    Ok(results)
}

async fn delete_item_at(
    client: &GraphClient,
    base: &str,
    item: &AttachmentItem,
    my_drive: &mut Option<String>,
) -> Result<serde_json::Value> {
    let outcome = |deleted: bool, reason: Option<&str>| {
        let mut v = serde_json::json!({
            "index": item.index,
            "name": item.name,
            "deleted": deleted,
        });
        if let Some(reason) = reason {
            v["reason"] = reason.into();
        }
        v
    };

    if item.kind != ItemKind::FileReference {
        let reason = match item.kind {
            ItemKind::HostedContent => "inline image, stored in the message, not an upload",
            ItemKind::CodeSnippet => "code snippet, stored in the message, not an upload",
            _ => "not a file attachment",
        };
        return Ok(outcome(false, Some(reason)));
    }
    let Some(url) = item.content_url.as_deref() else {
        return Ok(outcome(false, Some("file attachment has no contentUrl")));
    };

    let drive_item = match api::files::shared_drive_item_at(client, base, url).await {
        Ok(d) => d,
        // Graph masks items the caller cannot see as 404, so a missing item
        // cannot be told apart from someone else's file: never claim it deleted.
        Err(TeamsError::NotFound(_)) | Err(TeamsError::PermissionDenied(_)) => {
            return Ok(outcome(
                false,
                Some("not found or not accessible (already deleted, or not your file)"),
            ));
        }
        Err(e) => return Err(e),
    };

    let my_drive_id = match my_drive {
        Some(id) => id.as_str(),
        None => my_drive.insert(api::files::my_drive_id_at(client, base).await?),
    };
    let item_drive = drive_item
        .parent_reference
        .as_ref()
        .and_then(|r| r.drive_id.as_deref());
    if item_drive != Some(my_drive_id) {
        return Ok(outcome(
            false,
            Some("not in your OneDrive (shared by someone else, or a channel file)"),
        ));
    }
    // A folder delete is recursive; attachments are files.
    if drive_item.file.is_none() {
        return Ok(outcome(false, Some("not a file")));
    }
    let Some(item_id) = drive_item.id.as_deref() else {
        return Ok(outcome(false, Some("drive item has no id")));
    };

    match api::files::delete_drive_item_at(client, base, my_drive_id, item_id).await {
        Ok(()) => Ok(outcome(true, None)),
        Err(TeamsError::NotFound(_)) => Ok(outcome(true, Some("already deleted"))),
        Err(e) => Err(e),
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

    mod delete {
        use super::*;
        use crate::models::message::{ChatMessage, ChatMessageHostedContent};
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        const OWN_URL: &str = "https://tenant-my.sharepoint.com/personal/me/Documents/Microsoft%20Teams%20Chat%20Files/report.pdf";
        const FOREIGN_URL: &str =
            "https://tenant.sharepoint.com/sites/team/Shared%20Documents/General/plan.docx";

        fn test_client() -> GraphClient {
            GraphClient::new(
                crate::auth::token::TokenInfo {
                    access_token: "synthetic-test-token".into(),
                    expires_at: None,
                    token_type: "Bearer".into(),
                    scope: None,
                    refresh_token: None,
                    profile: "test".into(),
                },
                &crate::config::NetworkConfig {
                    timeout: 3,
                    max_retries: 0,
                    retry_backoff_base: 1,
                },
            )
            .unwrap()
        }

        /// Inventory of a message with one inline image (index 0) and file
        /// attachments for each of `urls` (index 1..).
        fn inventory(urls: &[&str]) -> Vec<AttachmentItem> {
            let attachments: Vec<serde_json::Value> = urls
                .iter()
                .enumerate()
                .map(|(i, url)| {
                    serde_json::json!({
                        "id": format!("att{i}"),
                        "contentType": "reference",
                        "contentUrl": url,
                        "name": format!("file{i}"),
                    })
                })
                .collect();
            let msg: ChatMessage = serde_json::from_value(serde_json::json!({
                "id": "m1",
                "body": {
                    "contentType": "html",
                    "content": "<img src=\"https://graph.microsoft.com/v1.0/chats/c/messages/m1/hostedContents/aW1n/$value\">",
                },
                "attachments": attachments,
            }))
            .unwrap();
            let hosted: Vec<ChatMessageHostedContent> =
                serde_json::from_value(serde_json::json!([{"id": "aW1n"}])).unwrap();
            build_inventory(&msg, &hosted)
        }

        async fn mount_me_drive(server: &MockServer, times: u64) {
            Mock::given(method("GET"))
                .and(path("/me/drive"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": "my-drive",
                })))
                .expect(times)
                .mount(server)
                .await;
        }

        async fn mount_share(server: &MockServer, url: &str, drive_id: &str, item_id: &str) {
            mount_share_body(
                server,
                url,
                serde_json::json!({
                    "id": item_id,
                    "name": "x",
                    "file": {"mimeType": "application/pdf"},
                    "parentReference": {"driveId": drive_id, "id": "parent"},
                }),
            )
            .await;
        }

        async fn mount_share_body(server: &MockServer, url: &str, body: serde_json::Value) {
            Mock::given(method("GET"))
                .and(path(format!(
                    "/shares/{}/driveItem",
                    endpoints::sharing_url_token(url)
                )))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .expect(1)
                .mount(server)
                .await;
        }

        #[tokio::test]
        async fn own_folder_is_never_deleted() {
            let server = MockServer::start().await;
            mount_me_drive(&server, 1).await;
            mount_share_body(
                &server,
                OWN_URL,
                serde_json::json!({
                    "id": "folder-item",
                    "folder": {"childCount": 3},
                    "parentReference": {"driveId": "my-drive", "id": "root"},
                }),
            )
            .await;
            mount_delete(&server, "folder-item", 204, 0).await;

            let items = inventory(&[OWN_URL]);
            let selected = select_for_delete(&items, Some(1)).unwrap();
            let results = delete_items_at(&test_client(), &server.uri(), &selected)
                .await
                .unwrap();
            assert_eq!(results[0]["deleted"], false);
            assert_eq!(results[0]["reason"], "not a file");
        }

        #[tokio::test]
        async fn server_error_after_a_delete_keeps_the_note_and_exit_code() {
            let server = MockServer::start().await;
            let second = "https://tenant-my.sharepoint.com/personal/me/Documents/Microsoft%20Teams%20Chat%20Files/c.pdf";
            mount_me_drive(&server, 1).await;
            mount_share(&server, OWN_URL, "my-drive", "own-item").await;
            mount_share(&server, second, "my-drive", "item-c").await;
            mount_delete(&server, "own-item", 204, 1).await;
            mount_delete(&server, "item-c", 503, 1).await;

            let items = inventory(&[OWN_URL, second]);
            let selected = select_for_delete(&items, None).unwrap();
            let err = delete_items_at(&test_client(), &server.uri(), &selected)
                .await
                .unwrap_err();
            assert_eq!(err.exit_code(), 8);
            assert!(err.to_string().contains("item(s) 1."), "{err}");
        }

        async fn mount_delete(server: &MockServer, item_id: &str, status: u16, times: u64) {
            Mock::given(method("DELETE"))
                .and(path(format!("/drives/my-drive/items/{item_id}")))
                .respond_with(ResponseTemplate::new(status))
                .expect(times)
                .mount(server)
                .await;
        }

        #[tokio::test]
        async fn own_upload_is_deleted_and_others_are_skipped() {
            let server = MockServer::start().await;
            mount_me_drive(&server, 1).await;
            mount_share(&server, OWN_URL, "my-drive", "own-item").await;
            mount_share(&server, FOREIGN_URL, "team-drive", "team-item").await;
            mount_delete(&server, "own-item", 204, 1).await;
            Mock::given(method("DELETE"))
                .and(path("/drives/team-drive/items/team-item"))
                .respond_with(ResponseTemplate::new(204))
                .expect(0)
                .mount(&server)
                .await;

            let items = inventory(&[OWN_URL, FOREIGN_URL]);
            let selected = select_for_delete(&items, None).unwrap();
            let results = delete_items_at(&test_client(), &server.uri(), &selected)
                .await
                .unwrap();

            assert_eq!(results.len(), 3);
            assert_eq!(results[0]["index"], 0);
            assert_eq!(results[0]["deleted"], false);
            assert!(results[0]["reason"]
                .as_str()
                .unwrap()
                .contains("inline image"));
            assert_eq!(results[1]["name"], "file0");
            assert_eq!(results[1]["deleted"], true);
            assert!(results[1].get("reason").is_none());
            assert_eq!(results[2]["deleted"], false);
            assert!(results[2]["reason"]
                .as_str()
                .unwrap()
                .contains("not in your OneDrive"));
        }

        #[tokio::test]
        async fn delete_404_counts_as_already_deleted() {
            let server = MockServer::start().await;
            mount_me_drive(&server, 1).await;
            mount_share(&server, OWN_URL, "my-drive", "own-item").await;
            mount_delete(&server, "own-item", 404, 1).await;

            let items = inventory(&[OWN_URL]);
            let selected = select_for_delete(&items, Some(1)).unwrap();
            let results = delete_items_at(&test_client(), &server.uri(), &selected)
                .await
                .unwrap();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0]["deleted"], true);
            assert_eq!(results[0]["reason"], "already deleted");
        }

        #[tokio::test]
        async fn unresolvable_share_is_skipped_without_delete() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(format!(
                    "/shares/{}/driveItem",
                    endpoints::sharing_url_token(OWN_URL)
                )))
                .respond_with(ResponseTemplate::new(404))
                .expect(1)
                .mount(&server)
                .await;
            mount_me_drive(&server, 0).await;
            mount_delete(&server, "own-item", 204, 0).await;

            let items = inventory(&[OWN_URL]);
            let selected = select_for_delete(&items, Some(1)).unwrap();
            let results = delete_items_at(&test_client(), &server.uri(), &selected)
                .await
                .unwrap();
            assert_eq!(results[0]["deleted"], false);
            assert!(results[0]["reason"].as_str().unwrap().contains("not found"));
        }

        #[tokio::test]
        async fn delete_failure_is_an_error_noting_earlier_deletes() {
            let server = MockServer::start().await;
            let second = "https://tenant-my.sharepoint.com/personal/me/Documents/Microsoft%20Teams%20Chat%20Files/b.pdf";
            mount_me_drive(&server, 1).await;
            mount_share(&server, OWN_URL, "my-drive", "own-item").await;
            mount_share(&server, second, "my-drive", "item-b").await;
            mount_delete(&server, "own-item", 204, 1).await;
            Mock::given(method("DELETE"))
                .and(path("/drives/my-drive/items/item-b"))
                .respond_with(ResponseTemplate::new(403))
                .expect(1)
                .mount(&server)
                .await;

            let items = inventory(&[OWN_URL, second]);
            let selected = select_for_delete(&items, None).unwrap();
            let err = delete_items_at(&test_client(), &server.uri(), &selected)
                .await
                .unwrap_err();
            assert_eq!(err.exit_code(), 4);
            let msg = err.to_string();
            assert!(msg.contains("Files.ReadWrite"), "{msg}");
            assert!(
                msg.contains("Deleted before the failure: item(s) 1."),
                "{msg}"
            );
        }

        #[tokio::test]
        async fn inline_image_only_selection_makes_no_requests() {
            let server = MockServer::start().await;
            let items = inventory(&[OWN_URL]);
            let selected = select_for_delete(&items, Some(0)).unwrap();
            let results = delete_items_at(&test_client(), &server.uri(), &selected)
                .await
                .unwrap();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0]["deleted"], false);
            assert!(server.received_requests().await.unwrap().is_empty());
        }

        #[test]
        fn index_selection_matches_list_indexes() {
            let items = inventory(&[OWN_URL, FOREIGN_URL]);
            let picked = select_for_delete(&items, Some(2)).unwrap();
            assert_eq!(picked.len(), 1);
            assert_eq!(picked[0].index, 2);
            assert_eq!(picked[0].content_url.as_deref(), Some(FOREIGN_URL));

            let err = select_for_delete(&items, Some(9)).unwrap_err();
            assert_eq!(err.exit_code(), 2);
            assert!(err.to_string().contains("3 item(s)"));

            assert_eq!(select_for_delete(&[], None).unwrap_err().exit_code(), 5);
        }
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
