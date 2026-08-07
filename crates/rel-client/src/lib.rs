//! Typed Rust client for Rel RPC v1.
//!
//! This module contains no desktop-app lifecycle or local-file behavior. It can
//! therefore be used by other Rust programs without adopting the bundled CLI's
//! macOS-specific conveniences.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, BufRead, BufReader, Lines, Read};
use std::time::Duration;

const DEFAULT_AGENT_PORT: u16 = 17_319;
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub struct RelClient {
    base_url: String,
    request_timeout: Duration,
}

impl RelClient {
    /// Connect to the standard loopback Rel RPC v1 endpoint.
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

    pub fn session_defaults(&self) -> Result<RpcResponse<SessionDefaultsData>, ClientError> {
        self.request::<SessionDefaultsData, Value>("GET", "/session-defaults", None)
    }

    pub fn update_session_defaults(
        &self,
        request: &SessionDefaultsUpdateRequest,
    ) -> Result<RpcResponse<SessionDefaultsData>, ClientError> {
        self.request("PATCH", "/session-defaults", Some(request))
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

    pub fn delete_session(&self, id: &str) -> Result<RpcResponse<DeletedData>, ClientError> {
        self.request::<DeletedData, Value>(
            "DELETE",
            &format!("/sessions/{}", encode_path_segment(id)),
            None,
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
                "Rel RPC base URL cannot be empty".to_string(),
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
            "Rel RPC success response has status {:?}",
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
                "Rel RPC returned HTTP {status} with an invalid error envelope: {error}"
            ))
        }
    };
    if failure.status != "error" {
        return ClientError::Protocol(format!(
            "Rel RPC error response has status {:?}",
            failure.status
        ));
    }
    if let Err(error) = validate_request_id(header_request_id.as_deref(), &failure.request_id) {
        return error;
    }
    if failure.error.id.trim().is_empty() || failure.error.message.trim().is_empty() {
        return ClientError::Protocol(
            "Rel RPC error response has an incomplete error object".to_string(),
        );
    }
    if failure
        .error
        .details
        .as_ref()
        .is_some_and(|details| !details.is_object())
    {
        return ClientError::Protocol("Rel RPC error details must be a JSON object".to_string());
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
            "Rel RPC returned unsupported Content-Type {content_type:?}"
        )))
    }
}

