use rel_client::{
    self as client, Action, CaptureRequest, PageActionRequest, PageAttachRequest, RelClient,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

const CURRENT_PROTOCOL_VERSION: &str = "2026-07-28";
const LATEST_LEGACY_PROTOCOL_VERSION: &str = "2025-11-25";
const LEGACY_PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const MAX_MCP_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const TOOL_LIST_TTL_MS: u64 = 3_600_000;

pub(crate) fn serve_stdio(client: RelClient, server_version: &str) -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    serve(BufReader::new(stdin), stdout, client, server_version)
}

fn serve<R: BufRead, W: Write + Send + 'static>(
    mut reader: R,
    writer: W,
    client: RelClient,
    server_version: &str,
) -> Result<(), String> {
    let mut state = ServerState::new(server_version);
    let writer = Arc::new(Mutex::new(writer));
    let active_requests = Arc::new(Mutex::new(HashMap::new()));
    let mut workers = Vec::new();
    let mut bytes = Vec::new();
    loop {
        reap_workers(&mut workers);
        bytes.clear();
        match read_message(&mut reader, &mut bytes)
            .map_err(|error| format!("Could not read MCP stdin: {error}"))?
        {
            MessageRead::Eof => {
                cancel_workers(workers);
                return Ok(());
            }
            MessageRead::TooLarge => {
                write_message(
                    &writer,
                    &rpc_error(
                        Value::Null,
                        -32600,
                        "MCP message exceeds the 16 MiB limit",
                        None,
                    ),
                )?;
                continue;
            }
            MessageRead::Message => {}
        }
        while matches!(bytes.last(), Some(b'\n' | b'\r')) {
            bytes.pop();
        }
        if bytes.is_empty() {
            continue;
        }
        let message = match serde_json::from_slice::<Value>(&bytes) {
            Ok(message) => message,
            Err(error) => {
                write_message(
                    &writer,
                    &rpc_error(
                        Value::Null,
                        -32700,
                        "Parse error",
                        Some(json!({"detail": error.to_string()})),
                    ),
                )?;
                continue;
            }
        };
        match handle_message(&mut state, &active_requests, message) {
            MessageAction::Notification => {}
            MessageAction::Response(response) => write_message(&writer, &response)?,
            MessageAction::ToolCall { id, params, era } => {
                let key = request_id_key(&id).expect("validated request ID has a key");
                let cancellation = Arc::new(AtomicBool::new(false));
                {
                    let mut active = active_requests
                        .lock()
                        .map_err(|_| "MCP active request registry is unavailable".to_string())?;
                    if active.contains_key(&key) {
                        drop(active);
                        write_message(
                            &writer,
                            &rpc_error(id, -32600, "Request ID is already active", None),
                        )?;
                        continue;
                    }
                    active.insert(key.clone(), cancellation.clone());
                }
                let worker_writer = writer.clone();
                let worker_active_requests = active_requests.clone();
                let worker_client = client.clone();
                let worker_cancellation = cancellation.clone();
                let worker_server_version = state.server_version.clone();
                let handle = thread::spawn(move || {
                    let result = match handle_tool_call(
                        &worker_client,
                        &params,
                        era,
                        &worker_server_version,
                    ) {
                        Ok(result) => rpc_result(id.clone(), result),
                        Err(error) => with_rpc_id(id, error),
                    };
                    if !cancellation.load(Ordering::Acquire) {
                        if let Err(error) = write_message(&worker_writer, &result) {
                            eprintln!("rel MCP response failed: {error}");
                        }
                    }
                    if let Ok(mut active) = worker_active_requests.lock() {
                        active.remove(&key);
                    }
                });
                workers.push(ToolWorker {
                    handle,
                    cancellation: worker_cancellation,
                });
            }
        }
    }
}

struct ToolWorker {
    handle: JoinHandle<()>,
    cancellation: Arc<AtomicBool>,
}

fn reap_workers(workers: &mut Vec<ToolWorker>) {
    let mut index = 0;
    while index < workers.len() {
        if workers[index].handle.is_finished() {
            let worker = workers.swap_remove(index);
            if worker.handle.join().is_err() {
                eprintln!("rel MCP tool worker panicked");
            }
        } else {
            index += 1;
        }
    }
}

fn cancel_workers(workers: Vec<ToolWorker>) {
    for worker in workers {
        worker.cancellation.store(true, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MessageRead {
    Eof,
    Message,
    TooLarge,
}

fn read_message(reader: &mut impl BufRead, bytes: &mut Vec<u8>) -> io::Result<MessageRead> {
    let mut saw_bytes = false;
    let mut too_large = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(if !saw_bytes {
                MessageRead::Eof
            } else if too_large {
                MessageRead::TooLarge
            } else {
                MessageRead::Message
            });
        }
        saw_bytes = true;
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        if !too_large {
            let remaining = MAX_MCP_MESSAGE_BYTES.saturating_add(1) - bytes.len();
            let copied = consumed.min(remaining);
            bytes.extend_from_slice(&available[..copied]);
            too_large = bytes.len() > MAX_MCP_MESSAGE_BYTES || copied < consumed;
        }
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(if too_large {
                MessageRead::TooLarge
            } else {
                MessageRead::Message
            });
        }
    }
}

