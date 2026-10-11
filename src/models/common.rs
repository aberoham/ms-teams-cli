use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub request_id: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Set on paged list results: true when Graph has further pages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_count: Option<u64>,
}

impl Metadata {
    pub fn new() -> Self {
        Self {
            request_id: uuid::Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            api_version: Some("v1.0".to_string()),
            duration_ms: None,
            has_more: None,
            next_link: None,
            total_count: None,
        }
    }

    pub fn with_duration(mut self, ms: u64) -> Self {
        self.duration_ms = Some(ms);
        self
    }

    pub fn with_page(mut self, next_link: Option<&str>) -> Self {
        self.has_more = Some(next_link.is_some());
        self.next_link = next_link.map(str::to_string);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<u64>,
}

/// Standard JSON output envelope (PRD §8.2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope<T: Serialize> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
    pub metadata: Metadata,
}

impl<T: Serialize> Envelope<T> {
    pub fn success(data: T, metadata: Metadata) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
            metadata,
        }
    }
}

impl Envelope<()> {
    pub fn error(code: impl Into<String>, message: impl Into<String>, metadata: Metadata) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(ErrorBody {
                code: code.into(),
                message: message.into(),
                graph_error_code: None,
                status: None,
                retry_after: None,
            }),
            metadata,
        }
    }
}

/// Identity — used by meetings (participants) and files (createdBy/lastModifiedBy)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Container for identity references
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentitySet {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<Identity>,
}

/// Generic paginated response from Microsoft Graph API
#[derive(Debug, Clone, Deserialize)]
pub struct PageResponse<T> {
    pub value: Vec<T>,
    #[serde(rename = "@odata.nextLink")]
    pub next_link: Option<String>,
}

/// A list result: one page, or every page with `next_link` None.
#[derive(Debug, Clone)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_link: Option<String>,
}

impl<T> Page<T> {
    pub fn complete(items: Vec<T>) -> Self {
        Self {
            items,
            next_link: None,
        }
    }
}

impl<T> From<PageResponse<T>> for Page<T> {
    fn from(resp: PageResponse<T>) -> Self {
        Self {
            items: resp.value,
            next_link: resp.next_link,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_envelope_serializes_correctly() {
        let meta = Metadata::new().with_duration(42);
        let envelope = Envelope::success("hello", meta);
        let json = serde_json::to_value(&envelope).unwrap();

        assert_eq!(json["success"], true);
        assert_eq!(json["data"], "hello");
        assert!(json.get("error").is_none());
        assert_eq!(json["metadata"]["duration_ms"], 42);
        assert_eq!(json["metadata"]["api_version"], "v1.0");
    }

    #[test]
    fn error_envelope_serializes_correctly() {
        let meta = Metadata::new();
        let envelope = Envelope::<()>::error("AUTH_FAILED", "bad credentials", meta);
        let json = serde_json::to_value(&envelope).unwrap();

        assert_eq!(json["success"], false);
        assert!(json.get("data").is_none());
        assert_eq!(json["error"]["code"], "AUTH_FAILED");
        assert_eq!(json["error"]["message"], "bad credentials");
    }

    #[test]
    fn page_metadata_reports_continuation() {
        let more = serde_json::to_value(Metadata::new().with_page(Some("https://next"))).unwrap();
        assert_eq!(more["has_more"], true);
        assert_eq!(more["next_link"], "https://next");

        let last = serde_json::to_value(Metadata::new().with_page(None)).unwrap();
        assert_eq!(last["has_more"], false);
        assert!(last.get("next_link").is_none());

        let plain = serde_json::to_value(Metadata::new()).unwrap();
        assert!(plain.get("has_more").is_none());
    }

    #[test]
    fn metadata_has_uuid_and_timestamp() {
        let meta = Metadata::new();
        assert!(!meta.request_id.is_empty());
        assert!(!meta.timestamp.is_empty());
        // Validate UUID format (8-4-4-4-12)
        assert_eq!(meta.request_id.len(), 36);
    }

    #[test]
    fn page_response_with_next_link() {
        let json = r#"{
            "value": [1, 2, 3],
            "@odata.nextLink": "https://graph.microsoft.com/v1.0/next"
        }"#;
        let resp: PageResponse<i32> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.value, vec![1, 2, 3]);
        assert_eq!(
            resp.next_link.as_deref(),
            Some("https://graph.microsoft.com/v1.0/next")
        );
    }

    #[test]
    fn page_response_without_next_link() {
        let json = r#"{"value": ["a", "b"]}"#;
        let resp: PageResponse<String> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.value, vec!["a", "b"]);
        assert!(resp.next_link.is_none());
    }
}
