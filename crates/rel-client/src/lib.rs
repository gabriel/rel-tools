//! Typed Rust client for REL RPC v1.
//!
//! This module contains no desktop-app lifecycle or local-file behavior. It can
//! therefore be used by other Rust programs without adopting the bundled CLI's
//! macOS-specific conveniences.

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{self, BufRead, BufReader, Lines, Read};
use std::time::Duration;

pub mod transfer;

const DEFAULT_AGENT_PORT: u16 = 17_319;
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_PAGE_READ_MAX_CHARS: usize = 12_000;
const MIN_PAGE_READ_MAX_CHARS: usize = 512;
const MAX_PAGE_READ_MAX_CHARS: usize = 32_768;
const DEFAULT_PAGE_READ_MAX_SECTIONS: usize = 24;
const MAX_PAGE_READ_MAX_SECTIONS: usize = 100;

/// Stable application error codes for REL RPC v1.
///
/// Codes begin at 10,000 so they cannot be mistaken for HTTP transport
/// statuses. String error IDs remain available for readable diagnostics.
pub mod rpc_error_codes {
    pub const MINIMUM: u32 = 10_000;

    pub const INVALID_REQUEST: u32 = 10_000;
    pub const ROUTE_NOT_FOUND: u32 = 10_001;
    pub const METHOD_NOT_ALLOWED: u32 = 10_002;
    pub const PAYLOAD_TOO_LARGE: u32 = 10_003;
    pub const UNSUPPORTED_MEDIA_TYPE: u32 = 10_004;
    pub const VALIDATION_FAILED: u32 = 10_005;
    pub const UNSUPPORTED_MODALITY: u32 = 10_006;
    pub const OBSERVATION_TOO_LARGE: u32 = 10_007;

    pub const SESSION_NOT_FOUND: u32 = 10_100;
    pub const PAGE_NOT_FOUND: u32 = 10_101;
    pub const PAGE_MISMATCH: u32 = 10_102;
    pub const PROXY_NOT_FOUND: u32 = 10_103;
    pub const ACTIVE_PAGE_NOT_FOUND: u32 = 10_104;

    pub const CONFLICT: u32 = 10_200;
    pub const BROWSER_BUSY: u32 = 10_201;
    pub const NETWORK_PAUSED: u32 = 10_202;
    pub const ACTION_TARGET_NOT_FOUND: u32 = 10_203;
    pub const REQUEST_CANCELLED: u32 = 10_204;
    pub const RATE_LIMITED: u32 = 10_205;
    pub const ACTION_TIMEOUT: u32 = 10_206;
    pub const OBSERVATION_STALE: u32 = 10_207;
    pub const PRO_REQUIRED: u32 = 10_208;

    pub const UPSTREAM_UNAVAILABLE: u32 = 10_300;
    pub const BROWSER_UNAVAILABLE: u32 = 10_301;
    pub const AGENT_UNHEALTHY: u32 = 10_302;
    pub const TIMEOUT: u32 = 10_303;
    pub const PROXY_CONFIGURATION_FAILED: u32 = 10_304;
    pub const BROWSER_CREATION_FAILED: u32 = 10_305;
    pub const SEMANTIC_EXTRACTION_FAILED: u32 = 10_306;

    pub const INTERNAL_ERROR: u32 = 10_999;

    pub fn for_id(id: &str) -> u32 {
        match id {
            "INVALID_REQUEST" => INVALID_REQUEST,
            "ROUTE_NOT_FOUND" => ROUTE_NOT_FOUND,
            "METHOD_NOT_ALLOWED" => METHOD_NOT_ALLOWED,
            "PAYLOAD_TOO_LARGE" => PAYLOAD_TOO_LARGE,
            "UNSUPPORTED_MEDIA_TYPE" => UNSUPPORTED_MEDIA_TYPE,
            "VALIDATION_FAILED" => VALIDATION_FAILED,
            "UNSUPPORTED_MODALITY" => UNSUPPORTED_MODALITY,
            "OBSERVATION_TOO_LARGE" => OBSERVATION_TOO_LARGE,
            "SESSION_NOT_FOUND" => SESSION_NOT_FOUND,
            "PAGE_NOT_FOUND" => PAGE_NOT_FOUND,
            "PAGE_MISMATCH" => PAGE_MISMATCH,
            "PROXY_NOT_FOUND" => PROXY_NOT_FOUND,
            "ACTIVE_PAGE_NOT_FOUND" => ACTIVE_PAGE_NOT_FOUND,
            "CONFLICT" => CONFLICT,
            "BROWSER_BUSY" => BROWSER_BUSY,
            "NETWORK_PAUSED" => NETWORK_PAUSED,
            "ACTION_TARGET_NOT_FOUND" => ACTION_TARGET_NOT_FOUND,
            "REQUEST_CANCELLED" => REQUEST_CANCELLED,
            "RATE_LIMITED" => RATE_LIMITED,
            "ACTION_TIMEOUT" => ACTION_TIMEOUT,
            "OBSERVATION_STALE" => OBSERVATION_STALE,
            "PRO_REQUIRED" => PRO_REQUIRED,
            "UPSTREAM_UNAVAILABLE" => UPSTREAM_UNAVAILABLE,
            "BROWSER_UNAVAILABLE" => BROWSER_UNAVAILABLE,
            "AGENT_UNHEALTHY" => AGENT_UNHEALTHY,
            "TIMEOUT" => TIMEOUT,
            "PROXY_CONFIGURATION_FAILED" => PROXY_CONFIGURATION_FAILED,
            "BROWSER_CREATION_FAILED" => BROWSER_CREATION_FAILED,
            "SEMANTIC_EXTRACTION_FAILED" => SEMANTIC_EXTRACTION_FAILED,
            _ => INTERNAL_ERROR,
        }
    }

    pub fn is_valid(id: &str, code: u32) -> bool {
        if code < MINIMUM {
            return false;
        }
        let expected = for_id(id);
        (expected == INTERNAL_ERROR && id != "INTERNAL_ERROR") || code == expected
    }
}

#[derive(Clone, Debug)]
pub struct RelClient {
    base_url: String,
    request_timeout: Duration,
}

impl RelClient {
    /// Connect to the standard loopback REL RPC v1 endpoint.
    pub fn local() -> Self {
        let port = std::env::var("REL_AGENT_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|port| *port > 0)
            .unwrap_or(DEFAULT_AGENT_PORT);
        Self::new(format!("http://127.0.0.1:{port}/v1"))
    }

    /// Connect to an explicit RPC v1 base URL, such as
    /// `http://127.0.0.1:17319/v1`.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn health(&self) -> Result<RpcResponse<Health>, ClientError> {
        self.request::<Health, Value>("GET", "/health", None)
    }

    pub fn status(&self) -> Result<RpcResponse<StatusReport>, ClientError> {
        self.request::<StatusReport, Value>("GET", "/status", None)
    }

    /// List the bounded in-memory queue of website notifications that the user
    /// explicitly opted in to share with agents.
    pub fn list_notifications(
        &self,
    ) -> Result<RpcResponse<BrowserNotificationListData>, ClientError> {
        self.request::<BrowserNotificationListData, Value>("GET", "/notifications", None)
    }

    pub fn capture(&self, request: &CaptureRequest) -> Result<CaptureStream, ClientError> {
        let timeout = capture_request_timeout(request);
        let response = self.send("POST", "/captures", Some(request), timeout)?;
        match response {
            SentResponse::Success(response) => CaptureStream::from_response(response),
            SentResponse::Failure { status, response } => Err(parse_rpc_failure(status, response)),
        }
    }

    pub fn navigate(
        &self,
        request: &NavigateRequest,
    ) -> Result<RpcResponse<PageOperationData>, ClientError> {
        self.request_with_timeout(
            "POST",
            "/navigate",
            Some(request),
            page_request_timeout(request.timeout, request.wait),
        )
    }

    pub fn perform(
        &self,
        request: &PerformRequest,
    ) -> Result<RpcResponse<PageOperationData>, ClientError> {
        self.request_with_timeout(
            "POST",
            "/perform",
            Some(request),
            page_request_timeout(request.timeout, request.wait),
        )
    }

    pub fn capture_current_page(
        &self,
        request: &PageCaptureRequest,
    ) -> Result<RpcResponse<PageOperationData>, ClientError> {
        self.request_with_timeout(
            "POST",
            "/capture",
            Some(request),
            page_request_timeout(request.timeout, request.wait),
        )
    }

    pub fn screenshot_current_page(
        &self,
        request: &ScreenshotRequest,
    ) -> Result<RpcResponse<ScreenshotOperationData>, ClientError> {
        self.request_with_timeout(
            "POST",
            "/screenshot",
            Some(request),
            page_request_timeout(request.timeout, request.wait),
        )
    }

    /// Observe the current shorthand page using compact rendered semantics and,
    /// for hybrid or visual mode, a synchronized viewport screenshot.
    pub fn observe_current_page(
        &self,
        request: &ObservationRequest,
    ) -> Result<RpcResponse<ObservationOperationData>, ClientError> {
        self.request_with_timeout(
            "POST",
            "/observe",
            Some(request),
            page_request_timeout(request.timeout, request.wait),
        )
    }

    /// Navigate in embedded Chromium and return the first synchronized page
    /// observation without requiring a separate observe request.
    pub fn navigate_and_observe(
        &self,
        request: &NavigateObservationRequest,
    ) -> Result<RpcResponse<ObservationOperationData>, ClientError> {
        self.request_with_timeout(
            "POST",
            "/navigate/observe",
            Some(request),
            page_request_timeout(request.timeout, request.wait),
        )
    }

    /// Read either a URL or the current shorthand page as bounded,
    /// query-directed Markdown. This is a semantic-only convenience over the
    /// canonical `/navigate/observe` and `/observe` RPC v1 operations.
    pub fn read_page(
        &self,
        request: &PageReadRequest,
    ) -> Result<RpcResponse<PageReadData>, ClientError> {
        let (max_chars, max_sections) = page_read_limits(request.max_chars, request.max_sections)?;

        let response = if let Some(url) = request.url.as_deref() {
            self.navigate_and_observe(&NavigateObservationRequest {
                url: Some(url.to_string()),
                session_id: request.session_id.clone(),
                profile: request.profile.clone(),
                proxy: request.proxy.clone(),
                mode: Some(ObservationMode::Semantic),
                timeout: request.timeout,
                wait: request.wait,
                ..NavigateObservationRequest::default()
            })?
        } else {
            if request.profile.is_some() || request.proxy.is_some() {
                return Err(ClientError::Protocol(
                    "profile and proxy require a URL when reading a page".to_string(),
                ));
            }
            self.observe_current_page(&ObservationRequest {
                session_id: request.session_id.clone(),
                mode: Some(ObservationMode::Semantic),
                timeout: request.timeout,
                wait: request.wait,
            })?
        };

        let RpcResponse {
            status,
            request_id,
            data,
        } = response;
        Ok(RpcResponse {
            status,
            request_id,
            data: page_read_data(data, request.query.as_deref(), max_chars, max_sections),
        })
    }

    /// Return one retained public semantic observation. Interaction references
    /// may be stale after navigation; use this operation only for reading.
    pub fn get_observation(
        &self,
        observation_id: &str,
    ) -> Result<RpcResponse<ObservationOperationData>, ClientError> {
        self.request::<ObservationOperationData, Value>(
            "GET",
            &format!("/observations/{}", encode_path_segment(observation_id)),
            None,
        )
    }

    /// Re-read one retained public observation as bounded, query-directed
    /// Markdown without navigating the browser again.
    pub fn read_observation(
        &self,
        observation_id: &str,
        request: &ObservationReadRequest,
    ) -> Result<RpcResponse<PageReadData>, ClientError> {
        let (max_chars, max_sections) = page_read_limits(request.max_chars, request.max_sections)?;
        let RpcResponse {
            status,
            request_id,
            data,
        } = self.get_observation(observation_id)?;
        Ok(RpcResponse {
            status,
            request_id,
            data: page_read_data(data, request.query.as_deref(), max_chars, max_sections),
        })
    }

    pub fn attach_page(
        &self,
        request: &PageAttachRequest,
    ) -> Result<RpcResponse<PageOperationData>, ClientError> {
        self.request_with_timeout(
            "POST",
            "/pages",
            Some(request),
            page_request_timeout(request.timeout, request.wait),
        )
    }

    pub fn perform_page_action(
        &self,
        page_id: &str,
        request: &PageActionRequest,
    ) -> Result<RpcResponse<PageOperationData>, ClientError> {
        let path = format!("/pages/{}/actions", encode_path_segment(page_id));
        self.request_with_timeout(
            "POST",
            &path,
            Some(request),
            page_request_timeout(request.timeout, request.wait),
        )
    }