fn write_message<W: Write>(writer: &Arc<Mutex<W>>, message: &Value) -> Result<(), String> {
    let mut writer = writer
        .lock()
        .map_err(|_| "MCP stdout lock is unavailable".to_string())?;
    serde_json::to_writer(&mut *writer, message)
        .map_err(|error| format!("Could not encode MCP response: {error}"))?;
    writer
        .write_all(b"\n")
        .and_then(|()| writer.flush())
        .map_err(|error| format!("Could not write MCP stdout: {error}"))
}

struct ServerState {
    legacy_protocol_version: Option<String>,
    server_version: String,
}

impl ServerState {
    fn new(server_version: &str) -> Self {
        Self {
            legacy_protocol_version: None,
            server_version: server_version.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProtocolEra {
    Legacy,
    Modern,
}

type ActiveRequests = Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>;

enum MessageAction {
    Notification,
    Response(Value),
    ToolCall {
        id: Value,
        params: Value,
        era: ProtocolEra,
    },
}

fn handle_message(
    state: &mut ServerState,
    active_requests: &ActiveRequests,
    message: Value,
) -> MessageAction {
    let Some(object) = message.as_object() else {
        return MessageAction::Response(rpc_error(Value::Null, -32600, "Invalid Request", None));
    };
    let id = object.get("id").cloned();
    let response_id = id.clone().unwrap_or(Value::Null);
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || id.as_ref().is_some_and(|id| !valid_request_id(id))
    {
        return MessageAction::Response(rpc_error(response_id, -32600, "Invalid Request", None));
    }
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return MessageAction::Response(rpc_error(response_id, -32600, "Invalid Request", None));
    };
    let params = match object.get("params") {
        None => json!({}),
        Some(Value::Object(params)) => Value::Object(params.clone()),
        Some(_) => {
            return MessageAction::Response(rpc_error(
                response_id,
                -32602,
                "params must be an object",
                None,
            ))
        }
    };
    if id.is_none() {
        handle_notification(method, &params, active_requests);
        return MessageAction::Notification;
    }

    if method == "initialize" {
        return MessageAction::Response(handle_legacy_initialize(state, response_id, &params));
    }

    let era = match request_era(state, &params) {
        Ok(era) => era,
        Err(error) => return MessageAction::Response(with_rpc_id(response_id, error)),
    };

    let result = match method {
        "server/discover" if era == ProtocolEra::Modern => {
            modern_discover_result(&state.server_version)
        }
        "ping" => json!({}),
        "tools/list" => tools_list_result(era, &state.server_version),
        "tools/call" => {
            return MessageAction::ToolCall {
                id: response_id,
                params,
                era,
            }
        }
        _ => {
            return MessageAction::Response(rpc_error(
                response_id,
                -32601,
                "Method not found",
                Some(json!({"method": method})),
            ))
        }
    };
    MessageAction::Response(rpc_result(response_id, result))
}

fn valid_request_id(id: &Value) -> bool {
    id.is_string() || id.is_i64() || id.is_u64()
}

fn request_id_key(id: &Value) -> Option<String> {
    if let Some(value) = id.as_str() {
        Some(format!("s:{value}"))
    } else if let Some(value) = id.as_i64() {
        Some(format!("i:{value}"))
    } else {
        id.as_u64().map(|value| format!("u:{value}"))
    }
}

fn handle_notification(method: &str, params: &Value, active_requests: &ActiveRequests) {
    if method != "notifications/cancelled" {
        return;
    }
    let Some(key) = params.get("requestId").and_then(request_id_key) else {
        return;
    };
    if let Ok(active) = active_requests.lock() {
        if let Some(cancellation) = active.get(&key) {
            cancellation.store(true, Ordering::Release);
        }
    }
}

fn request_era(state: &ServerState, params: &Value) -> Result<ProtocolEra, Value> {
    let metadata = params.get("_meta").and_then(Value::as_object);
    if let Some(metadata) = metadata {
        if metadata.contains_key("io.modelcontextprotocol/protocolVersion") {
            let version = metadata
                .get("io.modelcontextprotocol/protocolVersion")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    rpc_error_without_id(
                        -32602,
                        "io.modelcontextprotocol/protocolVersion must be a string",
                        None,
                    )
                })?;
            if metadata
                .get("io.modelcontextprotocol/clientCapabilities")
                .and_then(Value::as_object)
                .is_none()
            {
                return Err(rpc_error_without_id(
                    -32602,
                    "io.modelcontextprotocol/clientCapabilities must be an object",
                    None,
                ));
            }
            if metadata
                .get("io.modelcontextprotocol/clientInfo")
                .is_some_and(|client_info| !valid_implementation(client_info))
            {
                return Err(rpc_error_without_id(
                    -32602,
                    "io.modelcontextprotocol/clientInfo must contain string name and version fields",
                    None,
                ));
            }
            if version != CURRENT_PROTOCOL_VERSION {
                return Err(rpc_error_without_id(
                    -32022,
                    "Unsupported protocol version",
                    Some(json!({
                        "supported": supported_protocol_versions(),
                        "requested": version
                    })),
                ));
            }
            return Ok(ProtocolEra::Modern);
        }
    }
    if state.legacy_protocol_version.is_some() {
        Ok(ProtocolEra::Legacy)
    } else {
        Err(rpc_error_without_id(
            -32602,
            "Missing MCP request metadata; modern clients must send protocolVersion and clientCapabilities, while legacy clients must initialize first",
            Some(json!({"supported": supported_protocol_versions()})),
        ))
    }
}