fn validate_request_id(header: Option<&str>, body: &str) -> Result<(), ClientError> {
    if body.trim().is_empty() {
        return Err(ClientError::Protocol(
            "Rel RPC response is missing request_id".to_string(),
        ));
    }
    match header {
        Some(header) if header == body => Ok(()),
        Some(header) => Err(ClientError::Protocol(format!(
            "Rel RPC request ID mismatch: header {header:?}, body {body:?}"
        ))),
        None => Err(ClientError::Protocol(
            "Rel RPC response is missing X-Request-Id".to_string(),
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
                "Rel capture returned unsupported Content-Type {content_type:?}"
            )));
        }
        let request_id = response
            .header("X-Request-Id")
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                ClientError::Protocol("Rel capture response is missing X-Request-Id".to_string())
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
                "Rel capture request ID mismatch: header {:?}, event {:?}",
                self.request_id, event.request_id
            ))));
        }
        if event.event.trim().is_empty() {
            return Some(Err(ClientError::Protocol(
                "Rel capture event is missing its event name".to_string(),
            )));
        }
        match event.status.as_str() {
            "ok" if event.data.is_some() && event.error.is_none() => {}
            "error"
                if event.error.as_ref().is_some_and(|error| {
                    !error.id.trim().is_empty() && !error.message.trim().is_empty()
                }) => {}
            _ => {
                return Some(Err(ClientError::Protocol(format!(
                    "Rel capture event {:?} has an invalid envelope",
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
                    "Rel capture.finished event is missing a valid exit_code".to_string(),
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
    },
    WaitFor {
        selector: String,
    },
    ClickLink {
        link: String,
        #[serde(rename = "match")]
        match_rule: FuzzyLinkMatch,
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

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct ProxyCreateRequest {
    pub alias: String,
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
    pub alias: String,
    pub upstream_host: String,
    pub upstream_port: u16,
    pub username: Option<String>,
    pub password_set: bool,
    #[serde(rename = "oxylabs", skip_serializing_if = "Option::is_none")]
    pub oxylabs: Option<OxylabsProxy>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct SessionCreateRequest {
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

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct SessionDefaultsUpdateRequest {
    #[serde(skip_serializing_if = "Change::is_unchanged")]
    pub proxy_alias: Change<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adblock_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_blocking_mode: Option<ImageBlockingMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_size_limit_kb: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_open_tabs: Option<i64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ImageBlockingMode {
    All,
    OverLimit,
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
pub struct SessionDefaultsData {
    pub session_defaults: SessionDefaults,
    #[serde(default)]
    pub closed_session_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SessionDefaults {
    pub proxy_alias: Option<String>,
    pub adblock_enabled: bool,
    pub image_blocking_mode: ImageBlockingMode,
    pub image_size_limit_kb: i64,
    pub max_open_tabs: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Session {
    pub id: String,
    pub name: String,
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
pub struct ProxyDeletedData {
    pub deleted_alias: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Health {
    pub version: String,
    pub pid: u32,
    pub browser_proxy_port: u16,
    pub worker: Worker,
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
            "proxy_alias": null,
            "adblock_enabled": true,
            "image_blocking_mode": "over_limit",
            "image_size_limit_kb": 100,
            "created_at": 1
        })
    }

    fn session_defaults_json() -> Value {
        json!({
            "proxy_alias": "office",
            "adblock_enabled": false,
            "image_blocking_mode": "all",
            "image_size_limit_kb": 250,
            "max_open_tabs": 8
        })
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
    fn session_create_uses_defaults_when_unchanged_and_can_force_direct_networking() {
        assert_eq!(
            serde_json::to_value(SessionCreateRequest::default()).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(SessionCreateRequest {
                proxy_alias: Change::Clear,
                ..SessionCreateRequest::default()
            })
            .unwrap(),
            json!({"proxy_alias":null})
        );
    }

    #[test]
    fn action_uses_the_canonical_rpc_shape() {
        let action = Action::ClickLink {
            link: "https://example.com/next".to_string(),
            match_rule: FuzzyLinkMatch::new(0.9),
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
            serde_json::to_value(Action::WaitFor {
                selector: "#loaded".to_string(),
            })
            .unwrap(),
            serde_json::json!({
                "action":"wait-for",
                "selector":"#loaded"
            })
        );
    }

    #[test]
    fn path_segments_are_percent_encoded() {
        assert_eq!(encode_path_segment("page 1/a"), "page%201%2Fa");
    }

    #[test]
    fn every_ordinary_rpc_method_uses_the_v1_route_and_typed_envelope() {
        let (base_url, server) = start_test_server(20, |index, request| {
            let request_id = format!("req_{index}");
            let data = match (request.method.as_str(), request.path.as_str()) {
                ("GET", "/v1/health") => json!({
                    "version":"0.1.7", "pid":123, "browser_proxy_port":17400,
                    "worker":{"state":"idle"}
                }),
                ("GET", "/v1/status") => json!({
                    "overall_status":"ok", "running_count":1, "total_count":1,
                    "checks":[{"id":"agent","name":"Agent","kind":"service","running":true,
                        "status":"running","detail":"ready","pids":[123]}]
                }),
                ("POST", "/v1/navigate")
                | ("POST", "/v1/perform")
                | ("POST", "/v1/capture")
                | ("POST", "/v1/pages")
                | ("POST", "/v1/pages/page_1/actions") => json!({
                    "page":{"id":"page_1","session_id":"machine-a.Session1","url":"https://example.com/"},
                    "capture":{"output_path":"tmp/page.html","bytesize":10,"target_http_status":200}
                }),
                ("GET", "/v1/proxies") => json!({"proxies":[proxy_json()]}),
                ("GET", "/v1/proxies/office")
                | ("POST", "/v1/proxies")
                | ("PATCH", "/v1/proxies/office")
                | ("POST", "/v1/proxies/office/rotate-session") => json!({"proxy":proxy_json()}),
                ("DELETE", "/v1/proxies/office") => {
                    json!({"deleted_alias":"office"})
                }
                ("DELETE", "/v1/sessions/machine-a.Session1") => {
                    json!({"deleted_id":"machine-a.Session1"})
                }
                ("GET", "/v1/sessions") => json!({"sessions":[session_json()]}),
                ("GET", "/v1/sessions/machine-a.Session1")
                | ("POST", "/v1/sessions")
                | ("PATCH", "/v1/sessions/machine-a.Session1") => json!({"session":session_json()}),
                ("GET", "/v1/session-defaults") | ("PATCH", "/v1/session-defaults") => {
                    json!({"session_defaults":session_defaults_json()})
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

        client.health().unwrap();
        client.status().unwrap();
        client
            .navigate(&NavigateRequest::new("example.com"))
            .unwrap();
        let mut perform = PerformRequest::new(vec![
            Action::Click {
                selector: "button.more".to_string(),
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
        let sessions = client.list_sessions().unwrap();
        assert_eq!(sessions.data.sessions[0].id, "machine-a.Session1");
        client.get_session("machine-a.Session1").unwrap();
        client
            .create_session(&SessionCreateRequest::default())
            .unwrap();
        client.session_defaults().unwrap();
        client
            .update_session_defaults(&SessionDefaultsUpdateRequest {
                proxy_alias: Change::Clear,
                ..SessionDefaultsUpdateRequest::default()
            })
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
        let deleted = client.delete_session("machine-a.Session1").unwrap();
        assert_eq!(deleted.data.deleted_id, "machine-a.Session1");

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
                ("POST", "/v1/navigate"),
                ("POST", "/v1/perform"),
                ("POST", "/v1/capture"),
                ("POST", "/v1/pages"),
                ("POST", "/v1/pages/page_1/actions"),
                ("GET", "/v1/proxies"),
                ("GET", "/v1/proxies/office"),
                ("POST", "/v1/proxies"),
                ("PATCH", "/v1/proxies/office"),
                ("DELETE", "/v1/proxies/office"),
                ("POST", "/v1/proxies/office/rotate-session"),
                ("GET", "/v1/sessions"),
                ("GET", "/v1/sessions/machine-a.Session1"),
                ("POST", "/v1/sessions"),
                ("GET", "/v1/session-defaults"),
                ("PATCH", "/v1/session-defaults"),
                ("PATCH", "/v1/sessions/machine-a.Session1"),
                ("DELETE", "/v1/sessions/machine-a.Session1"),
            ]
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[2].body).unwrap(),
            json!({"url":"example.com"})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[3].body).unwrap(),
            json!({"session_id":"machine-a.Session1","actions":[
                {"action":"click","selector":"button.more"},
                {"action":"wait","seconds":0.0}
            ]})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[4].body).unwrap(),
            json!({"session_id":"machine-a.Session1"})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[9].body).unwrap(),
            json!({"alias":"office","upstream_host":"proxy.example.com","upstream_port":8000})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[10].body).unwrap(),
            json!({"username":null})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[17].body).unwrap(),
            json!({"proxy_alias":null})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[18].body).unwrap(),
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
                json!({"status":"error","request_id":"req_capture","event":"capture.failed","error":{"id":"TIMEOUT","message":"Timed out.","retryable":true},"data":{}}).to_string(),
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