    pub fn take_page_screenshot(
        &self,
        page_id: &str,
        request: &PageScreenshotRequest,
    ) -> Result<RpcResponse<ScreenshotOperationData>, ClientError> {
        let path = format!("/pages/{}/screenshot", encode_path_segment(page_id));
        self.request_with_timeout(
            "POST",
            &path,
            Some(request),
            page_request_timeout(request.timeout, request.wait),
        )
    }

    /// Observe an attached page.
    pub fn observe_page(
        &self,
        page_id: &str,
        request: &PageObservationRequest,
    ) -> Result<RpcResponse<ObservationOperationData>, ClientError> {
        let path = format!("/pages/{}/observe", encode_path_segment(page_id));
        self.request_with_timeout(
            "POST",
            &path,
            Some(request),
            page_request_timeout(request.timeout, request.wait),
        )
    }

    /// Perform an allowlisted action through a reference from one observation.
    /// The response always contains a new post-action observation.
    pub fn perform_observation_action(
        &self,
        observation_id: &str,
        request: &ObservationActionRequest,
    ) -> Result<RpcResponse<ObservationOperationData>, ClientError> {
        let path = format!(
            "/observations/{}/actions",
            encode_path_segment(observation_id)
        );
        self.request_with_timeout(
            "POST",
            &path,
            Some(request),
            page_request_timeout(request.timeout, request.wait),
        )
    }

    /// Search one stored observation's public semantic snapshot.
    pub fn find_in_observation(
        &self,
        observation_id: &str,
        request: &ObservationFindRequest,
    ) -> Result<RpcResponse<ObservationFindData>, ClientError> {
        let path = format!("/observations/{}/find", encode_path_segment(observation_id));
        self.request("POST", &path, Some(request))
    }

    pub fn list_proxies(&self) -> Result<RpcResponse<ProxyListData>, ClientError> {
        self.request::<ProxyListData, Value>("GET", "/proxies", None)
    }

    pub fn get_proxy(&self, alias: &str) -> Result<RpcResponse<ProxyData>, ClientError> {
        self.request::<ProxyData, Value>(
            "GET",
            &format!("/proxies/{}", encode_path_segment(alias)),
            None,
        )
    }

    pub fn create_proxy(
        &self,
        request: &ProxyCreateRequest,
    ) -> Result<RpcResponse<ProxyData>, ClientError> {
        self.request("POST", "/proxies", Some(request))
    }

    pub fn update_proxy(
        &self,
        alias: &str,
        request: &ProxyUpdateRequest,
    ) -> Result<RpcResponse<ProxyData>, ClientError> {
        self.request(
            "PATCH",
            &format!("/proxies/{}", encode_path_segment(alias)),
            Some(request),
        )
    }

    pub fn delete_proxy(&self, alias: &str) -> Result<RpcResponse<ProxyDeletedData>, ClientError> {
        self.request::<ProxyDeletedData, Value>(
            "DELETE",
            &format!("/proxies/{}", encode_path_segment(alias)),
            None,
        )
    }

    pub fn rotate_proxy_session(&self, alias: &str) -> Result<RpcResponse<ProxyData>, ClientError> {
        self.request::<ProxyData, Value>(
            "POST",
            &format!("/proxies/{}/rotate-session", encode_path_segment(alias)),
            None,
        )
    }

    pub fn export_proxy_transfer(
        &self,
        request: &ProxyTransferExportRequest,
    ) -> Result<RpcResponse<TransferExportData>, ClientError> {
        self.request("POST", "/proxy-transfers/export", Some(request))
    }

    pub fn import_proxy_transfer(
        &self,
        request: &ProxyTransferImportRequest,
    ) -> Result<RpcResponse<ProxyData>, ClientError> {
        self.request("POST", "/proxy-transfers/import", Some(request))
    }

    pub fn list_sessions(&self) -> Result<RpcResponse<SessionListData>, ClientError> {
        self.request::<SessionListData, Value>("GET", "/sessions", None)
    }

    pub fn get_session(&self, id: &str) -> Result<RpcResponse<SessionData>, ClientError> {
        self.request::<SessionData, Value>(
            "GET",
            &format!("/sessions/{}", encode_path_segment(id)),
            None,
        )
    }

    pub fn create_session(
        &self,
        request: &SessionCreateRequest,
    ) -> Result<RpcResponse<SessionData>, ClientError> {
        self.request("POST", "/sessions", Some(request))
    }

    pub fn list_profiles(&self) -> Result<RpcResponse<ProfileListData>, ClientError> {
        self.request::<ProfileListData, Value>("GET", "/profiles", None)
    }

    pub fn create_profile(
        &self,
        request: &ProfileCreateRequest,
    ) -> Result<RpcResponse<ProfileData>, ClientError> {
        self.request("POST", "/profiles", Some(request))
    }

    pub fn update_profile_data(
        &self,
        id: &str,
        request: &ProfileDataUpdateRequest,
    ) -> Result<RpcResponse<ProfileData>, ClientError> {
        self.request(
            "PATCH",
            &format!("/profiles/{}", encode_path_segment(id)),
            Some(request),
        )
    }

    pub fn delete_profile(&self, id: &str) -> Result<RpcResponse<DeletedData>, ClientError> {
        self.request::<DeletedData, Value>(
            "DELETE",
            &format!("/profiles/{}", encode_path_segment(id)),
            None,
        )
    }

    pub fn export_profile_transfer(
        &self,
        request: &ProfileTransferExportRequest,
    ) -> Result<RpcResponse<TransferExportData>, ClientError> {
        self.request("POST", "/profile-transfers/export", Some(request))
    }

    pub fn import_profile_transfer(
        &self,
        request: &ProfileTransferImportRequest,
    ) -> Result<RpcResponse<ProfileData>, ClientError> {
        self.request("POST", "/profile-transfers/import", Some(request))
    }

    pub fn update_session(
        &self,
        id: &str,
        request: &SessionUpdateRequest,
    ) -> Result<RpcResponse<SessionData>, ClientError> {
        self.request(
            "PATCH",
            &format!("/sessions/{}", encode_path_segment(id)),
            Some(request),
        )
    }

    /// Pause all network activity in a persistent browser session.
    pub fn pause_session(
        &self,
        id: &str,
    ) -> Result<RpcResponse<SessionNetworkStateData>, ClientError> {
        self.request::<SessionNetworkStateData, Value>(
            "POST",
            &format!("/sessions/{}/pause", encode_path_segment(id)),
            None,
        )
    }

    /// Resume network activity and reload the current page when needed.
    pub fn play_session(
        &self,
        id: &str,
    ) -> Result<RpcResponse<SessionNetworkStateData>, ClientError> {
        self.request::<SessionNetworkStateData, Value>(
            "POST",
            &format!("/sessions/{}/play", encode_path_segment(id)),
            None,
        )
    }

    pub fn delete_session(&self, id: &str) -> Result<RpcResponse<DeletedData>, ClientError> {
        self.request::<DeletedData, Value>(
            "DELETE",
            &format!("/sessions/{}", encode_path_segment(id)),
            None,
        )
    }

    pub fn close_session_group(
        &self,
        group: &str,
    ) -> Result<RpcResponse<ClosedSessionGroupData>, ClientError> {
        self.request(
            "POST",
            "/sessions/close",
            Some(&SessionGroupCloseRequest { group }),
        )
    }

    fn request<T, B>(
        &self,
        method: &str,
        path: &str,
        body: Option<&B>,
    ) -> Result<RpcResponse<T>, ClientError>
    where
        T: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        self.request_with_timeout(method, path, body, self.request_timeout)
    }

    fn request_with_timeout<T, B>(
        &self,
        method: &str,
        path: &str,
        body: Option<&B>,
        timeout: Duration,
    ) -> Result<RpcResponse<T>, ClientError>
    where
        T: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        match self.send(method, path, body, timeout)? {
            SentResponse::Success(response) => parse_rpc_success(response),
            SentResponse::Failure { status, response } => Err(parse_rpc_failure(status, response)),
        }
    }

    fn send<B>(
        &self,
        method: &str,
        path: &str,
        body: Option<&B>,
        timeout: Duration,
    ) -> Result<SentResponse, ClientError>
    where
        B: Serialize + ?Sized,
    {
        if self.base_url.is_empty() {
            return Err(ClientError::Protocol(
                "REL RPC base URL cannot be empty".to_string(),
            ));
        }
        let url = format!("{}{}", self.base_url, path);
        let request = ureq::request(method, &url)
            .set("Accept", "application/json, application/x-ndjson")
            .timeout(timeout);
        let result = match body {
            Some(body) => {
                let body = serde_json::to_string(body).map_err(ClientError::Json)?;
                request
                    .set("Content-Type", "application/json")
                    .send_string(&body)
            }
            None => request.call(),
        };
        match result {
            Ok(response) => Ok(SentResponse::Success(response)),
            Err(ureq::Error::Status(status, response)) => {
                Ok(SentResponse::Failure { status, response })
            }
            Err(ureq::Error::Transport(error)) => Err(ClientError::Transport(error.to_string())),
        }
    }
}

enum SentResponse {
    Success(ureq::Response),
    Failure {
        status: u16,
        response: ureq::Response,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct RpcResponse<T> {
    pub status: String,
    pub request_id: String,
    pub data: T,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct RpcFailure {
    pub status: String,
    pub request_id: String,
    pub error: RpcError,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RpcError {
    pub id: String,
    pub code: u32,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl fmt::Display for RpcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.id, self.message)
    }
}

impl std::error::Error for RpcError {}

#[derive(Debug)]
pub enum ClientError {
    Transport(String),
    Protocol(String),
    Rpc(Box<RpcFailure>),
    Io(io::Error),
    Json(serde_json::Error),
}

impl ClientError {
    pub fn rpc_failure(&self) -> Option<&RpcFailure> {
        match self {
            Self::Rpc(failure) => Some(failure),
            _ => None,
        }
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(message) | Self::Protocol(message) => formatter.write_str(message),
            Self::Rpc(failure) => failure.error.fmt(formatter),
            Self::Io(error) => error.fmt(formatter),
            Self::Json(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Rpc(failure) => Some(&failure.error),
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Transport(_) | Self::Protocol(_) => None,
        }
    }
}

impl From<io::Error> for ClientError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn parse_rpc_success<T: DeserializeOwned>(
    response: ureq::Response,
) -> Result<RpcResponse<T>, ClientError> {
    validate_json_content_type(&response)?;
    let header_request_id = response.header("X-Request-Id").map(str::to_string);
    let body = response.into_string().map_err(ClientError::Io)?;
    let envelope = serde_json::from_str::<RpcResponse<T>>(&body).map_err(ClientError::Json)?;
    if envelope.status != "ok" {
        return Err(ClientError::Protocol(format!(
            "REL RPC success response has status {:?}",
            envelope.status
        )));
    }
    validate_request_id(header_request_id.as_deref(), &envelope.request_id)?;
    Ok(envelope)
}

fn parse_rpc_failure(status: u16, response: ureq::Response) -> ClientError {
    if let Err(error) = validate_json_content_type(&response) {
        return error;
    }
    let header_request_id = response.header("X-Request-Id").map(str::to_string);
    let body = match response.into_string() {
        Ok(body) => body,
        Err(error) => return ClientError::Io(error),
    };
    let failure = match serde_json::from_str::<RpcFailure>(&body) {
        Ok(failure) => failure,
        Err(error) => {
            return ClientError::Protocol(format!(
                "REL RPC returned HTTP {status} with an invalid error envelope: {error}"
            ))
        }
    };
    if failure.status != "error" {
        return ClientError::Protocol(format!(
            "REL RPC error response has status {:?}",
            failure.status
        ));
    }
    if let Err(error) = validate_request_id(header_request_id.as_deref(), &failure.request_id) {
        return error;
    }
    if failure.error.id.trim().is_empty()
        || !rpc_error_codes::is_valid(&failure.error.id, failure.error.code)
        || failure.error.message.trim().is_empty()
    {
        return ClientError::Protocol(
            "REL RPC error response has an incomplete error object".to_string(),
        );
    }
    if failure
        .error
        .details
        .as_ref()
        .is_some_and(|details| !details.is_object())
    {
        return ClientError::Protocol("REL RPC error details must be a JSON object".to_string());
    }
    ClientError::Rpc(Box::new(failure))
}

fn validate_json_content_type(response: &ureq::Response) -> Result<(), ClientError> {
    let content_type = response.header("Content-Type").unwrap_or_default();
    if content_type
        .to_ascii_lowercase()
        .starts_with("application/json")
    {
        Ok(())
    } else {
        Err(ClientError::Protocol(format!(
            "REL RPC returned unsupported Content-Type {content_type:?}"
        )))
    }
}

fn validate_request_id(header: Option<&str>, body: &str) -> Result<(), ClientError> {
    if body.trim().is_empty() {
        return Err(ClientError::Protocol(
            "REL RPC response is missing request_id".to_string(),
        ));
    }
    match header {
        Some(header) if header == body => Ok(()),
        Some(header) => Err(ClientError::Protocol(format!(
            "REL RPC request ID mismatch: header {header:?}, body {body:?}"
        ))),
        None => Err(ClientError::Protocol(
            "REL RPC response is missing X-Request-Id".to_string(),
        )),
    }
}

type ResponseReader = Box<dyn Read + Send + Sync + 'static>;

pub struct CaptureStream {
    request_id: String,
    lines: Lines<BufReader<ResponseReader>>,
    exit_code: Option<i32>,
    finished: bool,
}

impl CaptureStream {
    fn from_response(response: ureq::Response) -> Result<Self, ClientError> {
        let content_type = response.header("Content-Type").unwrap_or_default();
        if !content_type
            .to_ascii_lowercase()
            .starts_with("application/x-ndjson")
        {
            return Err(ClientError::Protocol(format!(
                "REL capture returned unsupported Content-Type {content_type:?}"
            )));
        }
        let request_id = response
            .header("X-Request-Id")
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                ClientError::Protocol("REL capture response is missing X-Request-Id".to_string())
            })?
            .to_string();
        Ok(Self {
            request_id,
            lines: BufReader::new(response.into_reader()).lines(),
            exit_code: None,
            finished: false,
        })
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Available after `capture.finished` has been read.
    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }
}