fn handle_legacy_initialize(state: &mut ServerState, id: Value, params: &Value) -> Value {
    let Some(requested) = params.get("protocolVersion").and_then(Value::as_str) else {
        return rpc_error(id, -32602, "protocolVersion is required", None);
    };
    if params
        .get("capabilities")
        .and_then(Value::as_object)
        .is_none()
        || !params.get("clientInfo").is_some_and(valid_implementation)
    {
        return rpc_error(
            id,
            -32602,
            "capabilities must be an object and clientInfo must contain string name and version fields",
            None,
        );
    }
    let selected = if LEGACY_PROTOCOL_VERSIONS.contains(&requested) {
        requested
    } else {
        LATEST_LEGACY_PROTOCOL_VERSION
    };
    state.legacy_protocol_version = Some(selected.to_string());
    rpc_result(
        id,
        json!({
            "protocolVersion": selected,
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": server_info(&state.server_version),
            "instructions": server_instructions()
        }),
    )
}

fn valid_implementation(value: &Value) -> bool {
    let Some(implementation) = value.as_object() else {
        return false;
    };
    implementation.get("name").and_then(Value::as_str).is_some()
        && implementation
            .get("version")
            .and_then(Value::as_str)
            .is_some()
}

fn supported_protocol_versions() -> Vec<&'static str> {
    std::iter::once(CURRENT_PROTOCOL_VERSION)
        .chain(LEGACY_PROTOCOL_VERSIONS.iter().copied())
        .collect()
}

fn server_info(server_version: &str) -> Value {
    json!({
        "name": "rel",
        "title": "Rel",
        "version": server_version,
        "description": "Browser capture and automation through Rel's embedded Chromium runtime",
        "websiteUrl": "https://rel.me"
    })
}

fn response_metadata(server_version: &str) -> Value {
    json!({"io.modelcontextprotocol/serverInfo": server_info(server_version)})
}

fn server_instructions() -> &'static str {
    "Use Rel to capture rendered pages or attach an ephemeral page for follow-up actions. Reuse returned page and session IDs explicitly; all browser work runs through the installed Rel app."
}

fn modern_discover_result(server_version: &str) -> Value {
    json!({
        "resultType": "complete",
        "supportedVersions": supported_protocol_versions(),
        "capabilities": {"tools": {"listChanged": false}},
        "_meta": response_metadata(server_version),
        "instructions": server_instructions(),
        "ttlMs": TOOL_LIST_TTL_MS,
        "cacheScope": "public"
    })
}

fn tools_list_result(era: ProtocolEra, server_version: &str) -> Value {
    let mut result = json!({"tools": tool_definitions()});
    if era == ProtocolEra::Modern {
        let object = result.as_object_mut().expect("tools result is an object");
        object.insert(
            "resultType".to_string(),
            Value::String("complete".to_string()),
        );
        object.insert("ttlMs".to_string(), Value::from(TOOL_LIST_TTL_MS));
        object.insert(
            "cacheScope".to_string(),
            Value::String("public".to_string()),
        );
        object.insert("_meta".to_string(), response_metadata(server_version));
    }
    result
}

fn tool_definitions() -> Vec<Value> {
    vec![
        tool_definition(
            "rel_status",
            "Rel Status",
            "Inspect the installed Rel app, local agent, browser proxy, and Chromium bridge.",
            empty_object_schema(),
            read_annotations(),
        ),
        tool_definition(
            "rel_capture",
            "Capture Rendered Page",
            "Load a URL in Rel's embedded Chromium, optionally perform ordered actions, and save rendered HTML. Returns the complete validated capture event stream and an output file URI.",
            capture_schema(),
            json!({
                "readOnlyHint": false,
                "destructiveHint": true,
                "idempotentHint": false,
                "openWorldHint": true
            }),
        ),
        tool_definition(
            "rel_page_attach",
            "Attach Browser Page",
            "Create or attach an ephemeral Rel automation page and return its page ID for later rel_page_action calls.",
            page_attach_schema(),
            json!({
                "readOnlyHint": false,
                "destructiveHint": true,
                "idempotentHint": false,
                "openWorldHint": true
            }),
        ),
        tool_definition(
            "rel_page_action",
            "Act on Browser Page",
            "Perform one canonical action on an attached page and return the rendered HTML as a file resource link.",
            page_action_schema(),
            json!({
                "readOnlyHint": false,
                "destructiveHint": true,
                "idempotentHint": false,
                "openWorldHint": true
            }),
        ),
        tool_definition(
            "rel_list_sessions",
            "List Browser Sessions",
            "List persistent Rel browser sessions and their canonical Session<number> IDs, proxy assignments, and filtering settings.",
            empty_object_schema(),
            read_annotations(),
        ),
        tool_definition(
            "rel_list_proxies",
            "List Proxies",
            "List configured Rel proxy aliases and non-secret connection metadata.",
            empty_object_schema(),
            read_annotations(),
        ),
    ]
}

fn tool_definition(
    name: &str,
    title: &str,
    description: &str,
    input_schema: Value,
    annotations: Value,
) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": input_schema,
        "outputSchema": {"type": "object"},
        "annotations": annotations
    })
}

fn read_annotations() -> Value {
    json!({
        "readOnlyHint": true,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false
    })
}

fn empty_object_schema() -> Value {
    json!({"type": "object", "additionalProperties": false})
}

fn action_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "properties": {
                    "action": {"const": "click"},
                    "selector": {"type": "string", "minLength": 1}
                },
                "required": ["action", "selector"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "action": {"const": "wait-for"},
                    "selector": {"type": "string", "minLength": 1}
                },
                "required": ["action", "selector"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "action": {"const": "wait"},
                    "seconds": {"type": "number", "minimum": 0}
                },
                "required": ["action", "seconds"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "action": {"const": "click-link"},
                    "link": {"type": "string", "minLength": 1},
                    "match": {
                        "type": "object",
                        "properties": {
                            "type": {"const": "fuzzy-link"},
                            "threshold": {"type": "number", "minimum": 0, "maximum": 1}
                        },
                        "required": ["type", "threshold"],
                        "additionalProperties": false
                    }
                },
                "required": ["action", "link", "match"],
                "additionalProperties": false
            }
        ]
    })
}

fn capture_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "url": {"type": "string", "minLength": 1},
            "output_uri": {"type": "string", "format": "uri", "pattern": "^file:///"},
            "timeout": {"type": "number", "exclusiveMinimum": 0},
            "wait": {"type": "number", "minimum": 0},
            "actions": {"type": "array", "items": action_schema()},
            "session_id": {"type": "string", "minLength": 1},
            "proxy": {"type": "string", "minLength": 1},
            "retry": {"type": "integer", "minimum": 0, "maximum": 100},
            "retry_delay": {"type": "number", "minimum": 0, "maximum": 86400}
        },
        "required": ["url"],
        "additionalProperties": false
    })
}

fn page_attach_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "url": {"type": "string", "minLength": 1},
            "session_id": {"type": "string", "minLength": 1},
            "proxy": {"type": "string", "minLength": 1},
            "output_uri": {"type": "string", "format": "uri", "pattern": "^file:///"},
            "timeout": {"type": "number", "exclusiveMinimum": 0},
            "wait": {"type": "number", "minimum": 0}
        },
        "required": ["url"],
        "additionalProperties": false
    })
}

fn page_action_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "page_id": {"type": "string", "minLength": 1},
            "action": action_schema(),
            "output_uri": {"type": "string", "format": "uri", "pattern": "^file:///"},
            "timeout": {"type": "number", "exclusiveMinimum": 0},
            "wait": {"type": "number", "minimum": 0}
        },
        "required": ["page_id", "action"],
        "additionalProperties": false
    })
}

fn handle_tool_call(
    client: &RelClient,
    params: &Value,
    era: ProtocolEra,
    server_version: &str,
) -> Result<Value, Value> {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return Err(rpc_error_without_id(-32602, "Tool name is required", None));
    };
    if !tool_definitions()
        .iter()
        .any(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
    {
        return Err(rpc_error_without_id(
            -32602,
            &format!("Unknown tool: {name}"),
            None,
        ));
    }
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if !arguments.is_object() {
        return Err(rpc_error_without_id(
            -32602,
            "Tool arguments must be an object",
            None,
        ));
    }

    let (structured, is_error) = match name {
        "rel_status" => decode_empty_arguments(arguments)
            .and_then(|()| client.status().map_err(client_error_value))
            .and_then(to_json_value),
        "rel_list_sessions" => decode_empty_arguments(arguments)
            .and_then(|()| client.list_sessions().map_err(client_error_value))
            .and_then(to_json_value),
        "rel_list_proxies" => decode_empty_arguments(arguments)
            .and_then(|()| client.list_proxies().map_err(client_error_value))
            .and_then(to_json_value),
        "rel_page_attach" => decode_arguments::<PageAttachArguments>(arguments)
            .and_then(PageAttachRequest::try_from)
            .and_then(|request| client.attach_page(&request).map_err(client_error_value))
            .and_then(to_json_value),
        "rel_page_action" => decode_arguments::<PageActionArguments>(arguments)
            .and_then(PageActionArguments::into_request)
            .and_then(|(page_id, request)| {
                client
                    .perform_page_action(&page_id, &request)
                    .map_err(client_error_value)
                    .and_then(to_json_value)
            }),
        "rel_capture" => decode_arguments::<CaptureArguments>(arguments)
            .and_then(CaptureRequest::try_from)
            .and_then(|request| capture_tool(client, &request)),
        _ => unreachable!("tool name was validated"),
    }
    .map(|value| (value, false))
    .unwrap_or_else(|error| (error, true));

    let is_error = is_error
        || (name == "rel_capture"
            && structured
                .get("exit_code")
                .and_then(Value::as_i64)
                .is_some_and(|exit_code| exit_code != 0));
    let (structured, resource_links, is_error) = match normalize_output_uris(structured) {
        Ok((structured, resource_links)) => (structured, resource_links, is_error),
        Err(error) => (error, Vec::new(), true),
    };
    Ok(tool_result(
        structured,
        resource_links,
        is_error,
        era,
        server_version,
    ))
}

fn to_json_value<T: serde::Serialize>(value: T) -> Result<Value, Value> {
    serde_json::to_value(value).map_err(|error| {
        tool_error_value(
            "MCP_ENCODING_ERROR",
            &format!("Could not encode Rel response: {error}"),
        )
    })
}