impl Iterator for CaptureStream {
    type Item = Result<CaptureEvent, ClientError>;

    fn next(&mut self) -> Option<Self::Item> {
        let line = loop {
            match self.lines.next()? {
                Ok(line) if line.trim().is_empty() => continue,
                Ok(line) => break line,
                Err(error) => return Some(Err(ClientError::Io(error))),
            }
        };
        let event = match serde_json::from_str::<CaptureEvent>(&line) {
            Ok(event) => event,
            Err(error) => return Some(Err(ClientError::Json(error))),
        };
        if event.request_id != self.request_id {
            return Some(Err(ClientError::Protocol(format!(
                "REL capture request ID mismatch: header {:?}, event {:?}",
                self.request_id, event.request_id
            ))));
        }
        if event.event.trim().is_empty() {
            return Some(Err(ClientError::Protocol(
                "REL capture event is missing its event name".to_string(),
            )));
        }
        match event.status.as_str() {
            "ok" if event.data.is_some() && event.error.is_none() => {}
            "error"
                if event.error.as_ref().is_some_and(|error| {
                    !error.id.trim().is_empty()
                        && rpc_error_codes::is_valid(&error.id, error.code)
                        && !error.message.trim().is_empty()
                }) => {}
            _ => {
                return Some(Err(ClientError::Protocol(format!(
                    "REL capture event {:?} has an invalid envelope",
                    event.event
                ))))
            }
        }
        if event.event == "capture.finished" {
            let exit_code = event
                .data
                .as_ref()
                .and_then(|data| data.get("exit_code"))
                .and_then(Value::as_i64)
                .and_then(|code| i32::try_from(code).ok());
            let Some(exit_code) = exit_code else {
                return Some(Err(ClientError::Protocol(
                    "REL capture.finished event is missing a valid exit_code".to_string(),
                )));
            };
            self.exit_code = Some(exit_code);
            self.finished = true;
        }
        Some(Ok(event))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct CaptureEvent {
    pub status: String,
    pub request_id: String,
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct CaptureRequest {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_delay: Option<f64>,
}

impl CaptureRequest {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct NavigateRequest {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait: Option<f64>,
}

impl NavigateRequest {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum Action {
    Click {
        selector: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mouse_move: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scroll: Option<bool>,
    },
    WaitFor {
        selector: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout: Option<f64>,
    },
    Type {
        selector: String,
        text: String,
    },
    Clear {
        selector: String,
    },
    Press {
        selector: String,
        key: String,
    },
    Select {
        selector: String,
        value: String,
    },
    ClickLink {
        link: String,
        #[serde(rename = "match")]
        match_rule: FuzzyLinkMatch,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mouse_move: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scroll: Option<bool>,
    },
    Wait {
        seconds: f64,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct FuzzyLinkMatch {
    #[serde(rename = "type")]
    kind: FuzzyLinkMatchType,
    pub threshold: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
enum FuzzyLinkMatchType {
    #[serde(rename = "fuzzy-link")]
    FuzzyLink,
}

impl FuzzyLinkMatch {
    pub fn new(threshold: f64) -> Self {
        Self {
            kind: FuzzyLinkMatchType::FuzzyLink,
            threshold,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct PageAttachRequest {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait: Option<f64>,
}

impl PageAttachRequest {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct PageActionRequest {
    pub action: Action,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait: Option<f64>,
}

impl PageActionRequest {
    pub fn new(action: Action) -> Self {
        Self {
            action,
            output: None,
            timeout: None,
            wait: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct PerformRequest {
    pub actions: Vec<Action>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait: Option<f64>,
}

impl PerformRequest {
    pub fn new(actions: Vec<Action>) -> Self {
        Self {
            actions,
            session_id: None,
            output: None,
            timeout: None,
            wait: None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct PageCaptureRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PageOperationData {
    pub page: Page,
    pub capture: PageCapture,
    #[serde(default)]
    pub closed_session_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Page {
    pub id: String,
    pub session_id: String,
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PageCapture {
    pub output_path: String,
    pub bytesize: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_http_status: Option<u16>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ScreenshotFormat {
    Png,
    Jpeg,
    Webp,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct ScreenshotRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<ScreenshotFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<u8>,
    #[serde(default)]
    pub full_page: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct PageScreenshotRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<ScreenshotFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<u8>,
    #[serde(default)]
    pub full_page: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ScreenshotOperationData {
    pub page: Page,
    pub screenshot: ScreenshotCapture,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ScreenshotCapture {
    pub output_path: String,
    pub bytesize: usize,
    pub format: ScreenshotFormat,
    pub mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

/// Public observation modalities. `auto` is intentionally an AI-harness
/// policy and is not accepted by RPC v1.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ObservationMode {
    #[default]
    Semantic,
    Hybrid,
    Visual,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct ObservationRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<ObservationMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct NavigateObservationRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub navigation: Option<ObservationNavigation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<ObservationMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait: Option<f64>,
}

impl NavigateObservationRequest {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: Some(url.into()),
            ..Self::default()
        }
    }
}

/// Parameters for a bounded semantic read of either a URL or the current
/// shorthand page.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct PageReadRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_sections: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct ObservationReadRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_sections: Option<usize>,
}

impl PageReadRequest {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: Some(url.into()),
            ..Self::default()
        }
    }
}

/// A compact page representation intended for research and reading rather
/// than interaction. Use an observation when action references are needed.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PageReadData {
    pub page: Page,
    pub observation_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    pub markdown: String,
    pub selected_outline_count: usize,
    pub selected_content_count: usize,
    pub selected_link_count: usize,
    pub available_content_count: usize,
    pub available_link_count: usize,
    pub source_truncated: bool,
    pub truncated: bool,
    pub matched_query: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ObservationNavigation {
    Url,
    Back,
    Forward,
    Reload,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct PageObservationRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<ObservationMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait: Option<f64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ObservationActionKind {
    Click,
    Type,
    Clear,
    Press,
    Select,
    Hover,
    Scroll,
    Wait,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ObservationAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "ref")]
    pub element_ref: Option<String>,
    pub action: ObservationActionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mouse_move: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Native horizontal wheel delta. Negative scrolls right; positive scrolls left.
    pub delta_x: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Native vertical wheel delta. Negative scrolls down; positive scrolls up.
    pub delta_y: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
}

impl ObservationAction {
    pub fn new(element_ref: impl Into<String>, action: ObservationActionKind) -> Self {
        Self {
            element_ref: Some(element_ref.into()),
            action,
            text: None,
            key: None,
            value: None,
            mouse_move: None,
            scroll: None,
            delta_x: None,
            delta_y: None,
            seconds: None,
        }
    }

    pub fn scroll(delta_x: i32, delta_y: i32) -> Self {
        Self {
            element_ref: None,
            action: ObservationActionKind::Scroll,
            text: None,
            key: None,
            value: None,
            mouse_move: None,
            scroll: None,
            delta_x: Some(delta_x),
            delta_y: Some(delta_y),
            seconds: None,
        }
    }

    pub fn wait(seconds: f64) -> Self {
        Self {
            element_ref: None,
            action: ObservationActionKind::Wait,
            text: None,
            key: None,
            value: None,
            mouse_move: None,
            scroll: None,
            delta_x: None,
            delta_y: None,
            seconds: Some(seconds),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ObservationActionRequest {
    pub actions: Vec<ObservationAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<ObservationMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait: Option<f64>,
}

impl ObservationActionRequest {
    pub fn new(element_ref: impl Into<String>, action: ObservationActionKind) -> Self {
        Self {
            actions: vec![ObservationAction::new(element_ref, action)],
            mode: None,
            timeout: None,
            wait: None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct ObservationFindRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ObservationFindData {
    pub observation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub matches: Vec<ObservationFindMatch>,
    pub total_matches: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ObservationFindMatch {
    Content {
        index: usize,
        content: ObservationContent,
    },
    Element {
        element: ObservationElement,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ObservationOperationData {
    pub page: Page,
    pub observation: PageObservation,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PageObservation {
    pub id: String,
    pub mode: ObservationMode,
    pub document_sequence: u64,
    pub captured_at: String,
    pub title: String,
    pub truncated: bool,
    pub omitted_node_count: usize,
    pub clipped_text_count: usize,
    pub visited_node_count: usize,
    pub semantic_bytes: usize,
    pub viewport: ObservationViewport,
    pub content: Vec<ObservationContent>,
    pub elements: Vec<ObservationElement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<ObservationScreenshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ObservationViewport {
    pub css_width: u32,
    pub css_height: u32,
    pub scroll_x: u32,
    pub scroll_y: u32,
    pub document_width: u32,
    pub document_height: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ObservationContent {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ObservationElement {
    #[serde(rename = "ref")]
    pub element_ref: String,
    pub role: String,
    pub name: String,
    pub states: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub in_viewport: bool,
    pub bounds: ObservationBounds,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ObservationBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ObservationScreenshot {
    pub output_path: String,
    pub bytesize: usize,
    pub format: ScreenshotFormat,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub css_to_image_scale_x: f64,
    pub css_to_image_scale_y: f64,
}

fn page_read_data(
    data: ObservationOperationData,
    query: Option<&str>,
    max_chars: usize,
    max_sections: usize,
) -> PageReadData {
    let ObservationOperationData { page, observation } = data;
    let normalized_query = query.map(str::trim).filter(|value| !value.is_empty());
    let terms = page_read_query_terms(normalized_query.unwrap_or_default());
    let query_active = normalized_query.is_some();

    let scored_content = observation
        .content
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let mut score =
                page_read_match_score(&item.text, normalized_query, &terms, item.kind == "heading");
            if page_read_query_requests_ratings(&terms) && page_read_text_is_rating(&item.text) {
                score = score.max(2);
            }
            (index, score)
        })
        .collect::<Vec<_>>();
    let content_matched = query_active && scored_content.iter().any(|(_, score)| *score > 0);
    let mut content = if content_matched {
        let mut ranked = scored_content
            .iter()
            .copied()
            .filter(|(_, score)| *score > 0)
            .collect::<Vec<_>>();
        ranked.sort_by_key(|(index, score)| (std::cmp::Reverse(*score), *index));
        page_read_matched_content_with_context(&observation.content, &ranked)
    } else if !query_active {
        page_read_coverage_content(&observation.content, max_sections)
    } else {
        Vec::new()
    };
    let available_content_count = if query_active {
        content.len()
    } else {
        observation.content.len()
    };
    content.truncate(max_sections);

    let mut seen_links = BTreeSet::new();
    let mut links = observation
        .elements
        .iter()
        .enumerate()
        .filter_map(|(index, element)| {
            let destination = element.destination.as_deref()?.trim();
            if destination.is_empty() {
                return None;
            }
            let key = format!("{}\n{}", element.name, destination);
            if !seen_links.insert(key) {
                return None;
            }
            let score = page_read_match_score(&element.name, normalized_query, &terms, false)
                .max(page_read_link_intent_score(destination, &terms));
            Some((index, score))
        })
        .collect::<Vec<_>>();
    let links_matched = query_active && links.iter().any(|(_, score)| *score > 0);
    if links_matched {
        links.retain(|(_, score)| *score > 0);
        links.sort_by_key(|(index, score)| (std::cmp::Reverse(*score), *index));
    } else if content_matched {
        links.clear();
    }
    let available_link_count = links.len();
    links.truncate(max_sections);

    let mut markdown = String::new();
    let title = observation.title.trim().to_string();
    let heading = if title.is_empty() {
        page.url.as_str()
    } else {
        title.as_str()
    };
    push_page_read_block(
        &mut markdown,
        &format!("# {}", escape_markdown_text(heading)),
        max_chars,
    );
    push_page_read_block(
        &mut markdown,
        &format!("Source: <{}>", escape_markdown_url(&page.url)),
        max_chars,
    );
    if let Some(query) = normalized_query {
        push_page_read_block(
            &mut markdown,
            &format!("Query: {}", escape_markdown_text(query)),
            max_chars,
        );
    }

    let outline = page_read_outline(&observation.content, max_sections.min(16));
    let mut selected_outline_count = 0;
    if !outline.is_empty() && push_page_read_block(&mut markdown, "## Page outline", max_chars) {
        for index in outline {
            let heading = &observation.content[index];
            let indent = "  ".repeat(heading.level.unwrap_or(2).saturating_sub(2) as usize);
            let block = format!("{indent}- {}", escape_markdown_text(heading.text.trim()));
            if push_page_read_block(&mut markdown, &block, max_chars) {
                selected_outline_count += 1;
            } else {
                break;
            }
        }
    }

    let mut selected_content_count = 0;
    let mut output_truncated = false;
    for (index, _) in &content {
        let (block, block_clipped) = page_read_bounded_content_block(
            page_read_content_markdown(&observation.content[*index]),
            max_chars,
        );
        output_truncated |= block_clipped;
        let (added, clipped) = push_page_read_excerpt(&mut markdown, &block, max_chars);
        if added {
            selected_content_count += 1;
        }
        if clipped {
            output_truncated = true;
            continue;
        }
    }

    let mut selected_link_count = 0;
    if !links.is_empty() && push_page_read_block(&mut markdown, "## Links", max_chars) {
        for (index, _) in &links {
            let element = &observation.elements[*index];
            let destination = element.destination.as_deref().unwrap_or_default();
            let label = if element.name.trim().is_empty() {
                destination
            } else {
                element.name.trim()
            };
            let block = format!(
                "- [{}](<{}>)",
                escape_markdown_text(label),
                escape_markdown_url(destination)
            );
            if push_page_read_block(&mut markdown, &block, max_chars) {
                selected_link_count += 1;
            } else {
                output_truncated = true;
                break;
            }
        }
    } else if !links.is_empty() {
        output_truncated = true;
    }

    let content_was_limited = available_content_count > selected_content_count;
    let links_were_limited = available_link_count > selected_link_count;

    PageReadData {
        page,
        observation_id: observation.id,
        title,
        query: normalized_query.map(str::to_string),
        markdown,
        selected_outline_count,
        selected_content_count,
        selected_link_count,
        available_content_count,
        available_link_count,
        source_truncated: observation.truncated,
        truncated: content_was_limited || links_were_limited || output_truncated,
        matched_query: content_matched || links_matched,
    }
}

fn page_read_matched_content_with_context(
    content: &[ObservationContent],
    ranked: &[(usize, usize)],
) -> Vec<(usize, usize)> {
    let mut selected = BTreeSet::new();
    for (index, _) in ranked {
        if let Some(heading) = (0..=*index)
            .rev()
            .find(|candidate| content[*candidate].kind == "heading")
        {
            selected.insert(heading);
        }
        selected.insert(*index);
        if page_read_text_is_rating(&content[*index].text) && *index > 0 {
            selected.insert(*index - 1);
        }
        if *index > 0 && page_read_text_is_rating(&content[*index - 1].text) {
            selected.insert(*index - 1);
        }
        if *index + 1 < content.len() && page_read_text_is_rating(&content[*index + 1].text) {
            selected.insert(*index + 1);
        }
    }
    selected
        .into_iter()
        .map(|index| {
            let score = ranked
                .iter()
                .find_map(|(candidate, score)| (*candidate == index).then_some(*score))
                .unwrap_or_default();
            (index, score)
        })
        .collect()
}

fn page_read_coverage_content(content: &[ObservationContent], limit: usize) -> Vec<(usize, usize)> {
    if content.len() <= limit {
        return (0..content.len()).map(|index| (index, 1)).collect();
    }
    if limit == 1 {
        return vec![(0, 1)];
    }
    let mut selected = BTreeSet::new();
    selected.insert(0);
    selected.insert(content.len() - 1);
    let heading_budget = (limit / 3).clamp(1, 8);
    let headings = content
        .iter()
        .enumerate()
        .filter_map(|(index, item)| (item.kind == "heading").then_some(index))
        .collect::<Vec<_>>();
    for index in evenly_spaced_indices(headings.len(), heading_budget.min(headings.len())) {
        selected.insert(headings[index]);
    }
    for index in evenly_spaced_indices(content.len(), limit) {
        if selected.len() >= limit {
            break;
        }
        selected.insert(index);
    }
    if selected.len() < limit {
        for index in 0..content.len() {
            if selected.len() >= limit {
                break;
            }
            selected.insert(index);
        }
    }
    selected.into_iter().map(|index| (index, 1)).collect()
}

fn evenly_spaced_indices(length: usize, count: usize) -> Vec<usize> {
    match (length, count) {
        (_, 0) | (0, _) => Vec::new(),
        (_, 1) => vec![0],
        _ if count >= length => (0..length).collect(),
        _ => (0..count)
            .map(|slot| slot * (length - 1) / (count - 1))
            .collect(),
    }
}

fn page_read_outline(content: &[ObservationContent], limit: usize) -> Vec<usize> {
    let headings = content
        .iter()
        .enumerate()
        .filter_map(|(index, item)| (item.kind == "heading").then_some(index))
        .collect::<Vec<_>>();
    evenly_spaced_indices(headings.len(), limit.min(headings.len()))
        .into_iter()
        .map(|index| headings[index])
        .collect()
}

fn page_read_limits(
    max_chars: Option<usize>,
    max_sections: Option<usize>,
) -> Result<(usize, usize), ClientError> {
    let max_chars = max_chars.unwrap_or(DEFAULT_PAGE_READ_MAX_CHARS);
    if !(MIN_PAGE_READ_MAX_CHARS..=MAX_PAGE_READ_MAX_CHARS).contains(&max_chars) {
        return Err(ClientError::Protocol(format!(
            "max_chars must be between {MIN_PAGE_READ_MAX_CHARS} and {MAX_PAGE_READ_MAX_CHARS}"
        )));
    }
    let max_sections = max_sections.unwrap_or(DEFAULT_PAGE_READ_MAX_SECTIONS);
    if !(1..=MAX_PAGE_READ_MAX_SECTIONS).contains(&max_sections) {
        return Err(ClientError::Protocol(format!(
            "max_sections must be between 1 and {MAX_PAGE_READ_MAX_SECTIONS}"
        )));
    }
    Ok((max_chars, max_sections))
}

fn page_read_query_terms(query: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "how", "in", "is", "it",
        "of", "on", "or", "that", "the", "this", "to", "was", "what", "when", "where", "which",
        "who", "why", "with",
    ];
    let mut terms = query
        .split(|character: char| !character.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|term| term.len() >= 2 && !STOP_WORDS.contains(&term.as_str()))
        .collect::<BTreeSet<_>>();
    let originals = terms.iter().cloned().collect::<Vec<_>>();
    for term in originals {
        match term.as_str() {
            "critic" | "critics" | "rating" | "ratings" | "score" | "scores" => {
                terms.extend(["review", "reviews", "rating", "score"].map(str::to_string));
            }
            "genre" | "genres" => {
                terms.extend(["genre", "genres", "style"].map(str::to_string));
            }
            _ => {}
        }
    }
    terms.into_iter().collect()
}

fn page_read_query_requests_ratings(terms: &[String]) -> bool {
    terms.iter().any(|term| {
        matches!(
            term.as_str(),
            "critic"
                | "critics"
                | "rating"
                | "ratings"
                | "review"
                | "reviews"
                | "score"
                | "scores"
                | "signal"
        )
    })
}

fn page_read_text_is_rating(text: &str) -> bool {
    let text = text.trim();
    if text.parse::<u8>().is_ok_and(|value| value <= 100) {
        return true;
    }
    text.split_whitespace().any(|token| {
        let token = token.trim_matches(|character: char| {
            !character.is_ascii_digit() && !matches!(character, '.' | '/' | '%')
        });
        if let Some(percent) = token.strip_suffix('%') {
            return percent
                .parse::<f64>()
                .is_ok_and(|value| (0.0..=100.0).contains(&value));
        }
        if let Some((value, scale)) = token.split_once('/') {
            return value
                .parse::<f64>()
                .ok()
                .zip(scale.parse::<f64>().ok())
                .is_some_and(|(value, scale)| scale > 0.0 && value >= 0.0 && value <= scale);
        }
        token.contains('.')
            && token
                .parse::<f64>()
                .is_ok_and(|value| (0.0..=100.0).contains(&value))
    })
}

fn page_read_link_intent_score(destination: &str, terms: &[String]) -> usize {
    let destination = destination.to_ascii_lowercase();
    let has_term =
        |candidates: &[&str]| terms.iter().any(|term| candidates.contains(&term.as_str()));
    if destination.contains("/genres/") && has_term(&["genre", "genres", "style"]) {
        return 6;
    }
    if destination.contains("/label/") && has_term(&["label", "labels"]) {
        return 6;
    }
    0
}

fn page_read_match_score(
    text: &str,
    query: Option<&str>,
    terms: &[String],
    is_heading: bool,
) -> usize {
    let Some(query) = query else {
        return 1;
    };
    let text = text.to_lowercase();
    let query = query.to_lowercase();
    let mut score = if !query.is_empty() && text.contains(&query) {
        12
    } else {
        0
    };
    for term in terms {
        score += text.matches(term).count().min(4) * 3;
    }
    if is_heading && score > 0 {
        score *= 2;
    }
    score
}

fn page_read_content_markdown(content: &ObservationContent) -> String {
    let text = escape_markdown_text(content.text.trim());
    let block = match content.kind.as_str() {
        "heading" => format!(
            "{} {text}",
            "#".repeat(content.level.unwrap_or(2).clamp(2, 6) as usize)
        ),
        "listitem" | "list_item" | "item" => format!("- {text}"),
        _ => text,
    };
    match content
        .context
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(context) => format!("Context: {}\n\n{block}", escape_markdown_text(context)),
        None => block,
    }
}

fn page_read_bounded_content_block(block: String, max_chars: usize) -> (String, bool) {
    let maximum = (max_chars / 3).clamp(128, 2_048);
    if block.chars().count() <= maximum {
        return (block, false);
    }
    let mut bounded = block
        .chars()
        .take(maximum.saturating_sub(1))
        .collect::<String>();
    bounded.push('…');
    (bounded, true)
}

fn push_page_read_block(output: &mut String, block: &str, max_chars: usize) -> bool {
    let separator = if output.is_empty() { "" } else { "\n\n" };
    let required = separator.chars().count() + block.chars().count();
    if output.chars().count() + required > max_chars {
        return false;
    }
    output.push_str(separator);
    output.push_str(block);
    true
}

fn push_page_read_excerpt(output: &mut String, block: &str, max_chars: usize) -> (bool, bool) {
    if push_page_read_block(output, block, max_chars) {
        return (true, false);
    }
    let separator = if output.is_empty() { "" } else { "\n\n" };
    let used = output.chars().count() + separator.chars().count();
    let available = max_chars.saturating_sub(used);
    if available < 2 {
        return (false, true);
    }
    output.push_str(separator);
    output.extend(block.chars().take(available - 1));
    output.push('…');
    (true, true)
}

fn escape_markdown_text(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' | '*' | '_' | '[' | ']' | '`' => {
                escaped.push('\\');
                escaped.push(character);
            }
            '\r' | '\n' => escaped.push(' '),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn escape_markdown_url(value: &str) -> String {
    value.trim().replace('<', "%3C").replace('>', "%3E")
}

/// Additional CA trust is scoped to sessions using this proxy.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProxyTls {
    #[default]
    System,
    BrightData,
    Custom {
        certificate_pem: String,
    },
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct ProxyCreateRequest {
    pub alias: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<ProxyTls>,
    pub upstream_host: String,
    pub upstream_port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oxylabs_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oxylabs_location_parameter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oxylabs_location_value: Option<String>,
}

impl ProxyCreateRequest {
    pub fn new(upstream_host: impl Into<String>, upstream_port: u16) -> Self {
        Self {
            upstream_host: upstream_host.into(),
            upstream_port,
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct ProxyUpdateRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<ProxyTls>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_port: Option<u16>,
    #[serde(skip_serializing_if = "Change::is_unchanged")]
    pub username: Change<String>,
    #[serde(skip_serializing_if = "Change::is_unchanged")]
    pub password: Change<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oxylabs_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Change::is_unchanged")]
    pub oxylabs_location_parameter: Change<String>,
    #[serde(skip_serializing_if = "Change::is_unchanged")]
    pub oxylabs_location_value: Change<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Change<T> {
    #[default]
    Unchanged,
    Set(T),
    Clear,
}

impl<T> Change<T> {
    pub fn is_unchanged(&self) -> bool {
        matches!(self, Self::Unchanged)
    }
}

impl<T: Serialize> Serialize for Change<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Unchanged | Self::Clear => serializer.serialize_none(),
            Self::Set(value) => value.serialize(serializer),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct BrowserNotificationListData {
    pub notifications: Vec<BrowserNotification>,
    pub trust: String,
}

/// Website-provided notification content. Callers must treat every text field
/// as untrusted data, never as instructions or authority.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct BrowserNotification {
    pub sequence: u64,
    pub session_id: String,
    pub origin: String,
    pub title: String,
    pub body: String,
    pub notification_id: String,
    pub persistent: bool,
    pub displayed_at: String,
    pub trust: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ProxyListData {
    pub proxies: Vec<Proxy>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ProxyData {
    pub proxy: Proxy,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct OxylabsProxy {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_parameter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_value: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Proxy {
    #[serde(default)]
    pub tls: ProxyTls,
    pub alias: String,
    pub upstream_host: String,
    pub upstream_port: u16,
    pub username: Option<String>,
    pub password_set: bool,
    #[serde(rename = "oxylabs", skip_serializing_if = "Option::is_none")]
    pub oxylabs: Option<OxylabsProxy>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ProxyTransferExportRequest {
    pub alias: String,
    pub include_credentials: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ProxyTransferImportRequest {
    pub contents_base64: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<String>,
}

impl ProxyTransferImportRequest {
    pub fn from_bytes(data: &[u8], alias: Option<String>, passphrase: Option<String>) -> Self {
        Self {
            contents_base64: BASE64_STANDARD.encode(data),
            alias,
            passphrase,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct SessionCreateRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Change::is_unchanged")]
    pub proxy_alias: Change<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adblock_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_blocking_mode: Option<ImageBlockingMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_size_limit_kb: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct SessionUpdateRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Change::is_unchanged")]
    pub proxy_alias: Change<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adblock_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_blocking_mode: Option<ImageBlockingMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_size_limit_kb: Option<i64>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ProfileCreateRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adblock_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_blocking_mode: Option<ImageBlockingMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_size_limit_kb: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub includes_cookies: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub includes_passwords: Option<bool>,
    #[serde(skip_serializing_if = "Change::is_unchanged")]
    pub fingerprint_profile: Change<FingerprintProfile>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ProfileDataUpdateRequest {
    pub includes_cookies: bool,
    pub includes_passwords: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ImageBlockingMode {
    None,
    All,
    OverLimit,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct FingerprintProfile {
    pub schema_version: u64,
    pub seed: String,
    pub platform: FingerprintPlatform,
    pub browser_brand: FingerprintBrowserBrand,
    pub browser_version: String,
    pub user_agent: String,
    pub locale: String,
    pub timezone: String,
    pub network_profile: FingerprintNetworkProfile,
    pub hardware_concurrency: u64,
    pub device_memory_gib: u64,
    pub max_touch_points: u64,
    pub screen: FingerprintScreen,
    pub graphics_profile: String,
    pub storage_quota_bytes: u64,
    pub canvas_noise_mode: FingerprintNoiseMode,
    pub audio_noise_mode: FingerprintNoiseMode,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FingerprintPlatform {
    Macos,
    Linux,
    Windows,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FingerprintBrowserBrand {
    Chromium,
    Chrome,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FingerprintNetworkProfile {
    Desktop,
    Residential,
    Datacenter,
    Mobile,
    Slow,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FingerprintNoiseMode {
    Native,
    Deterministic,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct FingerprintScreen {
    pub width: u64,
    pub height: u64,
    pub available_height: u64,
    pub device_scale_factor: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SessionListData {
    pub sessions: Vec<Session>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SessionData {
    pub session: Session,
    #[serde(default)]
    pub closed_session_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SessionNetworkStateData {
    pub session_id: String,
    pub network_paused: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ProfileListData {
    pub profiles: Vec<Profile>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ProfileData {
    pub profile: Profile,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub proxy_alias: Option<String>,
    pub adblock_enabled: bool,
    pub image_blocking_mode: ImageBlockingMode,
    pub image_size_limit_kb: i64,
    pub includes_cookies: bool,
    pub includes_passwords: bool,
    #[serde(default)]
    pub fingerprint_profile: Option<FingerprintProfile>,
    pub is_builtin: bool,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct TransferExportData {
    pub filename: String,
    pub contents_base64: String,
}

impl TransferExportData {
    pub fn contents(&self) -> Result<Vec<u8>, String> {
        BASE64_STANDARD
            .decode(&self.contents_base64)
            .map_err(|error| format!("REL returned invalid transfer data: {error}"))
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ProfileTransferExportRequest {
    pub name: String,
    pub include_cookies: bool,
    pub include_passwords: bool,
    pub include_proxy_credentials: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ProfileTransferImportRequest {
    pub contents_base64: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub browser_data_ready: bool,
}

impl ProfileTransferImportRequest {
    pub fn from_bytes(data: &[u8], name: Option<String>, passphrase: Option<String>) -> Self {
        Self {
            contents_base64: BASE64_STANDARD.encode(data),
            name,
            passphrase,
            browser_data_ready: false,
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub profile: String,
    pub profile_data_id: Option<String>,
    pub group: Option<String>,
    pub proxy_alias: Option<String>,
    pub adblock_enabled: bool,
    pub image_blocking_mode: ImageBlockingMode,
    pub image_size_limit_kb: i64,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct DeletedData {
    pub deleted_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ClosedSessionGroupData {
    pub group: String,
    pub deleted_ids: Vec<String>,
}

#[derive(Serialize)]
struct SessionGroupCloseRequest<'a> {
    group: &'a str,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ProxyDeletedData {
    pub deleted_alias: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Health {
    pub version: String,
    pub pid: u32,
    pub browser_proxy_port: u16,
    pub build: Option<BuildIdentity>,
    pub worker: Worker,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct BuildIdentity {
    pub id: String,
    pub configuration: String,
    pub worktree: String,
    pub branch: String,
    pub commit: String,
    pub dirty: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Worker {
    pub state: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct StatusReport {
    pub overall_status: String,
    pub running_count: usize,
    pub total_count: usize,
    pub build: Option<BuildIdentity>,
    pub checks: Vec<StatusCheck>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct StatusCheck {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub running: bool,
    pub status: String,
    pub detail: String,
    pub pids: Vec<u32>,
}

fn capture_request_timeout(request: &CaptureRequest) -> Duration {
    let timeout = request.timeout.unwrap_or(90.0);
    let wait = request.wait.unwrap_or(1.0);
    let retries = request.retry.unwrap_or(1);
    let retry_delay = request.retry_delay.unwrap_or(3.0);
    let attempts = f64::from(retries) + 1.0;
    bounded_duration(
        ((timeout + wait + 30.0).max(30.0) * attempts) + retry_delay * f64::from(retries),
        180.0,
    )
}

fn page_request_timeout(timeout: Option<f64>, wait: Option<f64>) -> Duration {
    bounded_duration(timeout.unwrap_or(90.0) + wait.unwrap_or(1.0) + 30.0, 120.0)
}

fn bounded_duration(seconds: f64, fallback: f64) -> Duration {
    const MAX_CLIENT_TIMEOUT_SECONDS: f64 = 7.0 * 24.0 * 60.0 * 60.0;
    let seconds = if seconds.is_finite() && seconds > 0.0 {
        seconds.min(MAX_CLIENT_TIMEOUT_SECONDS)
    } else {
        fallback
    };
    Duration::from_secs_f64(seconds)
}

fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread::{self, JoinHandle};

    #[test]
    fn rpc_error_codes_use_a_distinct_high_numeric_namespace() {
        let codes = [
            rpc_error_codes::INVALID_REQUEST,
            rpc_error_codes::ROUTE_NOT_FOUND,
            rpc_error_codes::METHOD_NOT_ALLOWED,
            rpc_error_codes::PAYLOAD_TOO_LARGE,
            rpc_error_codes::UNSUPPORTED_MEDIA_TYPE,
            rpc_error_codes::VALIDATION_FAILED,
            rpc_error_codes::UNSUPPORTED_MODALITY,
            rpc_error_codes::OBSERVATION_TOO_LARGE,
            rpc_error_codes::SESSION_NOT_FOUND,
            rpc_error_codes::PAGE_NOT_FOUND,
            rpc_error_codes::PAGE_MISMATCH,
            rpc_error_codes::PROXY_NOT_FOUND,
            rpc_error_codes::ACTIVE_PAGE_NOT_FOUND,
            rpc_error_codes::CONFLICT,
            rpc_error_codes::BROWSER_BUSY,
            rpc_error_codes::NETWORK_PAUSED,
            rpc_error_codes::ACTION_TARGET_NOT_FOUND,
            rpc_error_codes::REQUEST_CANCELLED,
            rpc_error_codes::RATE_LIMITED,
            rpc_error_codes::ACTION_TIMEOUT,
            rpc_error_codes::OBSERVATION_STALE,
            rpc_error_codes::PRO_REQUIRED,
            rpc_error_codes::UPSTREAM_UNAVAILABLE,
            rpc_error_codes::BROWSER_UNAVAILABLE,
            rpc_error_codes::AGENT_UNHEALTHY,
            rpc_error_codes::TIMEOUT,
            rpc_error_codes::PROXY_CONFIGURATION_FAILED,
            rpc_error_codes::BROWSER_CREATION_FAILED,
            rpc_error_codes::SEMANTIC_EXTRACTION_FAILED,
            rpc_error_codes::INTERNAL_ERROR,
        ];

        assert!(codes.iter().all(|code| *code >= rpc_error_codes::MINIMUM));
        let unique = codes
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), codes.len());
        assert_eq!(
            rpc_error_codes::for_id("UNKNOWN_ERROR"),
            rpc_error_codes::INTERNAL_ERROR
        );
        assert!(rpc_error_codes::is_valid("FUTURE_ERROR", 11_000));
        assert!(!rpc_error_codes::is_valid("SESSION_NOT_FOUND", 10_303));
    }

    #[derive(Debug)]
    struct TestRequest {
        method: String,
        path: String,
        body: String,
    }

    fn start_test_server<F>(
        request_count: usize,
        mut responder: F,
    ) -> (String, JoinHandle<Vec<TestRequest>>)
    where
        F: FnMut(usize, &TestRequest) -> String + Send + 'static,
    {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for index in 0..request_count {
                let (stream, _) = listener.accept().unwrap();
                let (request, mut stream) = read_test_request(stream);
                let response = responder(index, &request);
                stream.write_all(response.as_bytes()).unwrap();
                requests.push(request);
            }
            requests
        });
        (format!("http://{address}/v1"), handle)
    }

    fn read_test_request(stream: TcpStream) -> (TestRequest, TcpStream) {
        let mut reader = BufReader::new(stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line).unwrap();
        let mut fields = request_line.split_whitespace();
        let method = fields.next().unwrap().to_string();
        let path = fields.next().unwrap().to_string();
        let mut content_length = 0_usize;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("Content-Length") {
                    content_length = value.trim().parse().unwrap();
                }
            }
        }
        let mut body = vec![0_u8; content_length];
        reader.read_exact(&mut body).unwrap();
        let stream = reader.into_inner();
        (
            TestRequest {
                method,
                path,
                body: String::from_utf8(body).unwrap(),
            },
            stream,
        )
    }

    fn http_json(status: u16, request_id: &str, body: Value) -> String {
        http_response(
            status,
            "application/json",
            Some(request_id),
            &body.to_string(),
        )
    }

    fn http_response(
        status: u16,
        content_type: &str,
        request_id: Option<&str>,
        body: &str,
    ) -> String {
        let reason = match status {
            200 => "OK",
            404 => "Not Found",
            _ => "Error",
        };
        let request_id = request_id
            .map(|request_id| format!("X-Request-Id: {request_id}\r\n"))
            .unwrap_or_default();
        format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\n{request_id}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn proxy_json() -> Value {
        json!({
            "alias": "office",
            "upstream_host": "proxy.example.com",
            "upstream_port": 8000,
            "username": null,
            "password_set": false
        })
    }

    fn session_json() -> Value {
        json!({
            "id": "machine-a.Session1",
            "name": "Session1",
            "profile": "Default",
            "profile_data_id": null,
            "group": "pgm",
            "proxy_alias": null,
            "adblock_enabled": true,
            "image_blocking_mode": "over_limit",
            "image_size_limit_kb": 100,
            "created_at": 1
        })
    }

    fn profile_json() -> Value {
        json!({
            "id": "builtin-default",
            "name": "Private",
            "proxy_alias": null,
            "adblock_enabled": false,
            "image_blocking_mode": "none",
            "image_size_limit_kb": 100,
            "includes_cookies": false,
            "includes_passwords": false,
            "fingerprint_profile": {
                "schema_version": 1,
                "seed": "12345",
                "platform": "macos",
                "browser_brand": "chromium",
                "browser_version": "151.0.7922.76",
                "user_agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.7922.76 Safari/537.36",
                "locale": "en-US",
                "timezone": "America/Los_Angeles",
                "network_profile": "desktop",
                "hardware_concurrency": 8,
                "device_memory_gib": 8,
                "max_touch_points": 0,
                "screen": {
                    "width": 1920,
                    "height": 1080,
                    "available_height": 985,
                    "device_scale_factor": 2
                },
                "graphics_profile": "apple-m2",
                "storage_quota_bytes": 107374182400_u64,
                "canvas_noise_mode": "deterministic",
                "audio_noise_mode": "deterministic"
            },
            "is_builtin": true,
            "created_at": 0
        })
    }

    fn observation_operation() -> ObservationOperationData {
        ObservationOperationData {
            page: Page {
                id: "page-1".to_string(),
                session_id: "session-1".to_string(),
                url: "https://example.com/guide".to_string(),
            },
            observation: PageObservation {
                id: "observation-1".to_string(),
                mode: ObservationMode::Semantic,
                document_sequence: 1,
                captured_at: "2026-08-19T00:00:00Z".to_string(),
                title: "Example Guide".to_string(),
                truncated: false,
                omitted_node_count: 0,
                clipped_text_count: 0,
                visited_node_count: 12,
                semantic_bytes: 200,
                viewport: ObservationViewport {
                    css_width: 1280,
                    css_height: 720,
                    scroll_x: 0,
                    scroll_y: 0,
                    document_width: 1280,
                    document_height: 1800,
                },
                content: vec![
                    ObservationContent {
                        kind: "heading".to_string(),
                        level: Some(2),
                        context: Some("main".to_string()),
                        text: "Installation".to_string(),
                    },
                    ObservationContent {
                        kind: "paragraph".to_string(),
                        level: None,
                        context: Some("main > section: Installation".to_string()),
                        text: "Install the package with Cargo.".to_string(),
                    },
                    ObservationContent {
                        kind: "paragraph".to_string(),
                        level: None,
                        context: Some("main > section: History".to_string()),
                        text: "Unrelated company history.".to_string(),
                    },
                ],
                elements: vec![ObservationElement {
                    element_ref: "e1".to_string(),
                    role: "link".to_string(),
                    name: "Installation reference".to_string(),
                    states: Vec::new(),
                    value: None,
                    destination: Some("https://example.com/install".to_string()),
                    context: Some("main > navigation: Documentation".to_string()),
                    in_viewport: true,
                    bounds: ObservationBounds {
                        x: 0.0,
                        y: 0.0,
                        width: 100.0,
                        height: 20.0,
                    },
                }],
                screenshot: None,
            },
        }
    }

    #[test]
    fn page_read_is_query_directed_and_markdown_bounded() {
        let data = page_read_data(observation_operation(), Some("install package"), 512, 10);
        assert!(data.markdown.contains("## Installation"));
        assert!(data.markdown.contains("Install the package with Cargo."));
        assert!(data.markdown.contains("Installation reference"));
        assert!(data.markdown.contains("Context: main"));
        assert!(!data.markdown.contains("company history"));
        assert!(data.matched_query);
        assert!(data.markdown.chars().count() <= 512);

        let mut operation = observation_operation();
        operation.observation.content[1].text = "x".repeat(1_000);
        let bounded = page_read_data(operation, None, 512, 10);
        assert!(bounded.markdown.chars().count() <= 512);
        assert!(bounded.markdown.contains('…'));
        assert!(bounded.selected_content_count >= 2);
        assert!(bounded.truncated);

        let mut source_limited = observation_operation();
        source_limited.observation.truncated = true;
        let source_limited = page_read_data(source_limited, None, 32_768, 10);
        assert!(source_limited.source_truncated);
        assert!(!source_limited.truncated);
    }

    #[test]
    fn unqueried_page_read_samples_the_whole_document_and_reports_availability() {
        let mut operation = observation_operation();
        operation.observation.content = (0..30)
            .map(|index| ObservationContent {
                kind: if index % 10 == 0 {
                    "heading".to_string()
                } else {
                    "paragraph".to_string()
                },
                level: (index % 10 == 0).then_some(2),
                context: Some(format!("main > section {}", index / 10 + 1)),
                text: format!("Document section {index}"),
            })
            .collect();

        let data = page_read_data(operation, None, 8_000, 6);

        assert!(data.markdown.contains("## Page outline"));
        assert!(data.markdown.contains("Document section 0"));
        assert!(data.markdown.contains("Document section 29"));
        assert_eq!(data.available_content_count, 30);
        assert_eq!(data.selected_content_count, 6);
        assert!(data.selected_outline_count > 0);
        assert!(data.truncated);
    }

    #[test]
    fn page_read_keeps_rating_pairs_and_ignores_generic_url_path_matches() {
        let mut operation = observation_operation();
        operation.observation.title = "Materia by Julia Holter".to_string();
        operation.observation.content = [
            ("heading", Some(1), "Materia"),
            ("paragraph", None, "2026 / Aug 21 / 7 tracks / 35m"),
            ("text", None, "Metacritic"),
            ("text", None, "87%"),
            ("text", None, "Paste"),
            ("text", None, "8.3/10"),
            ("text", None, "Pitchfork 9.2 Adventurous and precise."),
            ("text", None, "Guardian 5/5 A singular achievement."),
            ("paragraph", None, "Unrelated album history"),
        ]
        .into_iter()
        .map(|(kind, level, text)| ObservationContent {
            kind: kind.to_string(),
            level,
            context: None,
            text: text.to_string(),
        })
        .collect();
        operation.observation.elements = vec![
            ObservationElement {
                element_ref: "e1".to_string(),
                role: "link".to_string(),
                name: "Read Metacritic review".to_string(),
                states: Vec::new(),
                value: None,
                destination: Some("https://reviews.example/materia".to_string()),
                context: None,
                in_viewport: true,
                bounds: ObservationBounds {
                    x: 0.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                },
            },
            ObservationElement {
                element_ref: "e2".to_string(),
                role: "link".to_string(),
                name: "Other record".to_string(),
                states: Vec::new(),
                value: None,
                destination: Some("https://example.com/album/other".to_string()),
                context: None,
                in_viewport: false,
                bounds: ObservationBounds {
                    x: 0.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                },
            },
            ObservationElement {
                element_ref: "e3".to_string(),
                role: "link".to_string(),
                name: "Art Pop".to_string(),
                states: Vec::new(),
                value: None,
                destination: Some("https://example.com/genres/pop/art-pop".to_string()),
                context: None,
                in_viewport: true,
                bounds: ObservationBounds {
                    x: 0.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                },
            },
        ];

        let data = page_read_data(
            operation,
            Some("album details, genres, critic ratings and scores"),
            4_000,
            20,
        );
        let metacritic = data.markdown.find("Metacritic").unwrap();
        let metacritic_score = data.markdown.find("87%").unwrap();
        let paste = data.markdown.find("Paste").unwrap();
        let paste_score = data.markdown.find("8.3/10").unwrap();
        let pitchfork_score = data.markdown.find("Pitchfork 9.2").unwrap();
        let guardian_score = data.markdown.find("Guardian 5/5").unwrap();
        assert!(metacritic < metacritic_score);
        assert!(metacritic_score < paste);
        assert!(paste < paste_score);
        assert!(paste_score < pitchfork_score);
        assert!(pitchfork_score < guardian_score);
        assert!(data.markdown.contains("Art Pop"));
        assert!(data.markdown.contains("Read Metacritic review"));
        assert!(!data.markdown.contains("https://example.com/album/other"));
    }

    #[test]
    fn page_read_url_uses_semantic_navigate_observe() {
        let response_body = json!({
            "status": "ok",
            "request_id": "request-1",
            "data": observation_operation()
        });
        let (base_url, handle) = start_test_server(1, move |_, _| {
            http_json(200, "request-1", response_body.clone())
        });
        let response = RelClient::new(base_url)
            .read_page(&PageReadRequest {
                url: Some("https://example.com/guide".to_string()),
                query: Some("install".to_string()),
                ..PageReadRequest::default()
            })
            .unwrap();
        assert_eq!(response.data.observation_id, "observation-1");
        let requests = handle.join().unwrap();
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].path, "/v1/navigate/observe");
        let body: Value = serde_json::from_str(&requests[0].body).unwrap();
        assert_eq!(body["mode"], "semantic");
    }

    #[test]
    fn retained_observation_can_be_read_without_navigation() {
        let response_body = json!({
            "status": "ok",
            "request_id": "request-1",
            "data": observation_operation()
        });
        let (base_url, handle) = start_test_server(1, move |_, _| {
            http_json(200, "request-1", response_body.clone())
        });
        let response = RelClient::new(base_url)
            .read_observation(
                "observation-1",
                &ObservationReadRequest {
                    query: Some("install".to_string()),
                    ..ObservationReadRequest::default()
                },
            )
            .unwrap();
        assert!(response.data.markdown.contains("Install the package"));
        let requests = handle.join().unwrap();
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].path, "/v1/observations/observation-1");
    }

    #[test]
    fn change_serializes_unchanged_as_missing_and_clear_as_null() {
        let request = ProxyUpdateRequest {
            upstream_host: Some("proxy.example.com".to_string()),
            username: Change::Clear,
            ..ProxyUpdateRequest::default()
        };
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({"upstream_host":"proxy.example.com", "username":null})
        );
    }

    #[test]
    fn session_create_uses_default_profile_when_unchanged_and_accepts_named_profile() {
        assert_eq!(
            serde_json::to_value(SessionCreateRequest::default()).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(SessionCreateRequest {
                group: Some("pgm".to_string()),
                profile: Some("BandwidthSaver".to_string()),
                proxy_alias: Change::Clear,
                ..SessionCreateRequest::default()
            })
            .unwrap(),
            json!({"group":"pgm", "profile":"BandwidthSaver", "proxy_alias":null})
        );
        let mut capture = CaptureRequest::new("https://example.com");
        capture.group = Some("pgm".to_string());
        capture.profile = Some("AdBlock".to_string());
        assert_eq!(
            serde_json::to_value(capture).unwrap(),
            json!({"url":"https://example.com", "profile":"AdBlock", "group":"pgm"})
        );
        assert_eq!(
            serde_json::to_value(SessionCreateRequest {
                image_blocking_mode: Some(ImageBlockingMode::None),
                ..SessionCreateRequest::default()
            })
            .unwrap(),
            json!({"image_blocking_mode":"none"})
        );
    }

    #[test]
    fn action_uses_the_canonical_rpc_shape() {
        let action = Action::ClickLink {
            link: "https://example.com/next".to_string(),
            match_rule: FuzzyLinkMatch::new(0.9),
            mouse_move: None,
            scroll: None,
        };
        assert_eq!(
            serde_json::to_value(action).unwrap(),
            serde_json::json!({
                "action":"click-link",
                "link":"https://example.com/next",
                "match":{"type":"fuzzy-link", "threshold":0.9}
            })
        );
        assert_eq!(
            serde_json::to_value(Action::Click {
                selector: "button.more".to_string(),
                mouse_move: None,
                scroll: None,
            })
            .unwrap(),
            serde_json::json!({"action":"click", "selector":"button.more"})
        );
        assert_eq!(
            serde_json::to_value(Action::Click {
                selector: "button.more".to_string(),
                mouse_move: Some(false),
                scroll: Some(false),
            })
            .unwrap(),
            serde_json::json!({
                "action":"click",
                "selector":"button.more",
                "mouse_move":false,
                "scroll":false
            })
        );
        assert_eq!(
            serde_json::to_value(Action::WaitFor {
                selector: "#loaded".to_string(),
                timeout: None,
            })
            .unwrap(),
            serde_json::json!({"action":"wait-for", "selector":"#loaded"})
        );
        assert_eq!(
            serde_json::to_value(Action::WaitFor {
                selector: "#loaded".to_string(),
                timeout: Some(2.5),
            })
            .unwrap(),
            serde_json::json!({
                "action":"wait-for", "selector":"#loaded", "timeout":2.5
            })
        );
        for (action, expected) in [
            (
                Action::Type {
                    selector: "#search".to_string(),
                    text: "Magickraft".to_string(),
                },
                serde_json::json!({
                    "action":"type", "selector":"#search", "text":"Magickraft"
                }),
            ),
            (
                Action::Clear {
                    selector: "#query".to_string(),
                },
                serde_json::json!({"action":"clear", "selector":"#query"}),
            ),
            (
                Action::Press {
                    selector: "#search".to_string(),
                    key: "Enter".to_string(),
                },
                serde_json::json!({
                    "action":"press", "selector":"#search", "key":"Enter"
                }),
            ),
            (
                Action::Select {
                    selector: "#genre".to_string(),
                    value: "disco".to_string(),
                },
                serde_json::json!({
                    "action":"select", "selector":"#genre", "value":"disco"
                }),
            ),
        ] {
            assert_eq!(serde_json::to_value(action).unwrap(), expected);
        }
    }

    #[test]
    fn observation_actions_serialize_as_one_sequence() {
        assert_eq!(
            serde_json::to_value(NavigateObservationRequest {
                navigation: Some(ObservationNavigation::Back),
                session_id: Some("Session1".to_string()),
                ..NavigateObservationRequest::default()
            })
            .unwrap(),
            json!({"navigation":"back","session_id":"Session1"})
        );
        let mut hover = ObservationAction::new("e2", ObservationActionKind::Hover);
        hover.scroll = Some(false);
        let request = ObservationActionRequest {
            actions: vec![
                ObservationAction::new("e1", ObservationActionKind::Click),
                hover,
                ObservationAction::scroll(0, -600),
                ObservationAction::wait(0.25),
            ],
            mode: Some(ObservationMode::Semantic),
            timeout: None,
            wait: None,
        };
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            json!({
                "actions": [
                    {"ref":"e1","action":"click"},
                    {"ref":"e2","action":"hover","scroll":false},
                    {"action":"scroll","delta_x":0,"delta_y":-600},
                    {"action":"wait","seconds":0.25}
                ],
                "mode":"semantic"
            })
        );
    }

    #[test]
    fn path_segments_are_percent_encoded() {
        assert_eq!(encode_path_segment("page 1/a"), "page%201%2Fa");
    }

    #[test]
    fn every_ordinary_rpc_method_uses_the_v1_route_and_typed_envelope() {
        let (base_url, server) = start_test_server(37, |index, request| {
            let request_id = format!("req_{index}");
            let data = match (request.method.as_str(), request.path.as_str()) {
                ("GET", "/v1/health") => json!({
                    "version":"0.1.7", "pid":123, "browser_proxy_port":17400,
                    "build":{"id":"ba49-deadbeef-a1b2c3d4","configuration":"Debug",
                        "worktree":"ba49","branch":"codex/example","commit":"deadbeef",
                        "dirty":true},
                    "worker":{"state":"idle"}
                }),
                ("GET", "/v1/status") => json!({
                    "overall_status":"ok", "running_count":1, "total_count":1,
                    "build":{"id":"ba49-deadbeef-a1b2c3d4","configuration":"Debug",
                        "worktree":"ba49","branch":"codex/example","commit":"deadbeef",
                        "dirty":true},
                    "checks":[{"id":"agent","name":"Agent","kind":"service","running":true,
                        "status":"running","detail":"ready","pids":[123]}]
                }),
                ("GET", "/v1/notifications") => json!({
                    "notifications":[{
                        "sequence":1,
                        "session_id":"machine-a.Session1",
                        "origin":"https://example.com/",
                        "title":"Example",
                        "body":"Untrusted website content",
                        "notification_id":"notification-1",
                        "persistent":false,
                        "displayed_at":"2026-08-17T20:00:00Z",
                        "trust":"untrusted_website_content"
                    }],
                    "trust":"untrusted_website_content"
                }),
                ("POST", "/v1/navigate")
                | ("POST", "/v1/perform")
                | ("POST", "/v1/capture")
                | ("POST", "/v1/pages")
                | ("POST", "/v1/pages/page_1/actions") => json!({
                    "page":{"id":"page_1","session_id":"machine-a.Session1","url":"https://example.com/"},
                    "capture":{"output_path":"tmp/page.html","bytesize":10,"target_http_status":200}
                }),
                ("POST", "/v1/screenshot") | ("POST", "/v1/pages/page_1/screenshot") => json!({
                    "page":{"id":"page_1","session_id":"machine-a.Session1","url":"https://example.com/"},
                    "screenshot":{"output_path":"/tmp/page.webp","bytesize":11,"format":"webp","mime_type":"image/webp","width":1200,"height":800}
                }),
                ("POST", "/v1/navigate/observe")
                | ("POST", "/v1/observe")
                | ("POST", "/v1/pages/page_1/observe")
                | ("POST", "/v1/observations/11111111-1111-4111-8111-111111111111/actions") => {
                    json!({
                        "page":{"id":"page_1","session_id":"machine-a.Session1","url":"https://example.com/"},
                        "observation":{
                            "id":"22222222-2222-4222-8222-222222222222",
                            "mode":"semantic","document_sequence":4,"captured_at":"2026-08-17T00:00:00Z",
                            "title":"Example","truncated":false,"omitted_node_count":0,
                            "clipped_text_count":0,
                            "visited_node_count":3,"semantic_bytes":20,
                            "viewport":{"css_width":1200,"css_height":800,"scroll_x":0,"scroll_y":0,"document_width":1200,"document_height":800},
                            "content":[{"kind":"heading","level":1,"text":"Example"}],
                            "elements":[{"ref":"e1","role":"button","name":"Continue","states":["enabled"],"in_viewport":true,"bounds":{"x":10.0,"y":20.0,"width":100.0,"height":40.0}}]
                        }
                    })
                }
                ("POST", "/v1/observations/11111111-1111-4111-8111-111111111111/find") => json!({
                    "observation_id":"11111111-1111-4111-8111-111111111111",
                    "query":"continue","role":"button",
                    "matches":[{"type":"element","element":{"ref":"e1","role":"button","name":"Continue","states":["enabled"],"in_viewport":true,"bounds":{"x":10.0,"y":20.0,"width":100.0,"height":40.0}}}],
                    "total_matches":1,"truncated":false
                }),
                ("GET", "/v1/proxies") => json!({"proxies":[proxy_json()]}),
                ("GET", "/v1/proxies/office")
                | ("POST", "/v1/proxies")
                | ("PATCH", "/v1/proxies/office")
                | ("POST", "/v1/proxies/office/rotate-session") => json!({"proxy":proxy_json()}),
                ("DELETE", "/v1/proxies/office") => {
                    json!({"deleted_alias":"office"})
                }
                ("POST", "/v1/proxy-transfers/export") => {
                    json!({"filename":"office.relproxy","contents_base64":"U1FMaXRlIGZvcm1hdCAzAA=="})
                }
                ("POST", "/v1/proxy-transfers/import") => json!({"proxy":proxy_json()}),
                ("DELETE", "/v1/sessions/machine-a.Session1") => {
                    json!({"deleted_id":"machine-a.Session1"})
                }
                ("POST", "/v1/sessions/machine-a.Session1/pause") => {
                    json!({"session_id":"machine-a.Session1", "network_paused":true})
                }
                ("POST", "/v1/sessions/machine-a.Session1/play") => {
                    json!({"session_id":"machine-a.Session1", "network_paused":false})
                }
                ("POST", "/v1/sessions/close") => {
                    json!({"group":"pgm", "deleted_ids":["machine-a.Session1"]})
                }
                ("GET", "/v1/sessions") => json!({"sessions":[session_json()]}),
                ("GET", "/v1/sessions/machine-a.Session1")
                | ("POST", "/v1/sessions")
                | ("PATCH", "/v1/sessions/machine-a.Session1") => json!({"session":session_json()}),
                ("GET", "/v1/profiles") => json!({"profiles":[profile_json()]}),
                ("POST", "/v1/profiles") | ("PATCH", "/v1/profiles/custom-profile-id") => {
                    json!({"profile":profile_json()})
                }
                ("DELETE", "/v1/profiles/custom-profile-id") => {
                    json!({"deleted_id":"custom-profile-id"})
                }
                ("POST", "/v1/profile-transfers/export") => {
                    json!({"filename":"Research.relprofile","contents_base64":"U1FMaXRlIGZvcm1hdCAzAA=="})
                }
                ("POST", "/v1/profile-transfers/import") => {
                    json!({"profile":profile_json()})
                }
                route => panic!("unexpected route {route:?}"),
            };
            http_json(
                200,
                &request_id,
                json!({"status":"ok", "request_id":request_id, "data":data}),
            )
        });
        let client = RelClient::new(base_url);

        let health = client.health().unwrap();
        assert_eq!(health.data.build.as_ref().unwrap().worktree, "ba49");
        let status = client.status().unwrap();
        assert_eq!(status.data.build, health.data.build);
        let notifications = client.list_notifications().unwrap();
        assert_eq!(notifications.data.notifications[0].sequence, 1);
        assert_eq!(notifications.data.trust, "untrusted_website_content");
        client
            .navigate(&NavigateRequest::new("example.com"))
            .unwrap();
        let mut perform = PerformRequest::new(vec![
            Action::ClickLink {
                link: "https://example.com/more".to_string(),
                match_rule: FuzzyLinkMatch::new(1.0),
                mouse_move: None,
                scroll: None,
            },
            Action::Wait { seconds: 0.0 },
        ]);
        perform.session_id = Some("machine-a.Session1".to_string());
        client.perform(&perform).unwrap();
        client
            .capture_current_page(&PageCaptureRequest {
                session_id: Some("machine-a.Session1".to_string()),
                ..PageCaptureRequest::default()
            })
            .unwrap();
        client
            .screenshot_current_page(&ScreenshotRequest {
                session_id: Some("machine-a.Session1".to_string()),
                format: Some(ScreenshotFormat::Webp),
                quality: Some(80),
                full_page: true,
                ..ScreenshotRequest::default()
            })
            .unwrap();
        client
            .attach_page(&PageAttachRequest::new("example.com"))
            .unwrap();
        client
            .perform_page_action(
                "page_1",
                &PageActionRequest {
                    action: Action::Wait { seconds: 0.0 },
                    output: None,
                    timeout: None,
                    wait: None,
                },
            )
            .unwrap();
        client
            .take_page_screenshot(
                "page_1",
                &PageScreenshotRequest {
                    format: Some(ScreenshotFormat::Png),
                    ..PageScreenshotRequest::default()
                },
            )
            .unwrap();
        client
            .observe_current_page(&ObservationRequest {
                session_id: Some("machine-a.Session1".to_string()),
                mode: Some(ObservationMode::Semantic),
                timeout: None,
                wait: None,
            })
            .unwrap();
        client
            .navigate_and_observe(&NavigateObservationRequest::new("example.com"))
            .unwrap();
        client
            .observe_page("page_1", &PageObservationRequest::default())
            .unwrap();
        client
            .perform_observation_action(
                "11111111-1111-4111-8111-111111111111",
                &ObservationActionRequest::new("e1", ObservationActionKind::Click),
            )
            .unwrap();
        client
            .find_in_observation(
                "11111111-1111-4111-8111-111111111111",
                &ObservationFindRequest {
                    query: Some("continue".to_string()),
                    role: Some("button".to_string()),
                    limit: Some(5),
                },
            )
            .unwrap();
        client.list_proxies().unwrap();
        client.get_proxy("office").unwrap();
        client
            .create_proxy(&ProxyCreateRequest {
                alias: "office".to_string(),
                upstream_host: "proxy.example.com".to_string(),
                upstream_port: 8000,
                ..ProxyCreateRequest::default()
            })
            .unwrap();
        client
            .update_proxy(
                "office",
                &ProxyUpdateRequest {
                    username: Change::Clear,
                    ..ProxyUpdateRequest::default()
                },
            )
            .unwrap();
        client.delete_proxy("office").unwrap();
        client.rotate_proxy_session("office").unwrap();
        client
            .export_proxy_transfer(&ProxyTransferExportRequest {
                alias: "office".to_string(),
                include_credentials: false,
                passphrase: None,
            })
            .unwrap();
        client
            .import_proxy_transfer(&ProxyTransferImportRequest::from_bytes(
                b"SQLite format 3\0",
                Some("backup".to_string()),
                None,
            ))
            .unwrap();
        let sessions = client.list_sessions().unwrap();
        assert_eq!(sessions.data.sessions[0].id, "machine-a.Session1");
        assert_eq!(sessions.data.sessions[0].group.as_deref(), Some("pgm"));
        client.get_session("machine-a.Session1").unwrap();
        client
            .create_session(&SessionCreateRequest::default())
            .unwrap();
        let profiles = client.list_profiles().unwrap();
        assert_eq!(
            profiles.data.profiles[0]
                .fingerprint_profile
                .as_ref()
                .map(|profile| profile.seed.as_str()),
            Some("12345")
        );
        client
            .create_profile(&ProfileCreateRequest {
                name: "Research".to_string(),
                proxy_alias: None,
                adblock_enabled: Some(true),
                image_blocking_mode: Some(ImageBlockingMode::OverLimit),
                image_size_limit_kb: Some(10),
                includes_cookies: Some(false),
                includes_passwords: Some(false),
                fingerprint_profile: Change::Unchanged,
            })
            .unwrap();
        client
            .update_profile_data(
                "custom-profile-id",
                &ProfileDataUpdateRequest {
                    includes_cookies: true,
                    includes_passwords: true,
                },
            )
            .unwrap();
        client.delete_profile("custom-profile-id").unwrap();
        client
            .export_profile_transfer(&ProfileTransferExportRequest {
                name: "Research".to_string(),
                include_cookies: false,
                include_passwords: false,
                include_proxy_credentials: false,
                passphrase: None,
            })
            .unwrap();
        client
            .import_profile_transfer(&ProfileTransferImportRequest::from_bytes(
                b"SQLite format 3\0",
                Some("Research Copy".to_string()),
                None,
            ))
            .unwrap();
        client
            .update_session(
                "machine-a.Session1",
                &SessionUpdateRequest {
                    proxy_alias: Change::Clear,
                    ..SessionUpdateRequest::default()
                },
            )
            .unwrap();
        let paused = client.pause_session("machine-a.Session1").unwrap();
        assert!(paused.data.network_paused);
        let playing = client.play_session("machine-a.Session1").unwrap();
        assert!(!playing.data.network_paused);
        let deleted = client.delete_session("machine-a.Session1").unwrap();
        assert_eq!(deleted.data.deleted_id, "machine-a.Session1");
        let closed = client.close_session_group("pgm").unwrap();
        assert_eq!(closed.data.deleted_ids, ["machine-a.Session1"]);

        let requests = server.join().unwrap();
        let routes = requests
            .iter()
            .map(|request| (request.method.as_str(), request.path.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            routes,
            vec![
                ("GET", "/v1/health"),
                ("GET", "/v1/status"),
                ("GET", "/v1/notifications"),
                ("POST", "/v1/navigate"),
                ("POST", "/v1/perform"),
                ("POST", "/v1/capture"),
                ("POST", "/v1/screenshot"),
                ("POST", "/v1/pages"),
                ("POST", "/v1/pages/page_1/actions"),
                ("POST", "/v1/pages/page_1/screenshot"),
                ("POST", "/v1/observe"),
                ("POST", "/v1/navigate/observe"),
                ("POST", "/v1/pages/page_1/observe"),
                (
                    "POST",
                    "/v1/observations/11111111-1111-4111-8111-111111111111/actions"
                ),
                (
                    "POST",
                    "/v1/observations/11111111-1111-4111-8111-111111111111/find"
                ),
                ("GET", "/v1/proxies"),
                ("GET", "/v1/proxies/office"),
                ("POST", "/v1/proxies"),
                ("PATCH", "/v1/proxies/office"),
                ("DELETE", "/v1/proxies/office"),
                ("POST", "/v1/proxies/office/rotate-session"),
                ("POST", "/v1/proxy-transfers/export"),
                ("POST", "/v1/proxy-transfers/import"),
                ("GET", "/v1/sessions"),
                ("GET", "/v1/sessions/machine-a.Session1"),
                ("POST", "/v1/sessions"),
                ("GET", "/v1/profiles"),
                ("POST", "/v1/profiles"),
                ("PATCH", "/v1/profiles/custom-profile-id"),
                ("DELETE", "/v1/profiles/custom-profile-id"),
                ("POST", "/v1/profile-transfers/export"),
                ("POST", "/v1/profile-transfers/import"),
                ("PATCH", "/v1/sessions/machine-a.Session1"),
                ("POST", "/v1/sessions/machine-a.Session1/pause"),
                ("POST", "/v1/sessions/machine-a.Session1/play"),
                ("DELETE", "/v1/sessions/machine-a.Session1"),
                ("POST", "/v1/sessions/close"),
            ]
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[36].body).unwrap(),
            json!({"group":"pgm"})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[3].body).unwrap(),
            json!({"url":"example.com"})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[4].body).unwrap(),
            json!({"session_id":"machine-a.Session1","actions":[
                {"action":"click-link","link":"https://example.com/more","match":{"type":"fuzzy-link","threshold":1.0}},
                {"action":"wait","seconds":0.0}
            ]})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[5].body).unwrap(),
            json!({"session_id":"machine-a.Session1"})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[6].body).unwrap(),
            json!({
                "session_id":"machine-a.Session1",
                "format":"webp",
                "quality":80,
                "full_page":true
            })
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[11].body).unwrap(),
            json!({"url":"example.com"})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[13].body).unwrap(),
            json!({"actions":[{"ref":"e1","action":"click"}]})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[14].body).unwrap(),
            json!({"query":"continue","role":"button","limit":5})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[17].body).unwrap(),
            json!({"alias":"office","upstream_host":"proxy.example.com","upstream_port":8000})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[18].body).unwrap(),
            json!({"username":null})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[21].body).unwrap(),
            json!({"alias":"office","include_credentials":false})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[22].body).unwrap(),
            json!({"contents_base64":"U1FMaXRlIGZvcm1hdCAzAA==","alias":"backup"})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[27].body).unwrap(),
            json!({
                "name":"Research",
                "adblock_enabled":true,
                "image_blocking_mode":"over_limit",
                "image_size_limit_kb":10,
                "includes_cookies":false,
                "includes_passwords":false
            })
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[28].body).unwrap(),
            json!({"includes_cookies":true,"includes_passwords":true})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[30].body).unwrap(),
            json!({
                "name":"Research",
                "include_cookies":false,
                "include_passwords":false,
                "include_proxy_credentials":false
            })
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[31].body).unwrap(),
            json!({
                "contents_base64":"U1FMaXRlIGZvcm1hdCAzAA==",
                "name":"Research Copy"
            })
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[32].body).unwrap(),
            json!({"proxy_alias":null})
        );
    }

    #[test]
    fn structured_rpc_errors_are_preserved_and_protocol_is_validated() {
        let (base_url, server) = start_test_server(1, |_index, _request| {
            http_json(
                404,
                "req_missing",
                json!({
                    "status":"error",
                    "request_id":"req_missing",
                    "error":{
                        "id":"SESSION_NOT_FOUND",
                        "code":10100,
                        "message":"Session machine-a.Session999 was not found.",
                        "retryable":false,
                        "details":{"id":"machine-a.Session999"}
                    }
                }),
            )
        });
        let error = RelClient::new(base_url)
            .get_session("machine-a.Session999")
            .unwrap_err();
        let failure = error.rpc_failure().unwrap();
        assert_eq!(failure.request_id, "req_missing");
        assert_eq!(failure.error.id, "SESSION_NOT_FOUND");
        assert_eq!(failure.error.code, rpc_error_codes::SESSION_NOT_FOUND);
        assert_eq!(
            failure.error.details.as_ref().unwrap()["id"],
            "machine-a.Session999"
        );
        server.join().unwrap();

        let (base_url, server) = start_test_server(1, |_index, _request| {
            http_json(
                404,
                "req_legacy",
                json!({
                    "status":"error",
                    "request_id":"req_legacy",
                    "error":{
                        "id":"SESSION_NOT_FOUND",
                        "http_code":404,
                        "message":"Session machine-a.Session999 was not found.",
                        "retryable":false
                    }
                }),
            )
        });
        assert!(matches!(
            RelClient::new(base_url).get_session("machine-a.Session999"),
            Err(ClientError::Protocol(message)) if message.contains("unknown field `http_code`")
        ));
        server.join().unwrap();

        let (base_url, server) = start_test_server(1, |_index, _request| {
            http_json(
                404,
                "req_http_like_code",
                json!({
                    "status":"error",
                    "request_id":"req_http_like_code",
                    "error":{
                        "id":"SESSION_NOT_FOUND",
                        "code":404,
                        "message":"Session machine-a.Session999 was not found.",
                        "retryable":false
                    }
                }),
            )
        });
        assert!(matches!(
            RelClient::new(base_url).get_session("machine-a.Session999"),
            Err(ClientError::Protocol(message)) if message.contains("incomplete error object")
        ));
        server.join().unwrap();

        let (base_url, server) = start_test_server(1, |_index, _request| {
            http_json(
                404,
                "req_mismatched_code",
                json!({
                    "status":"error",
                    "request_id":"req_mismatched_code",
                    "error":{
                        "id":"SESSION_NOT_FOUND",
                        "code":10303,
                        "message":"Session machine-a.Session999 was not found.",
                        "retryable":false
                    }
                }),
            )
        });
        assert!(matches!(
            RelClient::new(base_url).get_session("machine-a.Session999"),
            Err(ClientError::Protocol(message)) if message.contains("incomplete error object")
        ));
        server.join().unwrap();

        let (base_url, server) = start_test_server(1, |_index, _request| {
            http_response(
                200,
                "text/plain",
                Some("req_wrong_type"),
                r#"{"status":"ok","request_id":"req_wrong_type","data":{}}"#,
            )
        });
        assert!(matches!(
            RelClient::new(base_url).list_proxies(),
            Err(ClientError::Protocol(message)) if message.contains("Content-Type")
        ));
        server.join().unwrap();

        let (base_url, server) = start_test_server(1, |_index, _request| {
            http_json(
                200,
                "req_header",
                json!({"status":"ok","request_id":"req_body","data":{"proxies":[]}}),
            )
        });
        assert!(matches!(
            RelClient::new(base_url).list_proxies(),
            Err(ClientError::Protocol(message)) if message.contains("request ID mismatch")
        ));
        server.join().unwrap();
    }

    #[test]
    fn capture_stream_is_validated_and_exposes_the_terminal_exit_code() {
        let (base_url, server) = start_test_server(1, |_index, _request| {
            let body = [
                json!({"status":"ok","request_id":"req_capture","event":"capture.started","data":{"url":"https://example.com/"}}).to_string(),
                json!({"status":"error","request_id":"req_capture","event":"capture.failed","error":{"id":"TIMEOUT","code":10303,"message":"Timed out.","retryable":true},"data":{}}).to_string(),
                json!({"status":"ok","request_id":"req_capture","event":"capture.finished","data":{"exit_code":1}}).to_string(),
            ]
            .join("\n")
                + "\n";
            http_response(200, "application/x-ndjson", Some("req_capture"), &body)
        });
        let client = RelClient::new(base_url);
        let mut stream = client.capture(&CaptureRequest::new("example.com")).unwrap();
        let events = stream
            .by_ref()
            .collect::<Result<Vec<_>, ClientError>>()
            .unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[1].error.as_ref().unwrap().id, "TIMEOUT");
        assert!(stream.is_finished());
        assert_eq!(stream.exit_code(), Some(1));
        let requests = server.join().unwrap();
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].path, "/v1/captures");
        assert_eq!(
            serde_json::from_str::<Value>(&requests[0].body).unwrap(),
            json!({"url":"example.com"})
        );
    }
}