fn capture_tool(client: &RelClient, request: &CaptureRequest) -> Result<Value, Value> {
    let mut stream = client.capture(request).map_err(client_error_value)?;
    let request_id = stream.request_id().to_string();
    let mut events = Vec::new();
    for event in stream.by_ref() {
        match event {
            Ok(event) => events.push(event),
            Err(error) => {
                return Err(json!({
                    "status": "error",
                    "request_id": request_id,
                    "error": client_error_object(&error),
                    "events": events
                }))
            }
        }
    }
    if !stream.is_finished() {
        return Err(json!({
            "status": "error",
            "request_id": request_id,
            "error": {
                "id": "INCOMPLETE_CAPTURE_STREAM",
                "message": "Rel capture stream ended before capture.finished"
            },
            "events": events
        }));
    }
    let exit_code = stream.exit_code().unwrap_or(1);
    Ok(json!({
        "request_id": request_id,
        "exit_code": exit_code,
        "events": events
    }))
}

fn decode_empty_arguments(arguments: Value) -> Result<(), Value> {
    decode_arguments::<EmptyArguments>(arguments).map(|_| ())
}

fn decode_arguments<T: DeserializeOwned>(arguments: Value) -> Result<T, Value> {
    serde_json::from_value(arguments).map_err(|error| {
        tool_error_value(
            "INVALID_ARGUMENTS",
            &format!("Invalid tool arguments: {error}"),
        )
    })
}

fn client_error_value(error: client::ClientError) -> Value {
    match error {
        client::ClientError::Rpc(failure) => serde_json::to_value(&*failure)
            .unwrap_or_else(|_| tool_error_value("REL_RPC_ERROR", &failure.error.to_string())),
        error => tool_error_value("REL_CLIENT_ERROR", &error.to_string()),
    }
}

fn client_error_object(error: &client::ClientError) -> Value {
    match error {
        client::ClientError::Rpc(failure) => serde_json::to_value(&failure.error).unwrap_or_else(
            |_| json!({"id": "REL_RPC_ERROR", "message": failure.error.to_string()}),
        ),
        error => json!({"id": "REL_CLIENT_ERROR", "message": error.to_string()}),
    }
}

fn tool_error_value(id: &str, message: &str) -> Value {
    json!({"status": "error", "error": {"id": id, "message": message}})
}

fn output_path_from_uri(output_uri: Option<String>) -> Result<Option<String>, Value> {
    output_uri
        .map(|output_uri| {
            let uri = url::Url::parse(&output_uri).map_err(|error| {
                tool_error_value(
                    "INVALID_OUTPUT_URI",
                    &format!("Invalid output_uri {output_uri:?}: {error}"),
                )
            })?;
            if uri.scheme() != "file" || !output_uri.starts_with("file:///") {
                return Err(tool_error_value(
                    "INVALID_OUTPUT_URI",
                    "output_uri must be an absolute file:/// URI",
                ));
            }
            let path = uri.to_file_path().map_err(|()| {
                tool_error_value(
                    "INVALID_OUTPUT_URI",
                    &format!("output_uri is not a local file URI: {output_uri}"),
                )
            })?;
            path.into_os_string().into_string().map_err(|_| {
                tool_error_value("INVALID_OUTPUT_URI", "output_uri path must be valid UTF-8")
            })
        })
        .transpose()
}

fn normalize_output_uris(mut structured: Value) -> Result<(Value, Vec<Value>), Value> {
    let mut resource_links = Vec::new();
    let mut seen_uris = HashSet::new();
    normalize_output_uris_in_value(&mut structured, &mut resource_links, &mut seen_uris)?;
    Ok((structured, resource_links))
}

fn normalize_output_uris_in_value(
    value: &mut Value,
    resource_links: &mut Vec<Value>,
    seen_uris: &mut HashSet<String>,
) -> Result<(), Value> {
    match value {
        Value::Object(object) => {
            if let Some(output_path) = object.remove("output_path") {
                let output_path = output_path.as_str().ok_or_else(|| {
                    tool_error_value("INVALID_OUTPUT_PATH", "Rel output_path must be a string")
                })?;
                let path = Path::new(output_path);
                if !path.is_absolute() {
                    return Err(tool_error_value(
                        "INVALID_OUTPUT_PATH",
                        &format!("Rel returned a relative output path: {output_path}"),
                    ));
                }
                let uri = url::Url::from_file_path(path).map_err(|()| {
                    tool_error_value(
                        "INVALID_OUTPUT_PATH",
                        &format!("Could not convert Rel output path to a file URI: {output_path}"),
                    )
                })?;
                let uri = uri.to_string();
                object.insert("output_uri".to_string(), Value::String(uri.clone()));
                if seen_uris.insert(uri.clone()) {
                    let name = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("capture.html");
                    resource_links.push(json!({
                        "type": "resource_link",
                        "uri": uri,
                        "name": name,
                        "description": "Rendered HTML captured by Rel",
                        "mimeType": "text/html"
                    }));
                }
            }
            for child in object.values_mut() {
                normalize_output_uris_in_value(child, resource_links, seen_uris)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                normalize_output_uris_in_value(child, resource_links, seen_uris)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn tool_result(
    structured: Value,
    resource_links: Vec<Value>,
    is_error: bool,
    era: ProtocolEra,
    server_version: &str,
) -> Value {
    let text = serde_json::to_string_pretty(&structured)
        .unwrap_or_else(|_| "Could not encode Rel tool result".to_string());
    let mut content = vec![json!({"type": "text", "text": text})];
    content.extend(resource_links);
    let mut result = json!({
        "content": content,
        "structuredContent": structured,
        "isError": is_error
    });
    if era == ProtocolEra::Modern {
        let object = result.as_object_mut().expect("tool result is an object");
        object.insert(
            "resultType".to_string(),
            Value::String("complete".to_string()),
        );
        object.insert("_meta".to_string(), response_metadata(server_version));
    }
    result
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_error(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    with_rpc_id(id, rpc_error_without_id(code, message, data))
}

fn rpc_error_without_id(code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({
        "jsonrpc": "2.0",
        "error": {"code": code, "message": message}
    });
    if let Some(data) = data {
        error
            .get_mut("error")
            .and_then(Value::as_object_mut)
            .expect("RPC error payload is an object")
            .insert("data".to_string(), data);
    }
    error
}

fn with_rpc_id(id: Value, mut error: Value) -> Value {
    error
        .as_object_mut()
        .expect("RPC error is an object")
        .insert("id".to_string(), id);
    error
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArguments {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureArguments {
    url: String,
    output_uri: Option<String>,
    timeout: Option<f64>,
    wait: Option<f64>,
    #[serde(default)]
    actions: Vec<Action>,
    session_id: Option<String>,
    proxy: Option<String>,
    retry: Option<u32>,
    retry_delay: Option<f64>,
}

impl TryFrom<CaptureArguments> for CaptureRequest {
    type Error = Value;

    fn try_from(value: CaptureArguments) -> Result<Self, Self::Error> {
        Ok(Self {
            url: value.url,
            output: output_path_from_uri(value.output_uri)?,
            timeout: value.timeout,
            wait: value.wait,
            actions: value.actions,
            session_id: value.session_id,
            proxy: value.proxy,
            retry: value.retry,
            retry_delay: value.retry_delay,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageAttachArguments {
    url: String,
    session_id: Option<String>,
    proxy: Option<String>,
    output_uri: Option<String>,
    timeout: Option<f64>,
    wait: Option<f64>,
}

impl TryFrom<PageAttachArguments> for PageAttachRequest {
    type Error = Value;

    fn try_from(value: PageAttachArguments) -> Result<Self, Self::Error> {
        Ok(Self {
            url: value.url,
            session_id: value.session_id,
            proxy: value.proxy,
            output: output_path_from_uri(value.output_uri)?,
            timeout: value.timeout,
            wait: value.wait,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageActionArguments {
    page_id: String,
    action: Action,
    output_uri: Option<String>,
    timeout: Option<f64>,
    wait: Option<f64>,
}

impl PageActionArguments {
    fn into_request(self) -> Result<(String, PageActionRequest), Value> {
        Ok((
            self.page_id,
            PageActionRequest {
                action: self.action,
                output: output_path_from_uri(self.output_uri)?,
                timeout: self.timeout,
                wait: self.wait,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor, Read};
    use std::net::{TcpListener, TcpStream};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    const TEST_SERVER_VERSION: &str = "9.8.7";

    #[derive(Clone, Default)]
    struct SharedWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl SharedWriter {
        fn contents(&self) -> Vec<u8> {
            self.bytes.lock().unwrap().clone()
        }
    }

    impl Write for SharedWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn modern_metadata() -> Value {
        json!({
            "io.modelcontextprotocol/protocolVersion": CURRENT_PROTOCOL_VERSION,
            "io.modelcontextprotocol/clientInfo": {"name": "test", "version": "1"},
            "io.modelcontextprotocol/clientCapabilities": {}
        })
    }

    fn run_messages(messages: &[Value]) -> Vec<Value> {
        run_messages_with_client(messages, RelClient::new("http://127.0.0.1:1/v1"))
    }

    fn run_messages_with_client(messages: &[Value], client: RelClient) -> Vec<Value> {
        let input = messages
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let input_writer = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            stream.write_all(input.as_bytes()).unwrap();
            thread::sleep(Duration::from_millis(200));
        });
        let (input, _) = listener.accept().unwrap();
        let output = SharedWriter::default();
        serve(
            BufReader::new(input),
            output.clone(),
            client,
            TEST_SERVER_VERSION,
        )
        .unwrap();
        input_writer.join().unwrap();
        String::from_utf8(output.contents())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn start_test_server(response: String) -> (String, JoinHandle<String>) {
        start_delayed_test_server(response, Duration::ZERO)
    }

    fn start_delayed_test_server(
        response: String,
        delay: Duration,
    ) -> (String, JoinHandle<String>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let (request, mut stream) = read_test_request(stream);
            thread::sleep(delay);
            stream.write_all(response.as_bytes()).unwrap();
            request
        });
        (format!("http://{address}/v1"), handle)
    }

    fn read_test_request(stream: TcpStream) -> (String, TcpStream) {
        let mut reader = BufReader::new(stream);
        let mut request = String::new();
        reader.read_line(&mut request).unwrap();
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
        request.push_str(&String::from_utf8(body).unwrap());
        (request, reader.into_inner())
    }

    fn http_response(content_type: &str, request_id: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nX-Request-Id: {request_id}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    #[test]
    fn serves_modern_discovery_and_tool_listing() {
        let metadata = modern_metadata();
        let output = run_messages(&[
            json!({
                "jsonrpc": "2.0",
                "id": "discover",
                "method": "server/discover",
                "params": {"_meta": metadata}
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": {"_meta": modern_metadata()}
            }),
        ]);

        assert_eq!(output[0]["id"], "discover");
        assert_eq!(output[0]["result"]["resultType"], "complete");
        assert_eq!(
            output[0]["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["version"],
            TEST_SERVER_VERSION
        );
        assert_eq!(
            output[0]["result"]["supportedVersions"][0],
            CURRENT_PROTOCOL_VERSION
        );
        let tools = output[1]["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 6);
        assert_eq!(tools[0]["name"], "rel_status");
        assert_eq!(tools[5]["name"], "rel_list_proxies");
        assert_eq!(output[1]["result"]["resultType"], "complete");
    }

    #[test]
    fn serves_legacy_initialize_ping_and_silent_notifications() {
        let output = run_messages(&[
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "1"}
                }
            }),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}),
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}),
        ]);

        assert_eq!(output.len(), 3);
        assert_eq!(output[0]["result"]["protocolVersion"], "2025-11-25");
        assert_eq!(
            output[0]["result"]["serverInfo"]["version"],
            TEST_SERVER_VERSION
        );
        assert_eq!(output[1], json!({"jsonrpc": "2.0", "id": 2, "result": {}}));
        assert!(output[2]["result"].get("resultType").is_none());
    }

    #[test]
    fn returns_protocol_errors_for_bad_input() {
        let output = SharedWriter::default();
        serve(
            Cursor::new(b"not-json\n".to_vec()),
            output.clone(),
            RelClient::new("http://127.0.0.1:1/v1"),
            TEST_SERVER_VERSION,
        )
        .unwrap();
        let parse_error: Value = serde_json::from_slice(&output.contents()).unwrap();
        assert_eq!(parse_error["error"]["code"], -32700);

        let output = run_messages(&[json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {"_meta": {
                "io.modelcontextprotocol/protocolVersion": "1900-01-01",
                "io.modelcontextprotocol/clientCapabilities": {}
            }}
        })]);
        assert_eq!(output[0]["error"]["code"], -32022);
        assert_eq!(output[0]["id"], 1);

        let output = run_messages(&[json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "server/discover",
            "params": {"_meta": {
                "io.modelcontextprotocol/protocolVersion": CURRENT_PROTOCOL_VERSION,
                "io.modelcontextprotocol/clientCapabilities": {},
                "io.modelcontextprotocol/clientInfo": {"name": "missing-version"}
            }}
        })]);
        assert_eq!(output[0]["error"]["code"], -32602);

        let output = run_messages(&[json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {}
            }
        })]);
        assert_eq!(output[0]["error"]["code"], -32602);
    }

    #[test]
    fn tool_schemas_are_closed_objects() {
        for tool in tool_definitions() {
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert_eq!(tool["inputSchema"]["additionalProperties"], false);
            assert_eq!(tool["outputSchema"]["type"], "object");
        }
    }

    #[test]
    fn status_tool_forwards_the_complete_rpc_envelope() {
        let body = json!({
            "status": "ok",
            "request_id": "req_status",
            "data": {
                "overall_status": "ok",
                "running_count": 4,
                "total_count": 4,
                "checks": []
            }
        })
        .to_string();
        let (base_url, server) =
            start_test_server(http_response("application/json", "req_status", &body));
        let output = run_messages_with_client(
            &[json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "tools/call",
                "params": {
                    "_meta": modern_metadata(),
                    "name": "rel_status",
                    "arguments": {}
                }
            })],
            RelClient::new(base_url),
        );
        let request = server.join().unwrap();

        assert!(request.starts_with("GET /v1/status HTTP/1.1"));
        assert_eq!(output[0]["result"]["isError"], false);
        assert_eq!(
            output[0]["result"]["structuredContent"]["request_id"],
            "req_status"
        );
        let text = output[0]["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(text).unwrap(),
            json!({
                "status": "ok",
                "request_id": "req_status",
                "data": {
                    "overall_status": "ok",
                    "running_count": 4,
                    "total_count": 4,
                    "checks": []
                }
            })
        );
    }

    #[test]
    fn capture_tool_collects_validated_events_and_reports_nonzero_exit() {
        let body = [
            json!({
                "status": "ok",
                "request_id": "req_capture",
                "event": "capture.started",
                "data": {"url": "https://example.com/"}
            })
            .to_string(),
            json!({
                "status": "ok",
                "request_id": "req_capture",
                "event": "capture.completed",
                "data": {
                    "url": "https://example.com/",
                    "output_path": "/tmp/rel capture.html",
                    "bytesize": 42
                }
            })
            .to_string(),
            json!({
                "status": "ok",
                "request_id": "req_capture",
                "event": "capture.finished",
                "data": {"exit_code": 1}
            })
            .to_string(),
        ]
        .join("\n")
            + "\n";
        let (base_url, server) =
            start_test_server(http_response("application/x-ndjson", "req_capture", &body));
        let output = run_messages_with_client(
            &[json!({
                "jsonrpc": "2.0",
                "id": "capture",
                "method": "tools/call",
                "params": {
                    "_meta": modern_metadata(),
                    "name": "rel_capture",
                    "arguments": {
                        "url": "https://example.com",
                        "output_uri": "file:///tmp/result%20page.html",
                        "retry": 0
                    }
                }
            })],
            RelClient::new(base_url),
        );
        let request = server.join().unwrap();

        assert!(request.starts_with("POST /v1/captures HTTP/1.1"));
        let request_body: Value =
            serde_json::from_str(request.split_once("\r\n").unwrap().1).unwrap();
        assert_eq!(
            request_body,
            json!({
                "url": "https://example.com",
                "output": "/tmp/result page.html",
                "retry": 0
            })
        );
        assert_eq!(output[0]["result"]["isError"], true);
        assert_eq!(
            output[0]["result"]["structuredContent"]["request_id"],
            "req_capture"
        );
        assert_eq!(output[0]["result"]["structuredContent"]["exit_code"], 1);
        assert_eq!(
            output[0]["result"]["structuredContent"]["events"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            output[0]["result"]["structuredContent"]["events"][1]["data"]["output_uri"],
            "file:///tmp/rel%20capture.html"
        );
        assert!(
            output[0]["result"]["structuredContent"]["events"][1]["data"]
                .get("output_path")
                .is_none()
        );
        assert_eq!(output[0]["result"]["content"].as_array().unwrap().len(), 2);
        assert_eq!(output[0]["result"]["content"][1]["type"], "resource_link");
        assert_eq!(
            output[0]["result"]["content"][1]["uri"],
            "file:///tmp/rel%20capture.html"
        );
        assert_eq!(output[0]["result"]["content"][1]["mimeType"], "text/html");
    }

    #[test]
    fn output_uri_requires_an_absolute_file_uri() {
        for output_uri in ["https://example.com/result.html", "file:result.html"] {
            let output = run_messages(&[
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "protocolVersion": "2025-11-25",
                        "capabilities": {},
                        "clientInfo": {"name": "test", "version": "1"}
                    }
                }),
                json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "tools/call",
                    "params": {
                        "name": "rel_capture",
                        "arguments": {
                            "url": "https://example.com",
                            "output_uri": output_uri
                        }
                    }
                }),
            ]);

            assert_eq!(output[1]["result"]["isError"], true);
            assert_eq!(
                output[1]["result"]["structuredContent"]["error"]["id"],
                "INVALID_OUTPUT_URI"
            );
        }
    }

    #[test]
    fn bad_tool_arguments_are_actionable_tool_errors() {
        let output = run_messages(&[
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "1"}
                }
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "rel_capture",
                    "arguments": {"url": "https://example.com", "unexpected": true}
                }
            }),
        ]);

        assert_eq!(output[1]["result"]["isError"], true);
        assert_eq!(
            output[1]["result"]["structuredContent"]["error"]["id"],
            "INVALID_ARGUMENTS"
        );
    }

    #[test]
    fn long_tool_calls_do_not_block_ping_and_cancelled_responses_are_suppressed() {
        let body = json!({
            "status": "ok",
            "request_id": "req_slow_status",
            "data": {
                "overall_status": "ok",
                "running_count": 4,
                "total_count": 4,
                "checks": []
            }
        })
        .to_string();
        let (base_url, server) = start_delayed_test_server(
            http_response("application/json", "req_slow_status", &body),
            Duration::from_millis(100),
        );
        let output = run_messages_with_client(
            &[
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "protocolVersion": "2025-11-25",
                        "capabilities": {},
                        "clientInfo": {"name": "test", "version": "1"}
                    }
                }),
                json!({
                    "jsonrpc": "2.0",
                    "id": "slow",
                    "method": "tools/call",
                    "params": {"name": "rel_status", "arguments": {}}
                }),
                json!({"jsonrpc": "2.0", "id": "ping", "method": "ping"}),
                json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/cancelled",
                    "params": {"requestId": "slow", "reason": "test cancellation"}
                }),
            ],
            RelClient::new(base_url),
        );
        server.join().unwrap();

        assert_eq!(output.len(), 2);
        assert_eq!(output[0]["id"], 1);
        assert_eq!(output[1]["id"], "ping");
        assert_eq!(output[1]["result"], json!({}));
        assert!(output.iter().all(|response| response["id"] != "slow"));
    }

    #[test]
    fn stdin_disconnect_returns_without_waiting_for_active_tool_calls() {
        let body = json!({
            "status": "ok",
            "request_id": "req_slow_status",
            "data": {
                "overall_status": "ok",
                "running_count": 4,
                "total_count": 4,
                "checks": []
            }
        })
        .to_string();
        let (base_url, server) = start_delayed_test_server(
            http_response("application/json", "req_slow_status", &body),
            Duration::from_millis(500),
        );
        let input = [
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "1"}
                }
            }),
            json!({
                "jsonrpc": "2.0",
                "id": "slow",
                "method": "tools/call",
                "params": {"name": "rel_status", "arguments": {}}
            }),
        ]
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let input_writer = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            stream.write_all(input.as_bytes()).unwrap();
            thread::sleep(Duration::from_millis(50));
        });
        let (input, _) = listener.accept().unwrap();
        let output = SharedWriter::default();
        let started = Instant::now();

        serve(
            BufReader::new(input),
            output,
            RelClient::new(base_url),
            TEST_SERVER_VERSION,
        )
        .unwrap();

        assert!(started.elapsed() < Duration::from_millis(250));
        input_writer.join().unwrap();
        server.join().unwrap();
    }
}
