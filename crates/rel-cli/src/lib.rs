use rel_client::transfer::{
    validate_transfer_file, PROFILE_TRANSFER_FORMAT, PROXY_TRANSFER_FORMAT, TRANSFER_FORMAT_VERSION,
};
use rel_client::{
    self as client, Action, CaptureEvent, CaptureRequest, Change, ImageBlockingMode,
    NavigateRequest, ObservationActionRequest, ObservationMode, ObservationRequest,
    PageActionRequest, PageAttachRequest, PageCaptureRequest, PageObservationRequest,
    PageReadRequest, PerformRequest, ProfileTransferExportRequest, ProfileTransferImportRequest,
    ProxyCreateRequest, ProxyTls, ProxyTransferExportRequest, ProxyTransferImportRequest,
    ProxyUpdateRequest, RelClient, SessionCreateRequest, SessionUpdateRequest,
};
use serde::Serialize;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use uuid::Uuid;

pub use rel_client::rpc_error_codes;

mod app;
mod mcp;
mod transfer;

use transfer::{read_transfer_file, write_transfer_file};

pub fn main_exit_code(args: Vec<OsString>) -> i32 {
    main_exit_code_with_version(args, env!("CARGO_PKG_VERSION"))
}

pub fn mcp_main_exit_code(args: Vec<OsString>) -> i32 {
    mcp_main_exit_code_with_version(args, env!("CARGO_PKG_VERSION"))
}

pub fn mcp_main_exit_code_with_version(args: Vec<OsString>, product_version: &str) -> i32 {
    let args = match utf8_args(args) {
        Ok(args) => args,
        Err(error) => return print_cli_error(error),
    };
    match args.as_slice() {
        [] => match mcp::serve_stdio(RelClient::local(), product_version) {
            Ok(()) => 0,
            Err(error) => print_cli_error(CliError::Message(error)),
        },
        [argument] if matches!(argument.as_str(), "-h" | "--help") => {
            println!("{}", mcp_help());
            0
        }
        [argument] if argument == "--version" => {
            println!("rel-mcp {product_version}");
            0
        }
        _ => print_cli_error(CliError::Message(
            "rel-mcp accepts no arguments; run `rel-mcp --help`".to_string(),
        )),
    }
}

pub fn main_exit_code_with_version(args: Vec<OsString>, product_version: &str) -> i32 {
    let mut args = match utf8_args(args) {
        Ok(args) => args,
        Err(error) => return print_cli_error(error),
    };
    if let Err(error) =
        apply_session_url_environment_default(&mut args, std::env::var_os("REL_SESSION_URL"))
    {
        return print_cli_error(error);
    }
    let mut command = match parse_command(args) {
        Ok(command) => command,
        Err(CliError::Help(help)) => {
            println!("{help}");
            return 0;
        }
        Err(CliError::Version) => {
            println!("rel {product_version}");
            return 0;
        }
        Err(error) => return print_cli_error(error),
    };
    if let Err(error) =
        apply_session_id_environment_default(&mut command, std::env::var_os("REL_SESSION_ID"))
    {
        return print_cli_error(error);
    }

    if command.starts_app() {
        if let Err(error) = app::ensure_agent_running() {
            return print_cli_error(CliError::Message(error));
        }
    }

    let client = RelClient::local();
    if let Err(error) = apply_latest_session_default(&client, &mut command) {
        return print_cli_error(error);
    }

    match run_command(client, command) {
        Ok(exit_code) => exit_code,
        Err(error) => print_cli_error(error),
    }
}

fn print_cli_error(error: CliError) -> i32 {
    match error {
        CliError::Client(client::ClientError::Rpc(failure)) => {
            eprintln!(
                "{}",
                serde_json::to_string(&failure).unwrap_or_else(|_| failure.error.to_string())
            );
        }
        CliError::Client(error) => eprintln!("{error}"),
        CliError::Message(message) => eprintln!("{message}"),
        CliError::Help(help) => eprintln!("{help}"),
        CliError::Version => unreachable!("version is handled before execution"),
    }
    1
}

fn run_command(client: RelClient, command: CliCommand) -> Result<i32, CliError> {
    match command {
        CliCommand::Health => {
            print_json(&client.health()?)?;
            Ok(0)
        }
        CliCommand::Status => {
            let response = client.status()?;
            let exit_code = i32::from(response.data.overall_status != "ok");
            print_json(&response)?;
            Ok(exit_code)
        }
        CliCommand::Navigate(request) => {
            print_json(&client.navigate(&request)?)?;
            Ok(0)
        }
        CliCommand::Read(request) => {
            print_json(&client.read_page(&request)?)?;
            Ok(0)
        }
        CliCommand::Perform(request) => {
            print_json(&client.perform(&request)?)?;
            Ok(0)
        }
        CliCommand::CaptureCurrent(mut request) => {
            let capture_to_stdout = request.output.is_none();
            let temporary_output = if capture_to_stdout {
                let output = TemporaryCaptureOutput::new()?;
                request.output = Some(output.request_path()?);
                Some(output)
            } else {
                None
            };
            let response = client.capture_current_page(&request)?;
            if let Some(output) = temporary_output {
                let stdout = io::stdout();
                output.write_to(&mut stdout.lock())?;
            } else {
                print_json(&response)?;
            }
            Ok(0)
        }
        CliCommand::Capture(mut request) => {
            let capture_to_stdout = request.output.is_none();
            let temporary_output = if capture_to_stdout {
                let output = TemporaryCaptureOutput::new()?;
                request.output = Some(output.request_path()?);
                Some(output)
            } else {
                None
            };
            let mut stream = client.capture(&request)?;
            let stdout = io::stdout();
            let stderr = io::stderr();
            let mut stdout = stdout.lock();
            let mut stderr = stderr.lock();
            let mut completed = false;
            for event in stream.by_ref() {
                let event = event?;
                completed |= event.event == "capture.completed";
                write_capture_event(event, capture_to_stdout, &mut stderr)?;
            }
            if !stream.is_finished() {
                return Err(CliError::Message(
                    "REL capture stream ended before capture.finished".to_string(),
                ));
            }
            if completed {
                if let Some(output) = &temporary_output {
                    output.write_to(&mut stdout)?;
                }
            } else if capture_to_stdout && stream.exit_code() == Some(0) {
                return Err(CliError::Message(
                    "REL capture finished successfully without capture.completed".to_string(),
                ));
            }
            Ok(stream.exit_code().unwrap_or(1))
        }
        CliCommand::PageAttach(request) => {
            print_json(&client.attach_page(&request)?)?;
            Ok(0)
        }
        CliCommand::PageAction { page_id, request } => {
            print_json(&client.perform_page_action(&page_id, &request)?)?;
            Ok(0)
        }
        CliCommand::Observe { page_id, request } => {
            let response = match page_id {
                Some(page_id) => client.observe_page(
                    &page_id,
                    &PageObservationRequest {
                        mode: request.mode,
                        timeout: request.timeout,
                        wait: request.wait,
                    },
                )?,
                None => client.observe_current_page(&request)?,
            };
            print_json(&response)?;
            Ok(0)
        }
        CliCommand::ObservationAction {
            observation_id,
            request,
        } => {
            print_json(&client.perform_observation_action(&observation_id, &request)?)?;
            Ok(0)
        }
        CliCommand::ProxyList => {
            print_json(&client.list_proxies()?)?;
            Ok(0)
        }
        CliCommand::ProxyGet(alias) => {
            print_json(&client.get_proxy(&alias)?)?;
            Ok(0)
        }
        CliCommand::ProxyCreate(request) => {
            print_json(&client.create_proxy(&request)?)?;
            Ok(0)
        }
        CliCommand::ProxyUpdate { alias, request } => {
            print_json(&client.update_proxy(&alias, &request)?)?;
            Ok(0)
        }
        CliCommand::ProxyDelete(alias) => {
            print_json(&client.delete_proxy(&alias)?)?;
            Ok(0)
        }
        CliCommand::ProxyRotate(alias) => {
            print_json(&client.rotate_proxy_session(&alias)?)?;
            Ok(0)
        }
        CliCommand::ProxyExport { alias, output } => {
            let transfer = client
                .export_proxy_transfer(&ProxyTransferExportRequest {
                    alias,
                    include_credentials: false,
                    passphrase: None,
                })?
                .data;
            let contents = transfer.contents().map_err(CliError::Message)?;
            validate_transfer_file(&contents).map_err(CliError::Message)?;
            let path = write_transfer_file(&contents, output, &transfer.filename)
                .map_err(CliError::Message)?;
            eprintln!("warning: proxy credentials are app-protected and were not exported");
            print_json(&serde_json::json!({
                "status": "ok",
                "path": path.display().to_string(),
                "format": PROXY_TRANSFER_FORMAT,
                "version": TRANSFER_FORMAT_VERSION,
                "credentials_included": false,
            }))?;
            Ok(0)
        }
        CliCommand::ProxyImport { path, alias } => {
            let data = read_transfer_file(&path).map_err(CliError::Message)?;
            validate_transfer_file(&data).map_err(CliError::Message)?;
            print_json(
                &client.import_proxy_transfer(&ProxyTransferImportRequest::from_bytes(
                    &data, alias, None,
                ))?,
            )?;
            Ok(0)
        }
        CliCommand::ProfileList => {
            print_json(&client.list_profiles()?)?;
            Ok(0)
        }
        CliCommand::ProfileExport { name, output } => {
            let transfer = client
                .export_profile_transfer(&ProfileTransferExportRequest {
                    name,
                    include_cookies: false,
                    include_passwords: false,
                    include_proxy_credentials: false,
                    passphrase: None,
                })?
                .data;
            let contents = transfer.contents().map_err(CliError::Message)?;
            validate_transfer_file(&contents).map_err(CliError::Message)?;
            let path = write_transfer_file(&contents, output, &transfer.filename)
                .map_err(CliError::Message)?;
            print_json(&serde_json::json!({
                "status": "ok",
                "path": path.display().to_string(),
                "format": PROFILE_TRANSFER_FORMAT,
                "version": TRANSFER_FORMAT_VERSION,
                "browser_data_included": false,
            }))?;
            Ok(0)
        }
        CliCommand::ProfileImport { path, name } => {
            let data = read_transfer_file(&path).map_err(CliError::Message)?;
            validate_transfer_file(&data).map_err(CliError::Message)?;
            print_json(&client.import_profile_transfer(
                &ProfileTransferImportRequest::from_bytes(&data, name, None),
            )?)?;
            Ok(0)
        }
        CliCommand::SessionList => {
            print_json(&client.list_sessions()?)?;
            Ok(0)
        }
        CliCommand::SessionGet(id) => {
            print_json(&client.get_session(&id)?)?;
            Ok(0)
        }
        CliCommand::SessionCreate { request, id_only } => {
            print_session_create_response(&client.create_session(&request)?, id_only)?;
            Ok(0)
        }
        CliCommand::SessionUpdate { id, request } => {
            print_json(&client.update_session(&id, &request)?)?;
            Ok(0)
        }
        CliCommand::SessionPause(id) => {
            print_json(&client.pause_session(&id)?)?;
            Ok(0)
        }
        CliCommand::SessionPlay(id) => {
            print_json(&client.play_session(&id)?)?;
            Ok(0)
        }
        CliCommand::SessionDelete(id) => {
            print_json(&client.delete_session(&id)?)?;
            Ok(0)
        }
        CliCommand::SessionCloseGroup(group) => {
            print_json(&client.close_session_group(&group)?)?;
            Ok(0)
        }
    }
}

fn print_json(value: &impl Serialize) -> Result<(), CliError> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    write_json(&mut output, value)
}

fn write_json(output: &mut dyn Write, value: &impl Serialize) -> Result<(), CliError> {
    serde_json::to_writer_pretty(&mut *output, value)
        .map_err(|error| CliError::Message(error.to_string()))?;
    writeln!(output).map_err(|error| CliError::Message(error.to_string()))
}

fn print_session_create_response(
    response: &client::RpcResponse<client::SessionData>,
    id_only: bool,
) -> Result<(), CliError> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    write_session_create_response(&mut output, response, id_only)
}

fn write_session_create_response(
    output: &mut dyn Write,
    response: &client::RpcResponse<client::SessionData>,
    id_only: bool,
) -> Result<(), CliError> {
    if id_only {
        writeln!(output, "{}", response.data.session.id)
            .map_err(|error| CliError::Message(error.to_string()))
    } else {
        write_json(output, response)
    }
}

fn print_json_line_to(output: &mut dyn Write, value: &impl Serialize) -> Result<(), CliError> {
    serde_json::to_writer(&mut *output, value)
        .map_err(|error| CliError::Message(error.to_string()))?;
    writeln!(output).map_err(|error| CliError::Message(error.to_string()))
}

fn write_capture_event(
    mut event: CaptureEvent,
    capture_to_stdout: bool,
    stderr: &mut dyn Write,
) -> Result<(), CliError> {
    if capture_to_stdout {
        if let Some(data) = event
            .data
            .as_mut()
            .and_then(serde_json::Value::as_object_mut)
        {
            if data.contains_key("output_path") {
                data.insert(
                    "output_path".to_string(),
                    serde_json::Value::String("-".to_string()),
                );
            }
        }
    }
    print_json_line_to(stderr, &event)
}

struct TemporaryCaptureOutput {
    directory: PathBuf,
    path: PathBuf,
}

impl TemporaryCaptureOutput {
    fn new() -> Result<Self, CliError> {
        let directory = std::env::temp_dir().join(format!("rel-capture-{}", Uuid::new_v4()));
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder.create(&directory).map_err(|error| {
            CliError::Message(format!(
                "Could not create temporary capture directory: {error}"
            ))
        })?;
        let path = directory.join("capture.html");
        Ok(Self { directory, path })
    }

    fn request_path(&self) -> Result<String, CliError> {
        self.path
            .to_str()
            .map(str::to_string)
            .ok_or_else(|| CliError::Message("Temporary capture path is not UTF-8".to_string()))
    }

    fn write_to(&self, output: &mut dyn Write) -> Result<(), CliError> {
        let mut capture = File::open(&self.path)
            .map_err(|error| CliError::Message(format!("Could not read captured HTML: {error}")))?;
        io::copy(&mut capture, output)
            .and_then(|_| output.flush())
            .map_err(|error| CliError::Message(format!("Could not write captured HTML: {error}")))
    }

    #[cfg(test)]
    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TemporaryCaptureOutput {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.directory);
    }
}

#[derive(Debug)]
enum CliError {
    Help(String),
    Version,
    Message(String),
    Client(client::ClientError),
}

impl From<client::ClientError> for CliError {
    fn from(error: client::ClientError) -> Self {
        Self::Client(error)
    }
}

#[derive(Clone, Debug, PartialEq)]
enum CliCommand {
    Health,
    Status,
    Navigate(NavigateRequest),
    Read(PageReadRequest),
    Perform(PerformRequest),
    CaptureCurrent(PageCaptureRequest),
    Capture(CaptureRequest),
    PageAttach(PageAttachRequest),
    PageAction {
        page_id: String,
        request: PageActionRequest,
    },
    Observe {
        page_id: Option<String>,
        request: ObservationRequest,
    },
    ObservationAction {
        observation_id: String,
        request: ObservationActionRequest,
    },
    ProxyList,
    ProxyGet(String),
    ProxyCreate(ProxyCreateRequest),
    ProxyUpdate {
        alias: String,
        request: ProxyUpdateRequest,
    },
    ProxyDelete(String),
    ProxyRotate(String),
    ProxyExport {
        alias: String,
        output: Option<PathBuf>,
    },
    ProxyImport {
        path: PathBuf,
        alias: Option<String>,
    },
    ProfileList,
    ProfileExport {
        name: String,
        output: Option<PathBuf>,
    },
    ProfileImport {
        path: PathBuf,
        name: Option<String>,
    },
    SessionList,
    SessionGet(String),
    SessionCreate {
        request: SessionCreateRequest,
        id_only: bool,
    },
    SessionUpdate {
        id: String,
        request: SessionUpdateRequest,
    },
    SessionPause(String),
    SessionPlay(String),
    SessionDelete(String),
    SessionCloseGroup(String),
}

impl CliCommand {
    fn starts_app(&self) -> bool {
        !matches!(self, Self::Health | Self::Status)
    }
}

fn utf8_args(args: Vec<OsString>) -> Result<Vec<String>, CliError> {
    args.into_iter()
        .map(|argument| {
            argument
                .into_string()
                .map_err(|_| CliError::Message("arguments must be valid UTF-8".to_string()))
        })
        .collect()
}

fn parse_command(args: Vec<String>) -> Result<CliCommand, CliError> {
    let mut args = Arguments::new(args);
    let Some(command) = args.pop() else {
        return Err(CliError::Help(root_help()));
    };
    match command.as_str() {
        "-h" | "--help" => Err(CliError::Help(root_help())),
        "--version" => {
            args.finish()?;
            Err(CliError::Version)
        }
        "health" => {
            parse_no_options(&mut args, "health", root_help())?;
            Ok(CliCommand::Health)
        }
        "status" => {
            parse_no_options(&mut args, "status", root_help())?;
            Ok(CliCommand::Status)
        }
        "navigate" => parse_navigate(args),
        "read" => parse_read(args),
        "perform" => parse_perform(args),
        "capture" => parse_capture(args),
        "page" => parse_page(args),
        "observe" => parse_observe(args),
        "observation" => parse_observation(args),
        "proxy" => parse_proxy(args),
        "profile" => parse_profile(args),
        "session" => parse_session(args),
        legacy
            if matches!(legacy, "ping" | "logs")
                || legacy.starts_with("--rotate-proxy-session") =>
        {
            Err(CliError::Message(format!(
                "unsupported legacy command {legacy:?}; run `rel --help` for the RPC v1 CLI"
            )))
        }
        option if option.starts_with('-') => Err(CliError::Message(format!(
            "unknown option {option:?}; run `rel --help`"
        ))),
        url => {
            args.values.push_front(url.to_string());
            parse_capture(args)
        }
    }
}

fn apply_session_url_environment_default(
    args: &mut Vec<String>,
    environment_value: Option<OsString>,
) -> Result<(), CliError> {
    let insertion_index = match args.as_slice() {
        [command, remaining @ ..]
            if command == "navigate" && positional_url_is_omitted(remaining) =>
        {
            1
        }
        [page, attach, remaining @ ..]
            if page == "page" && attach == "attach" && positional_url_is_omitted(remaining) =>
        {
            2
        }
        _ => return Ok(()),
    };
    let Some(environment_value) = environment_value else {
        return Ok(());
    };
    let environment_value = environment_value
        .into_string()
        .map_err(|_| CliError::Message("REL_SESSION_URL must be valid UTF-8".to_string()))?;
    let environment_value = environment_value.trim();
    if environment_value.is_empty() {
        return Err(CliError::Message(
            "REL_SESSION_URL must not be empty".to_string(),
        ));
    }
    args.insert(insertion_index, environment_value.to_string());
    Ok(())
}

fn positional_url_is_omitted(remaining: &[String]) -> bool {
    remaining.first().map_or(true, |value| {
        value.starts_with('-') && !matches!(value.as_str(), "-h" | "--help")
    })
}

fn apply_session_id_environment_default(
    command: &mut CliCommand,
    environment_value: Option<OsString>,
) -> Result<(), CliError> {
    let Some(session_id) = session_id_default_target(command) else {
        return Ok(());
    };
    if session_id.is_some() {
        return Ok(());
    }
    let Some(environment_value) = environment_value else {
        return Ok(());
    };
    let environment_value = environment_value
        .into_string()
        .map_err(|_| CliError::Message("REL_SESSION_ID must be valid UTF-8".to_string()))?;
    let environment_value = environment_value.trim();
    if environment_value.is_empty() {
        return Err(CliError::Message(
            "REL_SESSION_ID must not be empty".to_string(),
        ));
    }
    if environment_value.len() > 1_024 {
        return Err(CliError::Message(
            "REL_SESSION_ID must not be longer than 1024 bytes".to_string(),
        ));
    }
    *session_id = Some(environment_value.to_string());
    Ok(())
}

fn apply_latest_session_default(
    client: &RelClient,
    command: &mut CliCommand,
) -> Result<(), CliError> {
    if !needs_session_id_default(command) {
        return Ok(());
    }
    let sessions = client.list_sessions()?.data.sessions;
    apply_latest_session_id(command, sessions.iter().map(|session| session.id.as_str()));
    Ok(())
}

fn needs_session_id_default(command: &mut CliCommand) -> bool {
    session_id_default_target(command).is_some_and(|session_id| session_id.is_none())
}

fn apply_latest_session_id<'a>(
    command: &mut CliCommand,
    session_ids: impl IntoIterator<Item = &'a str>,
) {
    let Some(session_id) = session_id_default_target(command) else {
        return;
    };
    if session_id.is_none() {
        *session_id = latest_session_id(session_ids).map(str::to_string);
    }
}

fn session_id_default_target(command: &mut CliCommand) -> Option<&mut Option<String>> {
    match command {
        CliCommand::Navigate(request) if request.profile.is_none() => Some(&mut request.session_id),
        CliCommand::Read(request) if request.profile.is_none() => Some(&mut request.session_id),
        CliCommand::Capture(request) if request.group.is_none() && request.profile.is_none() => {
            Some(&mut request.session_id)
        }
        CliCommand::CaptureCurrent(request) => Some(&mut request.session_id),
        CliCommand::Perform(request) => Some(&mut request.session_id),
        CliCommand::PageAttach(request) if request.group.is_none() && request.profile.is_none() => {
            Some(&mut request.session_id)
        }
        CliCommand::Observe {
            page_id: None,
            request,
        } => Some(&mut request.session_id),
        _ => None,
    }
}

fn latest_session_id<'a>(session_ids: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    session_ids
        .into_iter()
        .max_by_key(|session_id| canonical_session_number(session_id))
}

fn canonical_session_number(session_id: &str) -> Option<u64> {
    session_id.strip_prefix("Session")?.parse().ok()
}

fn parse_capture(mut args: Arguments) -> Result<CliCommand, CliError> {
    if args.peek_is_help() {
        return Err(CliError::Help(capture_help()));
    }
    let captures_current_page = match args.values.front() {
        Some(value) => value.starts_with('-'),
        None => true,
    };
    if captures_current_page {
        return parse_current_capture(args);
    }
    let url = args.required_positional("capture URL")?;
    let mut request = CaptureRequest::new(url);
    while let Some((option, inline)) = args.pop_option()? {
        match option.as_str() {
            "--session-id" => request.session_id = Some(args.option_value(&option, inline)?),
            "--profile" => request.profile = Some(args.option_value(&option, inline)?),
            "--group" => request.group = Some(args.option_value(&option, inline)?),
            "--output" => request.output = Some(args.option_value(&option, inline)?),
            "--timeout" => request.timeout = Some(args.number(&option, inline)?),
            "--wait" => request.wait = Some(args.number(&option, inline)?),
            "--action" => request
                .actions
                .push(args.json_value::<Action>(&option, inline)?),
            "--actions" => request
                .actions
                .extend(args.json_value::<Vec<Action>>(&option, inline)?),
            "--proxy" => {
                request.proxy = Some(parse_proxy_selector(&args.option_value(&option, inline)?))
            }
            "--retry" => request.retry = Some(args.integer(&option, inline)?),
            "--retry-delay" => request.retry_delay = Some(args.number(&option, inline)?),
            "-h" | "--help" => return Err(CliError::Help(capture_help())),
            _ => return Err(args.unknown_option(&option, "capture")),
        }
    }
    if request.session_id.is_some() && (request.group.is_some() || request.profile.is_some()) {
        return Err(CliError::Message(
            "--group and --profile cannot be combined with --session-id".to_string(),
        ));
    }
    Ok(CliCommand::Capture(request))
}

fn parse_current_capture(mut args: Arguments) -> Result<CliCommand, CliError> {
    let mut request = PageCaptureRequest::default();
    while let Some((option, inline)) = args.pop_option()? {
        match option.as_str() {
            "--session-id" => request.session_id = Some(args.option_value(&option, inline)?),
            "--output" => request.output = Some(args.option_value(&option, inline)?),
            "--timeout" => request.timeout = Some(args.number(&option, inline)?),
            "--wait" => request.wait = Some(args.number(&option, inline)?),
            "-h" | "--help" => return Err(CliError::Help(capture_help())),
            _ => return Err(args.unknown_option(&option, "capture")),
        }
    }
    Ok(CliCommand::CaptureCurrent(request))
}

fn parse_navigate(mut args: Arguments) -> Result<CliCommand, CliError> {
    if args.peek_is_help() {
        return Err(CliError::Help(navigate_help()));
    }
    let url = args.required_positional("navigate URL")?;
    let mut request = NavigateRequest::new(url);
    while let Some((option, inline)) = args.pop_option()? {
        match option.as_str() {
            "--output" => request.output = Some(args.option_value(&option, inline)?),
            "--timeout" => request.timeout = Some(args.number(&option, inline)?),
            "--wait" => request.wait = Some(args.number(&option, inline)?),
            "--session-id" => request.session_id = Some(args.option_value(&option, inline)?),
            "--profile" => request.profile = Some(args.option_value(&option, inline)?),
            "--proxy" => {
                request.proxy = Some(parse_proxy_selector(&args.option_value(&option, inline)?))
            }
            "-h" | "--help" => return Err(CliError::Help(navigate_help())),
            _ => return Err(args.unknown_option(&option, "navigate")),
        }
    }
    if request.session_id.is_some() && request.profile.is_some() {
        return Err(CliError::Message(
            "--profile cannot be combined with --session-id".to_string(),
        ));
    }
    Ok(CliCommand::Navigate(request))
}

fn parse_read(mut args: Arguments) -> Result<CliCommand, CliError> {
    if args.peek_is_help() {
        return Err(CliError::Help(read_help()));
    }
    let mut request = PageReadRequest::default();
    if args
        .values
        .front()
        .is_some_and(|value| !value.starts_with('-'))
    {
        request.url = Some(args.required_positional("read URL")?);
    }
    while let Some((option, inline)) = args.pop_option()? {
        match option.as_str() {
            "--query" => request.query = Some(args.option_value(&option, inline)?),
            "--max-chars" => request.max_chars = Some(args.integer(&option, inline)?),
            "--max-sections" => request.max_sections = Some(args.integer(&option, inline)?),
            "--timeout" => request.timeout = Some(args.number(&option, inline)?),
            "--wait" => request.wait = Some(args.number(&option, inline)?),
            "--session-id" => request.session_id = Some(args.option_value(&option, inline)?),
            "--profile" => request.profile = Some(args.option_value(&option, inline)?),
            "--proxy" => {
                request.proxy = Some(parse_proxy_selector(&args.option_value(&option, inline)?))
            }
            "-h" | "--help" => return Err(CliError::Help(read_help())),
            _ => return Err(args.unknown_option(&option, "read")),
        }
    }
    if request.session_id.is_some() && request.profile.is_some() {
        return Err(CliError::Message(
            "--profile cannot be combined with --session-id".to_string(),
        ));
    }
    if request.url.is_none() && (request.profile.is_some() || request.proxy.is_some()) {
        return Err(CliError::Message(
            "--profile and --proxy require a URL when reading a page".to_string(),
        ));
    }
    Ok(CliCommand::Read(request))
}

fn parse_perform(mut args: Arguments) -> Result<CliCommand, CliError> {
    if args.peek_is_help() {
        return Err(CliError::Help(perform_help()));
    }
    let actions = args.required_positional("perform actions")?;
    let actions = serde_json::from_str::<Vec<Action>>(&actions).map_err(|error| {
        CliError::Message(format!("invalid perform actions JSON array: {error}"))
    })?;
    if actions.is_empty() {
        return Err(CliError::Message(
            "perform actions array must contain at least one action".to_string(),
        ));
    }
    let mut request = PerformRequest::new(actions);
    while let Some((option, inline)) = args.pop_option()? {
        match option.as_str() {
            "--session-id" => request.session_id = Some(args.option_value(&option, inline)?),
            "--output" => request.output = Some(args.option_value(&option, inline)?),
            "--timeout" => request.timeout = Some(args.number(&option, inline)?),
            "--wait" => request.wait = Some(args.number(&option, inline)?),
            "-h" | "--help" => return Err(CliError::Help(perform_help())),
            _ => return Err(args.unknown_option(&option, "perform")),
        }
    }
    Ok(CliCommand::Perform(request))
}

fn parse_page(mut args: Arguments) -> Result<CliCommand, CliError> {
    if args.peek_is_help() {
        return Err(CliError::Help(page_help()));
    }
    match args.required_positional("page subcommand")?.as_str() {
        "attach" => parse_page_attach(args),
        "action" => parse_page_action(args),
        subcommand => Err(CliError::Message(format!(
            "unknown page subcommand {subcommand:?}; run `rel page --help`"
        ))),
    }
}

fn parse_page_attach(mut args: Arguments) -> Result<CliCommand, CliError> {
    if args.peek_is_help() {
        return Err(CliError::Help(page_help()));
    }
    let url = args.required_positional("page URL")?;
    let mut request = PageAttachRequest::new(url);
    while let Some((option, inline)) = args.pop_option()? {
        match option.as_str() {
            "--output" => request.output = Some(args.option_value(&option, inline)?),
            "--timeout" => request.timeout = Some(args.number(&option, inline)?),
            "--wait" => request.wait = Some(args.number(&option, inline)?),
            "--session-id" => request.session_id = Some(args.option_value(&option, inline)?),
            "--profile" => request.profile = Some(args.option_value(&option, inline)?),
            "--group" => request.group = Some(args.option_value(&option, inline)?),
            "--proxy" => {
                request.proxy = Some(parse_proxy_selector(&args.option_value(&option, inline)?))
            }
            "-h" | "--help" => return Err(CliError::Help(page_help())),
            _ => return Err(args.unknown_option(&option, "page attach")),
        }
    }
    if request.session_id.is_some() && (request.group.is_some() || request.profile.is_some()) {
        return Err(CliError::Message(
            "--group and --profile cannot be combined with --session-id".to_string(),
        ));
    }
    Ok(CliCommand::PageAttach(request))
}

fn parse_page_action(mut args: Arguments) -> Result<CliCommand, CliError> {
    if args.peek_is_help() {
        return Err(CliError::Help(page_help()));
    }
    let page_id = args.required_positional("page ID")?;
    let mut action = None;
    let mut output = None;
    let mut timeout = None;
    let mut wait = None;
    while let Some((option, inline)) = args.pop_option()? {
        match option.as_str() {
            "--action" => set_once(
                &mut action,
                args.json_value::<Action>(&option, inline)?,
                &option,
            )?,
            "--output" => output = Some(args.option_value(&option, inline)?),
            "--timeout" => timeout = Some(args.number(&option, inline)?),
            "--wait" => wait = Some(args.number(&option, inline)?),
            "-h" | "--help" => return Err(CliError::Help(page_help())),
            _ => return Err(args.unknown_option(&option, "page action")),
        }
    }
    let action = action.ok_or_else(|| CliError::Message("--action is required".to_string()))?;
    Ok(CliCommand::PageAction {
        page_id,
        request: PageActionRequest {
            action,
            output,
            timeout,
            wait,
        },
    })
}

fn parse_observe(mut args: Arguments) -> Result<CliCommand, CliError> {
    if args.peek_is_help() {
        return Err(CliError::Help(observe_help()));
    }
    let mut page_id = None;
    let mut request = ObservationRequest::default();
    while let Some((option, inline)) = args.pop_option()? {
        match option.as_str() {
            "--page-id" => page_id = Some(args.option_value(&option, inline)?),
            "--session-id" => request.session_id = Some(args.option_value(&option, inline)?),
            "--mode" => {
                request.mode = Some(parse_observation_mode(
                    &args.option_value(&option, inline)?,
                )?)
            }
            "--timeout" => request.timeout = Some(args.number(&option, inline)?),
            "--wait" => request.wait = Some(args.number(&option, inline)?),
            "-h" | "--help" => return Err(CliError::Help(observe_help())),
            _ => return Err(args.unknown_option(&option, "observe")),
        }
    }
    if page_id.is_some() && request.session_id.is_some() {
        return Err(CliError::Message(
            "observe --page-id cannot be combined with --session-id".to_string(),
        ));
    }
    Ok(CliCommand::Observe { page_id, request })
}

fn parse_observation(mut args: Arguments) -> Result<CliCommand, CliError> {
    if args.peek_is_help() {
        return Err(CliError::Help(observation_help()));
    }
    match args.required_positional("observation subcommand")?.as_str() {
        "action" => {
            let observation_id = args.required_positional("observation ID")?;
            let mut request = None;
            while let Some((option, inline)) = args.pop_option()? {
                match option.as_str() {
                    "--request" => set_once(
                        &mut request,
                        args.json_value::<ObservationActionRequest>(&option, inline)?,
                        &option,
                    )?,
                    "-h" | "--help" => return Err(CliError::Help(observation_help())),
                    _ => return Err(args.unknown_option(&option, "observation action")),
                }
            }
            Ok(CliCommand::ObservationAction {
                observation_id,
                request: request.ok_or_else(|| {
                    CliError::Message("observation action requires --request JSON".to_string())
                })?,
            })
        }
        subcommand => Err(CliError::Message(format!(
            "unknown observation subcommand {subcommand:?}; run `rel observation --help`"
        ))),
    }
}

fn parse_observation_mode(value: &str) -> Result<ObservationMode, CliError> {
    match value {
        "semantic" => Ok(ObservationMode::Semantic),
        "hybrid" => Ok(ObservationMode::Hybrid),
        "visual" => Ok(ObservationMode::Visual),
        _ => Err(CliError::Message(
            "--mode must be semantic, hybrid, or visual".to_string(),
        )),
    }
}

fn parse_proxy(mut args: Arguments) -> Result<CliCommand, CliError> {
    if args.peek_is_help() {
        return Err(CliError::Help(proxy_help()));
    }
    match args.required_positional("proxy subcommand")?.as_str() {
        "list" => {
            parse_no_options(&mut args, "proxy list", proxy_help())?;
            Ok(CliCommand::ProxyList)
        }
        "get" => Ok(CliCommand::ProxyGet(parse_proxy_alias(&mut args)?)),
        "create" => parse_proxy_create(args),
        "update" => parse_proxy_update(args),
        "delete" => Ok(CliCommand::ProxyDelete(parse_proxy_alias(&mut args)?)),
        "rotate" => Ok(CliCommand::ProxyRotate(parse_proxy_alias(&mut args)?)),
        "export" => parse_proxy_export(args),
        "import" => parse_proxy_import(args),
        subcommand => Err(CliError::Message(format!(
            "unknown proxy subcommand {subcommand:?}; run `rel proxy --help`"
        ))),
    }
}

fn parse_proxy_tls(option: &str, value: &str) -> Result<ProxyTls, CliError> {
    if option == "--tls" {
        return match value {
            "system" => Ok(ProxyTls::System),
            "bright-data" => Ok(ProxyTls::BrightData),
            _ => Err(CliError::Message(
                "--tls must be system or bright-data; use --ca-cert PATH for a custom CA"
                    .to_string(),
            )),
        };
    }
    use std::io::Read;
    let file = File::open(value)
        .map_err(|error| CliError::Message(format!("Could not open CA file: {error}")))?;
    let mut pem = String::new();
    file.take(65_537)
        .read_to_string(&mut pem)
        .map_err(|error| CliError::Message(format!("Could not read PEM CA file: {error}")))?;
    if pem.is_empty() || pem.len() > 65_536 {
        return Err(CliError::Message(
            "CA file must contain a PEM bundle of at most 64 KiB".to_string(),
        ));
    }
    Ok(ProxyTls::Custom {
        certificate_pem: pem,
    })
}

fn parse_proxy_create(mut args: Arguments) -> Result<CliCommand, CliError> {
    let mut request = ProxyCreateRequest::default();
    while let Some((option, inline)) = args.pop_option()? {
        match option.as_str() {
            "--locale" => request.locale = Some(args.option_value(&option, inline)?),
            "--alias" => request.alias = args.option_value(&option, inline)?,
            "--upstream-host" => request.upstream_host = args.option_value(&option, inline)?,
            "--upstream-port" => request.upstream_port = args.integer(&option, inline)?,
            "--username" => request.username = Some(args.option_value(&option, inline)?),
            "--password" => request.password = Some(args.option_value(&option, inline)?),
            "--tls" | "--ca-cert" => {
                if request.tls.is_some() {
                    return Err(CliError::Message(
                        "Use only one of --tls and --ca-cert".to_string(),
                    ));
                }
                let value = args.option_value(&option, inline)?;
                request.tls = Some(parse_proxy_tls(&option, &value)?);
            }
            "--oxylabs-enabled" => request.oxylabs_enabled = Some(args.boolean(&option, inline)?),
            "--oxylabs-location-parameter" => {
                request.oxylabs_location_parameter = Some(args.option_value(&option, inline)?)
            }
            "--oxylabs-location-value" => {
                request.oxylabs_location_value = Some(args.option_value(&option, inline)?)
            }
            "-h" | "--help" => return Err(CliError::Help(proxy_help())),
            _ => return Err(args.unknown_option(&option, "proxy create")),
        }
    }
    if request.alias.is_empty() || request.upstream_host.is_empty() || request.upstream_port == 0 {
        return Err(CliError::Message(
            "proxy create requires --alias, --upstream-host, and --upstream-port".to_string(),
        ));
    }
    Ok(CliCommand::ProxyCreate(request))
}

fn parse_proxy_update(mut args: Arguments) -> Result<CliCommand, CliError> {
    let alias = args.required_positional("proxy alias")?;
    let mut request = ProxyUpdateRequest::default();
    while let Some((option, inline)) = args.pop_option()? {
        match option.as_str() {
            "--locale" => set_change(
                &mut request.locale,
                Change::Set(args.option_value(&option, inline)?),
                &option,
            )?,
            "--clear-locale" => {
                args.flag(&option, inline)?;
                set_change(&mut request.locale, Change::Clear, &option)?;
            }
            "--upstream-host" => request.upstream_host = Some(args.option_value(&option, inline)?),
            "--upstream-port" => request.upstream_port = Some(args.integer(&option, inline)?),
            "--username" => set_change(
                &mut request.username,
                Change::Set(args.option_value(&option, inline)?),
                &option,
            )?,
            "--password" => set_change(
                &mut request.password,
                Change::Set(args.option_value(&option, inline)?),
                &option,
            )?,
            "--clear-username" => {
                args.flag(&option, inline)?;
                set_change(&mut request.username, Change::Clear, &option)?;
            }
            "--clear-password" => {
                args.flag(&option, inline)?;
                set_change(&mut request.password, Change::Clear, &option)?;
            }
            "--tls" | "--ca-cert" => {
                if request.tls.is_some() {
                    return Err(CliError::Message(
                        "Use only one of --tls and --ca-cert".to_string(),
                    ));
                }
                let value = args.option_value(&option, inline)?;
                request.tls = Some(parse_proxy_tls(&option, &value)?);
            }
            "--oxylabs-enabled" => request.oxylabs_enabled = Some(args.boolean(&option, inline)?),
            "--oxylabs-location-parameter" => {
                set_change(
                    &mut request.oxylabs_location_parameter,
                    Change::Set(args.option_value(&option, inline)?),
                    &option,
                )?;
            }
            "--oxylabs-location-value" => {
                set_change(
                    &mut request.oxylabs_location_value,
                    Change::Set(args.option_value(&option, inline)?),
                    &option,
                )?;
            }
            "--clear-oxylabs-location" => {
                args.flag(&option, inline)?;
                set_change(
                    &mut request.oxylabs_location_parameter,
                    Change::Clear,
                    &option,
                )?;
                set_change(&mut request.oxylabs_location_value, Change::Clear, &option)?;
            }
            "-h" | "--help" => return Err(CliError::Help(proxy_help())),
            _ => return Err(args.unknown_option(&option, "proxy update")),
        }
    }
    if proxy_update_is_empty(&request) {
        return Err(CliError::Message(
            "proxy update requires at least one mutable option".to_string(),
        ));
    }
    Ok(CliCommand::ProxyUpdate { alias, request })
}

fn parse_proxy_export(mut args: Arguments) -> Result<CliCommand, CliError> {
    let alias = args.required_positional("proxy alias")?;
    let mut output = None;
    while let Some((option, inline)) = args.pop_option()? {
        match option.as_str() {
            "--output" => set_once(
                &mut output,
                PathBuf::from(args.option_value(&option, inline)?),
                &option,
            )?,
            "-h" | "--help" => return Err(CliError::Help(proxy_help())),
            _ => return Err(args.unknown_option(&option, "proxy export")),
        }
    }
    Ok(CliCommand::ProxyExport { alias, output })
}

fn parse_proxy_import(mut args: Arguments) -> Result<CliCommand, CliError> {
    let path = PathBuf::from(args.required_positional("proxy transfer file")?);
    let mut alias = None;
    while let Some((option, inline)) = args.pop_option()? {
        match option.as_str() {
            "--alias" => set_once(&mut alias, args.option_value(&option, inline)?, &option)?,
            "-h" | "--help" => return Err(CliError::Help(proxy_help())),
            _ => return Err(args.unknown_option(&option, "proxy import")),
        }
    }
    Ok(CliCommand::ProxyImport { path, alias })
}

fn parse_profile(mut args: Arguments) -> Result<CliCommand, CliError> {
    if args.peek_is_help() {
        return Err(CliError::Help(profile_help()));
    }
    match args.required_positional("profile subcommand")?.as_str() {
        "list" => {
            parse_no_options(&mut args, "profile list", profile_help())?;
            Ok(CliCommand::ProfileList)
        }
        "export" => parse_profile_export(args),
        "import" => parse_profile_import(args),
        subcommand => Err(CliError::Message(format!(
            "unknown profile subcommand {subcommand:?}; run `rel profile --help`"
        ))),
    }
}

fn parse_profile_export(mut args: Arguments) -> Result<CliCommand, CliError> {
    let name = args.required_positional("profile name")?;
    let mut output = None;
    while let Some((option, inline)) = args.pop_option()? {
        match option.as_str() {
            "--output" => set_once(
                &mut output,
                PathBuf::from(args.option_value(&option, inline)?),
                &option,
            )?,
            "-h" | "--help" => return Err(CliError::Help(profile_help())),
            _ => return Err(args.unknown_option(&option, "profile export")),
        }
    }
    Ok(CliCommand::ProfileExport { name, output })
}

fn parse_profile_import(mut args: Arguments) -> Result<CliCommand, CliError> {
    let path = PathBuf::from(args.required_positional("profile transfer file")?);
    let mut name = None;
    while let Some((option, inline)) = args.pop_option()? {
        match option.as_str() {
            "--name" => set_once(&mut name, args.option_value(&option, inline)?, &option)?,
            "-h" | "--help" => return Err(CliError::Help(profile_help())),
            _ => return Err(args.unknown_option(&option, "profile import")),
        }
    }
    Ok(CliCommand::ProfileImport { path, name })
}

fn parse_session(mut args: Arguments) -> Result<CliCommand, CliError> {
    if args.peek_is_help() {
        return Err(CliError::Help(session_help()));
    }
    match args.required_positional("session subcommand")?.as_str() {
        "list" => {
            parse_no_options(&mut args, "session list", session_help())?;
            Ok(CliCommand::SessionList)
        }
        "get" => Ok(CliCommand::SessionGet(parse_session_id(&mut args)?)),
        "create" => parse_session_create(args),
        "update" => parse_session_update(args),
        "pause" => Ok(CliCommand::SessionPause(parse_session_id(&mut args)?)),
        "play" => Ok(CliCommand::SessionPlay(parse_session_id(&mut args)?)),
        "delete" => Ok(CliCommand::SessionDelete(parse_session_id(&mut args)?)),
        "close" => parse_session_close_group(args),
        subcommand => Err(CliError::Message(format!(
            "unknown session subcommand {subcommand:?}; run `rel session --help`"
        ))),
    }
}

fn parse_session_create(mut args: Arguments) -> Result<CliCommand, CliError> {
    let mut request = SessionCreateRequest::default();
    let mut id_only = false;
    while let Some((option, inline)) = args.pop_option()? {
        match option.as_str() {
            "--name" => request.name = Some(args.option_value(&option, inline)?),
            "--group" => request.group = Some(args.option_value(&option, inline)?),
            "--profile" => request.profile = Some(args.option_value(&option, inline)?),
            "--proxy" => set_change(
                &mut request.proxy_alias,
                Change::Set(args.option_value(&option, inline)?),
                &option,
            )?,
            "--direct" => {
                args.flag(&option, inline)?;
                set_change(&mut request.proxy_alias, Change::Clear, &option)?;
            }
            "--adblock-enabled" => request.adblock_enabled = Some(args.boolean(&option, inline)?),
            "--image-blocking-mode" => {
                request.image_blocking_mode =
                    Some(parse_image_mode(&args.option_value(&option, inline)?)?)
            }
            "--image-size-limit-kb" => {
                request.image_size_limit_kb = Some(args.integer(&option, inline)?)
            }
            "--id-only" => {
                args.flag(&option, inline)?;
                id_only = true;
            }
            "-h" | "--help" => return Err(CliError::Help(session_help())),
            _ => return Err(args.unknown_option(&option, "session create")),
        }
    }
    Ok(CliCommand::SessionCreate { request, id_only })
}

fn parse_session_close_group(mut args: Arguments) -> Result<CliCommand, CliError> {
    let mut group = None;
    while let Some((option, inline)) = args.pop_option()? {
        match option.as_str() {
            "--group" => set_once(&mut group, args.option_value(&option, inline)?, &option)?,
            "-h" | "--help" => return Err(CliError::Help(session_help())),
            _ => return Err(args.unknown_option(&option, "session close")),
        }
    }
    let group = group
        .ok_or_else(|| CliError::Message("session close requires --group GROUP".to_string()))?;
    if group.trim().is_empty() {
        return Err(CliError::Message(
            "session group must not be empty".to_string(),
        ));
    }
    Ok(CliCommand::SessionCloseGroup(group))
}

fn parse_session_update(mut args: Arguments) -> Result<CliCommand, CliError> {
    let id = args.required_positional("session ID")?;
    if id.trim().is_empty() {
        return Err(CliError::Message(
            "session ID must not be empty".to_string(),
        ));
    }
    let mut request = SessionUpdateRequest::default();
    while let Some((option, inline)) = args.pop_option()? {
        match option.as_str() {
            "--name" => request.name = Some(args.option_value(&option, inline)?),
            "--proxy" => set_change(
                &mut request.proxy_alias,
                Change::Set(args.option_value(&option, inline)?),
                &option,
            )?,
            "--direct" => {
                args.flag(&option, inline)?;
                set_change(&mut request.proxy_alias, Change::Clear, &option)?;
            }
            "--adblock-enabled" => request.adblock_enabled = Some(args.boolean(&option, inline)?),
            "--image-blocking-mode" => {
                request.image_blocking_mode =
                    Some(parse_image_mode(&args.option_value(&option, inline)?)?)
            }
            "--image-size-limit-kb" => {
                request.image_size_limit_kb = Some(args.integer(&option, inline)?)
            }
            "-h" | "--help" => return Err(CliError::Help(session_help())),
            _ => return Err(args.unknown_option(&option, "session update")),
        }
    }
    if session_update_is_empty(&request) {
        return Err(CliError::Message(
            "session update requires at least one mutable option".to_string(),
        ));
    }
    Ok(CliCommand::SessionUpdate { id, request })
}

fn parse_session_id(args: &mut Arguments) -> Result<String, CliError> {
    if args.peek_is_help() {
        return Err(CliError::Help(session_help()));
    }
    let id = args.required_positional("session ID")?;
    args.finish()?;
    if id.trim().is_empty() {
        return Err(CliError::Message(
            "session ID must not be empty".to_string(),
        ));
    }
    Ok(id)
}

fn parse_proxy_alias(args: &mut Arguments) -> Result<String, CliError> {
    if args.peek_is_help() {
        return Err(CliError::Help(proxy_help()));
    }
    let alias = args.required_positional("proxy alias")?;
    args.finish()?;
    if alias.trim().is_empty() {
        return Err(CliError::Message(
            "proxy alias must not be empty".to_string(),
        ));
    }
    Ok(alias)
}

fn parse_no_options(args: &mut Arguments, command: &str, help: String) -> Result<(), CliError> {
    if args.peek_is_help() {
        return Err(CliError::Help(help));
    }
    args.finish().map_err(|_| {
        CliError::Message(format!(
            "{command} takes no arguments; run `rel {command} --help`"
        ))
    })
}

fn parse_proxy_selector(value: &str) -> String {
    value.to_string()
}

fn parse_image_mode(value: &str) -> Result<ImageBlockingMode, CliError> {
    match value {
        "none" => Ok(ImageBlockingMode::None),
        "all" => Ok(ImageBlockingMode::All),
        "over_limit" => Ok(ImageBlockingMode::OverLimit),
        _ => Err(CliError::Message(
            "--image-blocking-mode must be none, all, or over_limit".to_string(),
        )),
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, option: &str) -> Result<(), CliError> {
    if slot.replace(value).is_some() {
        Err(CliError::Message(format!(
            "{option} cannot be specified more than once"
        )))
    } else {
        Ok(())
    }
}

fn set_change<T>(slot: &mut Change<T>, value: Change<T>, option: &str) -> Result<(), CliError> {
    if slot.is_unchanged() {
        *slot = value;
        Ok(())
    } else {
        Err(CliError::Message(format!(
            "{option} conflicts with another option for the same field"
        )))
    }
}

fn proxy_update_is_empty(request: &ProxyUpdateRequest) -> bool {
    request.locale.is_unchanged()
        && request.upstream_host.is_none()
        && request.upstream_port.is_none()
        && request.username.is_unchanged()
        && request.password.is_unchanged()
        && request.tls.is_none()
        && request.oxylabs_enabled.is_none()
        && request.oxylabs_location_parameter.is_unchanged()
        && request.oxylabs_location_value.is_unchanged()
}

fn session_update_is_empty(request: &SessionUpdateRequest) -> bool {
    request.name.is_none()
        && request.proxy_alias.is_unchanged()
        && request.adblock_enabled.is_none()
        && request.image_blocking_mode.is_none()
        && request.image_size_limit_kb.is_none()
}

struct Arguments {
    values: VecDeque<String>,
}

impl Arguments {
    fn new(values: Vec<String>) -> Self {
        Self {
            values: values.into(),
        }
    }

    fn pop(&mut self) -> Option<String> {
        self.values.pop_front()
    }

    fn peek_is_help(&self) -> bool {
        self.values
            .front()
            .is_some_and(|value| matches!(value.as_str(), "-h" | "--help"))
    }

    fn required_positional(&mut self, label: &str) -> Result<String, CliError> {
        match self.pop() {
            Some(value) if !value.starts_with('-') => Ok(value),
            Some(value) => Err(CliError::Message(format!(
                "{label} is required before option {value:?}"
            ))),
            None => Err(CliError::Message(format!("{label} is required"))),
        }
    }

    fn pop_option(&mut self) -> Result<Option<(String, Option<String>)>, CliError> {
        let Some(value) = self.pop() else {
            return Ok(None);
        };
        if !value.starts_with('-') {
            return Err(CliError::Message(format!(
                "unexpected positional argument {value:?}"
            )));
        }
        if let Some((option, inline)) = value.split_once('=') {
            Ok(Some((option.to_string(), Some(inline.to_string()))))
        } else {
            Ok(Some((value, None)))
        }
    }

    fn option_value(&mut self, option: &str, inline: Option<String>) -> Result<String, CliError> {
        inline
            .or_else(|| self.pop())
            .ok_or_else(|| CliError::Message(format!("{option} requires a value")))
    }

    fn number(&mut self, option: &str, inline: Option<String>) -> Result<f64, CliError> {
        let value = self.option_value(option, inline)?;
        value
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| CliError::Message(format!("{option} must be a finite number")))
    }

    fn integer<T>(&mut self, option: &str, inline: Option<String>) -> Result<T, CliError>
    where
        T: std::str::FromStr,
    {
        let value = self.option_value(option, inline)?;
        value
            .parse::<T>()
            .map_err(|_| CliError::Message(format!("{option} must be an integer")))
    }

    fn boolean(&mut self, option: &str, inline: Option<String>) -> Result<bool, CliError> {
        match self.option_value(option, inline)?.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(CliError::Message(format!("{option} must be true or false"))),
        }
    }

    fn json_value<T>(&mut self, option: &str, inline: Option<String>) -> Result<T, CliError>
    where
        T: serde::de::DeserializeOwned,
    {
        let value = self.option_value(option, inline)?;
        serde_json::from_str(&value)
            .map_err(|error| CliError::Message(format!("invalid {option} JSON: {error}")))
    }

    fn flag(&self, option: &str, inline: Option<String>) -> Result<(), CliError> {
        if inline.is_some() {
            Err(CliError::Message(format!(
                "{option} does not accept a value"
            )))
        } else {
            Ok(())
        }
    }

    fn unknown_option(&self, option: &str, command: &str) -> CliError {
        CliError::Message(format!(
            "unknown {command} option {option:?}; run `rel {command} --help`"
        ))
    }

    fn finish(&mut self) -> Result<(), CliError> {
        match self.pop() {
            Some(value) => Err(CliError::Message(format!("unexpected argument {value:?}"))),
            None => Ok(()),
        }
    }
}

fn root_help() -> String {
    "REL CLI — typed client for REL RPC v1\n\n\
Usage:\n  \
rel URL [options]\n  \
rel health\n  \
rel status\n  \
rel-mcp\n  \
rel navigate [URL] [options]\n  \
rel read [URL] [--query TEXT] [options]\n  \
rel perform ACTIONS [options]\n  \
rel capture [options]           Capture the current shorthand page\n  \
rel capture URL [options]       Explicit equivalent of `rel URL`\n  \
rel page attach [URL] [options]\n  \
rel page action PAGE_ID --action JSON [options]\n  \
rel observe [--page-id ID] [--mode semantic|hybrid|visual] [options]\n  \
rel observation action OBSERVATION_ID --request JSON\n  \
rel proxy <list|get|create|update|delete|rotate|export|import> ...\n  \
rel profile <list|export|import> ...\n  \
rel session <list|get|create|update|pause|play|delete|close> ...\n  \
rel --help\n  \
rel --version\n\n\
Ordinary commands print an RPC v1 JSON envelope. Capture writes rendered HTML to\n\
stdout unless --output is supplied, and writes validated NDJSON events to stderr.\n\
`rel-mcp` serves MCP over stdio for model and agent clients.\n\
Run `rel navigate --help`, `rel read --help`, `rel perform --help`, `rel capture --help`,\n\
`rel page --help`, `rel observe --help`, `rel observation --help`,\n\
`rel proxy --help`, `rel profile --help`, or\n\
`rel session --help` for resource options. Commands that accept --session-id
use $REL_SESSION_ID when set, then the newest existing session; an explicit
option wins. Required URL arguments use $REL_SESSION_URL when omitted, and an
explicit URL wins."
        .to_string()
}

fn mcp_help() -> String {
    "REL MCP adapter — stdio bridge to the local REL API\n\n\
Usage:\n  \
rel-mcp\n  \
rel-mcp --help\n  \
rel-mcp --version\n\n\
The adapter reads newline-delimited MCP JSON-RPC from stdin and writes protocol\n\
responses to stdout. Startup and discovery do not launch REL.app; validated\n\
operational tools start it lazily when its local agent is unavailable."
        .to_string()
}

fn capture_help() -> String {
    "Usage:\n  \
rel capture [options]\n  \
rel URL [options]\n  \
rel capture URL [options]\n\n\
Options:\n  \
--output PATH\n  \
--timeout SECONDS\n  \
--wait SECONDS\n  \
--action JSON                 Repeat for multiple canonical action objects\n  \
--actions JSON                Canonical action object array\n  \
--session-id ID              Default: $REL_SESSION_ID, then newest session\n  \
--profile NAME               Create the session from this named profile\n  \
--group GROUP                Group a new URL-capture session; conflicts with --session-id\n  \
--proxy ALIAS\n  \
--retry COUNT\n  \
--retry-delay SECONDS\n\n\
Without a URL, captures the page selected by `rel navigate`. Without --output,\n\
rendered HTML is written to stdout. URL capture events are written as NDJSON\n\
to stderr."
        .to_string()
}

fn navigate_help() -> String {
    "Usage:\n  \
rel navigate [URL] [--session-id ID | --profile NAME] [--proxy ALIAS] [--output PATH] [--timeout S] [--wait S]\n\n\
Navigates the current shorthand page. The first call reuses the newest existing\n\
session, creating one only when none exists; later calls reuse that page and
session. --session-id defaults to $REL_SESSION_ID when set, then the newest
session; --profile creates a session from a named profile and suppresses that
default. URL defaults to $REL_SESSION_URL when omitted; an explicit URL wins."
        .to_string()
}

fn read_help() -> String {
    "Usage:\n  \
rel read [URL] [--query TEXT] [--max-chars COUNT] [--max-sections COUNT] [options]\n\n\
Options:\n  \
--query TEXT\n  \
--max-chars COUNT           512-32768; default 12000\n  \
--max-sections COUNT        1-100; default 24\n  \
--session-id ID             Default: $REL_SESSION_ID, then newest session\n  \
--profile NAME              Create the session from this named profile\n  \
--proxy ALIAS\n  \
--timeout SECONDS\n  \
--wait SECONDS\n\n\
Reads a URL or the current shorthand page through semantic observation and\n\
returns bounded, query-directed Markdown plus source metadata. Use `rel observe`\n\
when interaction references or a synchronized screenshot are needed. --profile\n\
and --proxy require URL."
        .to_string()
}

fn perform_help() -> String {
    "Usage:\n  \
rel perform ACTIONS [--session-id ID] [--output PATH] [--timeout S] [--wait S]\n\n\
ACTIONS is a non-empty JSON array of canonical action objects. Actions run in\n\
array order. Run `rel navigate URL` first. --session-id defaults to
\
$REL_SESSION_ID when set, then the newest existing session; an explicit value wins."
        .to_string()
}

fn page_help() -> String {
    "Usage:\n  \
rel page attach [URL] [--session-id ID | --profile NAME] [--group GROUP] [--proxy ALIAS] [--output PATH] [--timeout S] [--wait S]\n  \
rel page action PAGE_ID --action JSON [--output PATH] [--timeout S] [--wait S]\n\n\
For page attach, --session-id defaults to $REL_SESSION_ID when set, then the
newest existing session. --profile creates a session from a named profile;
--group labels a newly created session. Either creation option suppresses that
default. URL defaults to $REL_SESSION_URL when omitted; an explicit URL or
session ID wins."
        .to_string()
}

fn observe_help() -> String {
    "Usage:\n  \
rel observe [--page-id ID] [--session-id ID] [--mode semantic|hybrid|visual] [--timeout S] [--wait S]\n\n\
Observe the current shorthand page or one attached page. Hybrid and visual modes\n\
include a synchronized current-viewport PNG resource. --session-id defaults to\n\
$REL_SESSION_ID when set, then the newest session, and cannot be combined with
--page-id."
        .to_string()
}

fn observation_help() -> String {
    "Usage:\n  \
rel observation action OBSERVATION_ID --request JSON\n\n\
Perform click, type, clear, press, or select through an observation-scoped ref.\n\
The JSON request contains ref, action, optional action data, post-action mode,\n\
timeout, and wait. The command returns a new observation."
        .to_string()
}

fn proxy_help() -> String {
    "Usage:\n  \
rel proxy list\n  \
rel proxy get ALIAS\n  \
rel proxy create --alias ALIAS --upstream-host HOST --upstream-port PORT [options]\n  \
rel proxy update ALIAS [options]\n  \
rel proxy delete ALIAS\n  \
rel proxy rotate ALIAS\n  \
rel proxy export ALIAS [--output PATH]\n  \
rel proxy import FILE [--alias ALIAS]\n\n\
Write options:\n  \
--alias ALIAS --upstream-host HOST --upstream-port PORT\n  \
--tls system|bright-data OR --ca-cert PATH (PEM CA bundle)\n  \
--username USER --password PASS --locale BCP47 --oxylabs-enabled true|false\n  \
--oxylabs-location-parameter cc|country|st --oxylabs-location-value VALUE\n\
Update clear options:\n  \
--clear-username --clear-password --clear-oxylabs-location --clear-locale\n\n\
Export writes a versioned .relproxy file with non-secret routing settings.
App-protected credentials are not available to CLI export. Import accepts
settings-only SQLite archives; --alias overrides the alias stored in the file.
Use the REL app for passphrase-protected credential transfers."
        .to_string()
}

fn profile_help() -> String {
    "Usage:\n  \
rel profile list\n  \
rel profile export NAME [--output PATH]\n  \
rel profile import FILE [--name NAME]\n\n\
Export writes a versioned SQLite .relprofile file containing reusable settings.
Cookies, saved passwords, and proxy credentials are not included. Import creates
a new custom profile; --name overrides the name stored in the file. Use the REL
app for passphrase-protected private-data transfers."
        .to_string()
}

fn session_help() -> String {
    "Usage:\n  \
rel session list\n  \
rel session get SESSION_ID\n  \
rel session create [options]\n  \
rel session update SESSION_ID [options]\n  \
rel session pause SESSION_ID\n  \
rel session play SESSION_ID\n  \
rel session delete SESSION_ID\n  \
rel session close --group GROUP\n\n\
Options:\n  \
--name NAME --group GROUP --profile NAME --proxy ALIAS --adblock-enabled true|false\n  \
--image-blocking-mode none|all|over_limit --image-size-limit-kb KB\n  \
--direct                       Override the profile with a direct connection\n  \
--id-only                     For create, print only the new session ID"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rel_client::FuzzyLinkMatch;

    fn parse(args: &[&str]) -> Result<CliCommand, CliError> {
        parse_command(args.iter().map(|value| value.to_string()).collect())
    }

    fn parse_with_session_default(
        args: &[&str],
        session_id: Option<&str>,
    ) -> Result<CliCommand, CliError> {
        let mut command = parse(args)?;
        apply_session_id_environment_default(&mut command, session_id.map(OsString::from))?;
        Ok(command)
    }

    fn parse_with_url_default(
        args: &[&str],
        session_url: Option<&str>,
    ) -> Result<CliCommand, CliError> {
        let mut args = args.iter().map(|value| value.to_string()).collect();
        apply_session_url_environment_default(&mut args, session_url.map(OsString::from))?;
        parse_command(args)
    }

    #[test]
    fn parses_capture_into_the_public_sdk_request() {
        let command = parse(&[
            "capture",
            "example.com",
            "--session-id",
            "machine-x.Session2",
            "--proxy",
            "work-proxy",
            "--action",
            r#"{"action":"wait","seconds":0.5}"#,
            "--retry=0",
        ])
        .unwrap();
        let CliCommand::Capture(request) = command else {
            panic!("expected capture");
        };
        assert_eq!(request.url, "example.com");
        assert_eq!(request.session_id.as_deref(), Some("machine-x.Session2"));
        assert_eq!(request.proxy.as_deref(), Some("work-proxy"));
        assert_eq!(request.actions, vec![Action::Wait { seconds: 0.5 }]);
        assert_eq!(request.retry, Some(0));
    }

    #[test]
    fn session_environment_defaults_commands_that_accept_session_id() {
        let expected = Some("machine-x.Session7");

        let CliCommand::Capture(capture) =
            parse_with_session_default(&["capture", "https://example.com"], expected).unwrap()
        else {
            panic!("expected capture");
        };
        assert_eq!(capture.session_id.as_deref(), expected);

        let CliCommand::Navigate(navigate) =
            parse_with_session_default(&["navigate", "https://example.com"], expected).unwrap()
        else {
            panic!("expected navigate");
        };
        assert_eq!(navigate.session_id.as_deref(), expected);

        let CliCommand::Read(read) =
            parse_with_session_default(&["read", "https://example.com"], expected).unwrap()
        else {
            panic!("expected read");
        };
        assert_eq!(read.session_id.as_deref(), expected);

        let CliCommand::PageAttach(attach) =
            parse_with_session_default(&["page", "attach", "https://example.com"], expected)
                .unwrap()
        else {
            panic!("expected page attach");
        };
        assert_eq!(attach.session_id.as_deref(), expected);

        let CliCommand::Perform(perform) = parse_with_session_default(
            &["perform", r#"[{"action":"wait","seconds":0}]"#],
            expected,
        )
        .unwrap() else {
            panic!("expected perform");
        };
        assert_eq!(perform.session_id.as_deref(), expected);

        let CliCommand::CaptureCurrent(capture) =
            parse_with_session_default(&["capture"], expected).unwrap()
        else {
            panic!("expected current-page capture");
        };
        assert_eq!(capture.session_id.as_deref(), expected);
    }

    #[test]
    fn latest_session_defaults_commands_to_the_highest_assigned_id() {
        let mut command = parse(&["navigate", "https://example.com"]).unwrap();
        apply_latest_session_id(&mut command, ["Session2", "Session10", "Session9"]);
        let CliCommand::Navigate(navigate) = command else {
            panic!("expected navigate");
        };
        assert_eq!(navigate.session_id.as_deref(), Some("Session10"));

        assert_eq!(latest_session_id(std::iter::empty::<&str>()), None);
    }

    #[test]
    fn latest_session_default_does_not_override_explicit_or_creation_options() {
        let mut explicit =
            parse(&["capture", "https://example.com", "--session-id", "Session4"]).unwrap();
        apply_latest_session_id(&mut explicit, ["Session5"]);
        let CliCommand::Capture(explicit) = explicit else {
            panic!("expected capture");
        };
        assert_eq!(explicit.session_id.as_deref(), Some("Session4"));

        let mut profile =
            parse(&["capture", "https://example.com", "--profile", "Research"]).unwrap();
        apply_latest_session_id(&mut profile, ["Session5"]);
        let CliCommand::Capture(profile) = profile else {
            panic!("expected capture");
        };
        assert_eq!(profile.session_id, None);
    }

    #[test]
    fn explicit_session_id_wins_over_environment_default() {
        let CliCommand::Capture(request) = parse_with_session_default(
            &[
                "capture",
                "https://example.com",
                "--session-id",
                "machine-x.Explicit",
            ],
            Some("machine-x.Environment"),
        )
        .unwrap() else {
            panic!("expected capture");
        };

        assert_eq!(request.session_id.as_deref(), Some("machine-x.Explicit"));
    }

    #[test]
    fn session_url_defaults_required_url_arguments() {
        let expected = "https://example.com/current?tab=details#summary";

        let CliCommand::Navigate(request) =
            parse_with_url_default(&["navigate"], Some(expected)).unwrap()
        else {
            panic!("expected navigate");
        };
        assert_eq!(request.url, expected);

        let CliCommand::Navigate(request) = parse_with_url_default(
            &["navigate", "--session-id", "machine-x.Session1"],
            Some(expected),
        )
        .unwrap() else {
            panic!("expected navigate");
        };
        assert_eq!(request.url, expected);
        assert_eq!(request.session_id.as_deref(), Some("machine-x.Session1"));

        let CliCommand::PageAttach(request) =
            parse_with_url_default(&["page", "attach", "--output", "page.html"], Some(expected))
                .unwrap()
        else {
            panic!("expected page attach");
        };
        assert_eq!(request.url, expected);
        assert_eq!(request.output.as_deref(), Some("page.html"));
    }

    #[test]
    fn explicit_urls_and_current_page_commands_ignore_session_url() {
        let CliCommand::Navigate(request) = parse_with_url_default(
            &["navigate", "https://explicit.example"],
            Some("https://environment.example"),
        )
        .unwrap() else {
            panic!("expected navigate");
        };
        assert_eq!(request.url, "https://explicit.example");

        assert_eq!(
            parse_with_url_default(&["capture"], Some("https://environment.example")).unwrap(),
            CliCommand::CaptureCurrent(PageCaptureRequest::default())
        );
        let CliCommand::Read(request) =
            parse_with_url_default(&["read"], Some("https://environment.example")).unwrap()
        else {
            panic!("expected read");
        };
        assert_eq!(request.url, None);
    }

    #[test]
    fn session_url_is_validated_only_when_used() {
        let error = parse_with_url_default(&["navigate"], Some("   ")).unwrap_err();
        assert!(
            matches!(error, CliError::Message(message) if message.contains("must not be empty"))
        );

        let CliCommand::Navigate(request) =
            parse_with_url_default(&["navigate", "https://example.com"], Some("   ")).unwrap()
        else {
            panic!("expected navigate");
        };
        assert_eq!(request.url, "https://example.com");

        assert!(matches!(
            parse_with_url_default(&["navigate", "--help"], Some("   ")).unwrap_err(),
            CliError::Help(_)
        ));
    }

    #[test]
    fn session_group_labels_new_capture_and_page_sessions() {
        let CliCommand::Capture(capture) = parse_with_session_default(
            &["capture", "https://example.com", "--group", "pgm"],
            Some("machine-x.Environment"),
        )
        .unwrap() else {
            panic!("expected capture");
        };
        assert_eq!(capture.session_id, None);
        assert_eq!(capture.group.as_deref(), Some("pgm"));

        let CliCommand::PageAttach(page) = parse_with_session_default(
            &["page", "attach", "https://example.com", "--group=pgm"],
            Some("machine-x.Environment"),
        )
        .unwrap() else {
            panic!("expected page attach");
        };
        assert_eq!(page.session_id, None);
        assert_eq!(page.group.as_deref(), Some("pgm"));

        assert!(parse(&[
            "capture",
            "https://example.com",
            "--group",
            "pgm",
            "--session-id",
            "machine-x.Session1",
        ])
        .is_err());
    }

    #[test]
    fn named_profile_creates_sessions_and_suppresses_environment_default() {
        let CliCommand::Capture(capture) = parse_with_session_default(
            &[
                "capture",
                "https://example.com",
                "--profile",
                "BandwidthSaver",
            ],
            Some("machine-x.Environment"),
        )
        .unwrap() else {
            panic!("expected capture");
        };
        assert_eq!(capture.session_id, None);
        assert_eq!(capture.profile.as_deref(), Some("BandwidthSaver"));

        let CliCommand::Navigate(navigate) = parse_with_session_default(
            &["navigate", "https://example.com", "--profile", "AdBlock"],
            Some("machine-x.Environment"),
        )
        .unwrap() else {
            panic!("expected navigate");
        };
        assert_eq!(navigate.session_id, None);
        assert_eq!(navigate.profile.as_deref(), Some("AdBlock"));

        let CliCommand::SessionCreate { request, .. } =
            parse(&["session", "create", "--profile", "Research"]).unwrap()
        else {
            panic!("expected session create");
        };
        assert_eq!(request.profile.as_deref(), Some("Research"));

        assert!(parse(&[
            "page",
            "attach",
            "https://example.com",
            "--profile",
            "AdBlock",
            "--session-id",
            "machine-x.Session1",
        ])
        .is_err());
    }

    #[test]
    fn session_environment_default_is_validated_only_when_used() {
        let error = parse_with_session_default(&["navigate", "https://example.com"], Some("   "))
            .unwrap_err();
        assert!(
            matches!(error, CliError::Message(message) if message.contains("must not be empty"))
        );

        assert_eq!(
            parse_with_session_default(&["health"], Some("   ")).unwrap(),
            CliCommand::Health
        );
    }

    #[test]
    fn help_explains_session_environment_default() {
        for help in [
            root_help(),
            capture_help(),
            navigate_help(),
            read_help(),
            perform_help(),
            page_help(),
        ] {
            assert!(help.contains("REL_SESSION_ID"));
            assert!(help.contains("newest"));
        }
        for help in [root_help(), navigate_help(), page_help()] {
            assert!(help.contains("REL_SESSION_URL"));
        }
    }

    #[test]
    fn help_identifies_the_standalone_mcp_binary() {
        assert!(root_help().contains("rel-mcp"));
        assert!(mcp_help().contains("Usage:\n  rel-mcp"));
        assert!(mcp_help().contains("do not launch REL.app"));
    }

    #[test]
    fn parses_bounded_page_read_options() {
        let CliCommand::Read(request) = parse(&[
            "read",
            "https://example.com/docs",
            "--query",
            "installation",
            "--max-chars=4096",
            "--max-sections",
            "8",
            "--profile",
            "Research",
        ])
        .unwrap() else {
            panic!("expected read");
        };
        assert_eq!(request.url.as_deref(), Some("https://example.com/docs"));
        assert_eq!(request.query.as_deref(), Some("installation"));
        assert_eq!(request.max_chars, Some(4096));
        assert_eq!(request.max_sections, Some(8));
        assert_eq!(request.profile.as_deref(), Some("Research"));

        assert!(parse(&["read", "--profile", "Research"]).is_err());
    }

    #[test]
    fn parses_proxy_only_capture_as_a_new_session_request() {
        let CliCommand::Capture(request) =
            parse(&["capture", "https://example.com", "--proxy=oxylabs"]).unwrap()
        else {
            panic!("expected capture");
        };

        assert_eq!(request.session_id, None);
        assert_eq!(request.proxy.as_deref(), Some("oxylabs"));
    }

    #[test]
    fn capture_is_the_default_command() {
        let implicit = parse(&["example.com", "--proxy=oxylabs", "--wait", "0.5"]).unwrap();
        let explicit =
            parse(&["capture", "example.com", "--proxy=oxylabs", "--wait", "0.5"]).unwrap();

        assert_eq!(implicit, explicit);
        let CliCommand::Capture(request) = implicit else {
            panic!("expected capture");
        };
        assert_eq!(request.url, "example.com");
        assert_eq!(request.proxy.as_deref(), Some("oxylabs"));
        assert_eq!(request.wait, Some(0.5));
    }

    #[test]
    fn parses_stateful_shorthand_commands() {
        let CliCommand::Navigate(navigate) = parse(&[
            "navigate",
            "example.com",
            "--session-id",
            "machine-x.Session2",
            "--wait=0.5",
        ])
        .unwrap() else {
            panic!("expected navigate");
        };
        assert_eq!(navigate.url, "example.com");
        assert_eq!(navigate.session_id.as_deref(), Some("machine-x.Session2"));
        assert_eq!(navigate.wait, Some(0.5));

        let CliCommand::Perform(perform) = parse(&[
            "perform",
            r##"[{"action":"click","selector":"#more"},{"action":"wait","seconds":0.25}]"##,
            "--session-id=machine-x.Session2",
            "--timeout=15",
        ])
        .unwrap() else {
            panic!("expected perform");
        };
        assert_eq!(
            perform.actions,
            vec![
                Action::Click {
                    selector: "#more".to_string(),
                    mouse_move: None,
                    scroll: None
                },
                Action::Wait { seconds: 0.25 }
            ]
        );
        assert_eq!(perform.session_id.as_deref(), Some("machine-x.Session2"));
        assert_eq!(perform.timeout, Some(15.0));

        assert_eq!(
            parse(&["capture"]).unwrap(),
            CliCommand::CaptureCurrent(PageCaptureRequest::default())
        );
        assert_eq!(
            parse(&["capture", "--output", "page.html"]).unwrap(),
            CliCommand::CaptureCurrent(PageCaptureRequest {
                session_id: None,
                output: Some("page.html".to_string()),
                timeout: None,
                wait: None,
            })
        );
    }

    #[test]
    fn shorthand_commands_reject_missing_or_noncanonical_arguments() {
        assert!(parse(&["navigate"]).is_err());
        assert!(parse(&["perform"]).is_err());
        assert!(parse(&["perform", "click(#more)"]).is_err());
        assert!(parse(&["perform", r#"{"action":"wait","seconds":0}"#]).is_err());
        assert!(parse(&["perform", "[]"]).is_err());
        assert!(parse(&["capture", "--action", r#"{"action":"wait","seconds":0}"#]).is_err());
    }

    #[test]
    fn parses_capture_actions_array_in_order() {
        let command = parse(&[
            "capture",
            "https://example.com/page",
            "--actions",
            r##"[
                {"action":"wait","seconds":2},
                {"action":"wait-for","selector":"#loaded"},
                {"action":"click","selector":"#more"},
                {"action":"type","selector":"#search","text":"Magickraft"},
                {"action":"clear","selector":"#query"},
                {"action":"press","selector":"#search","key":"Enter"},
                {"action":"select","selector":"#genre","value":"disco"},
                {"action":"click-link","link":"https://example.com/more","match":{"type":"fuzzy-link","threshold":0.9}},
                {"action":"wait","seconds":0.5}
            ]"##,
        ])
        .unwrap();
        let CliCommand::Capture(request) = command else {
            panic!("expected capture");
        };
        assert_eq!(
            request.actions,
            vec![
                Action::Wait { seconds: 2.0 },
                Action::WaitFor {
                    selector: "#loaded".to_string(),
                    timeout: None
                },
                Action::Click {
                    selector: "#more".to_string(),
                    mouse_move: None,
                    scroll: None
                },
                Action::Type {
                    selector: "#search".to_string(),
                    text: "Magickraft".to_string()
                },
                Action::Clear {
                    selector: "#query".to_string()
                },
                Action::Press {
                    selector: "#search".to_string(),
                    key: "Enter".to_string()
                },
                Action::Select {
                    selector: "#genre".to_string(),
                    value: "disco".to_string()
                },
                Action::ClickLink {
                    link: "https://example.com/more".to_string(),
                    match_rule: FuzzyLinkMatch::new(0.9),
                    mouse_move: None,
                    scroll: None
                },
                Action::Wait { seconds: 0.5 }
            ]
        );
    }

    #[test]
    fn capture_staging_paths_are_sanitized_on_stderr() {
        let event = CaptureEvent {
            status: "ok".to_string(),
            request_id: "req_capture".to_string(),
            event: "capture.completed".to_string(),
            data: Some(serde_json::json!({
                "url": "https://example.com/",
                "output_path": "/private/tmp/rel-capture-id/capture.html",
                "bytesize": 23
            })),
            error: None,
        };
        let mut stderr = Vec::new();

        write_capture_event(event, true, &mut stderr).unwrap();

        let event: serde_json::Value = serde_json::from_slice(&stderr).unwrap();
        assert_eq!(event["event"], "capture.completed");
        assert_eq!(event["data"]["output_path"], "-");
    }

    #[test]
    fn capture_events_use_stderr_when_output_is_a_file() {
        let event = CaptureEvent {
            status: "ok".to_string(),
            request_id: "req_capture".to_string(),
            event: "capture.writing".to_string(),
            data: Some(serde_json::json!({ "output_path": "/tmp/example.html" })),
            error: None,
        };
        let mut stderr = Vec::new();

        write_capture_event(event, false, &mut stderr).unwrap();

        let event: serde_json::Value = serde_json::from_slice(&stderr).unwrap();
        assert_eq!(event["event"], "capture.writing");
        assert_eq!(event["data"]["output_path"], "/tmp/example.html");
    }

    #[test]
    fn temporary_capture_output_copies_exact_bytes_and_cleans_up() {
        use std::os::unix::fs::PermissionsExt;

        let directory;
        {
            let output = TemporaryCaptureOutput::new().unwrap();
            directory = output.directory.clone();
            assert_eq!(
                fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
            fs::write(output.path(), b"<html>\nexact bytes\n</html>").unwrap();

            let mut stdout = Vec::new();
            output.write_to(&mut stdout).unwrap();
            assert_eq!(stdout, b"<html>\nexact bytes\n</html>");
        }

        assert!(!directory.exists());
    }

    #[test]
    fn parses_every_resource_command_family() {
        assert_eq!(parse(&["health"]).unwrap(), CliCommand::Health);
        assert_eq!(parse(&["status"]).unwrap(), CliCommand::Status);
        assert_eq!(parse(&["proxy", "list"]).unwrap(), CliCommand::ProxyList);
        assert_eq!(
            parse(&["proxy", "get", "work-proxy"]).unwrap(),
            CliCommand::ProxyGet("work-proxy".to_string())
        );
        assert_eq!(
            parse(&["proxy", "delete", "work-proxy"]).unwrap(),
            CliCommand::ProxyDelete("work-proxy".to_string())
        );
        assert_eq!(
            parse(&["proxy", "rotate", "work-proxy"]).unwrap(),
            CliCommand::ProxyRotate("work-proxy".to_string())
        );
        assert_eq!(
            parse(&[
                "proxy",
                "export",
                "work-proxy",
                "--output",
                "office.relproxy",
            ])
            .unwrap(),
            CliCommand::ProxyExport {
                alias: "work-proxy".to_string(),
                output: Some(PathBuf::from("office.relproxy")),
            }
        );
        assert_eq!(
            parse(&["proxy", "import", "office.relproxy", "--alias", "backup",]).unwrap(),
            CliCommand::ProxyImport {
                path: PathBuf::from("office.relproxy"),
                alias: Some("backup".to_string()),
            }
        );
        assert_eq!(
            parse(&["profile", "list"]).unwrap(),
            CliCommand::ProfileList
        );
        assert_eq!(
            parse(&[
                "profile",
                "export",
                "Research",
                "--output=research.relprofile",
            ])
            .unwrap(),
            CliCommand::ProfileExport {
                name: "Research".to_string(),
                output: Some(PathBuf::from("research.relprofile")),
            }
        );
        assert_eq!(
            parse(&[
                "profile",
                "import",
                "research.relprofile",
                "--name",
                "Imported",
            ])
            .unwrap(),
            CliCommand::ProfileImport {
                path: PathBuf::from("research.relprofile"),
                name: Some("Imported".to_string()),
            }
        );
        assert_eq!(
            parse(&["session", "list"]).unwrap(),
            CliCommand::SessionList
        );
        assert_eq!(
            parse(&["session", "get", "machine-x.Session4"]).unwrap(),
            CliCommand::SessionGet("machine-x.Session4".to_string())
        );
        assert_eq!(
            parse(&["session", "delete", "machine-x.Session4"]).unwrap(),
            CliCommand::SessionDelete("machine-x.Session4".to_string())
        );
        assert_eq!(
            parse(&["session", "pause", "machine-x.Session4"]).unwrap(),
            CliCommand::SessionPause("machine-x.Session4".to_string())
        );
        assert_eq!(
            parse(&["session", "play", "machine-x.Session4"]).unwrap(),
            CliCommand::SessionPlay("machine-x.Session4".to_string())
        );
        assert_eq!(
            parse(&["session", "close", "--group", "pgm"]).unwrap(),
            CliCommand::SessionCloseGroup("pgm".to_string())
        );
        assert!(parse(&["session", "close"]).is_err());
        assert!(parse(&["tab", "list"]).is_err());
        let CliCommand::SessionCreate { request, .. } = parse(&[
            "session",
            "create",
            "--group",
            "pgm",
            "--adblock-enabled",
            "true",
            "--image-blocking-mode",
            "none",
        ])
        .unwrap() else {
            panic!("expected session create");
        };
        assert_eq!(request.adblock_enabled, Some(true));
        assert_eq!(request.group.as_deref(), Some("pgm"));
        assert_eq!(request.image_blocking_mode, Some(ImageBlockingMode::None));
    }

    #[test]
    fn proxy_locale_can_be_set_preserved_or_explicitly_cleared() {
        let CliCommand::ProxyCreate(request) = parse(&[
            "proxy",
            "create",
            "--alias",
            "canada",
            "--upstream-host",
            "proxy.example",
            "--upstream-port",
            "8080",
            "--locale",
            "fr-CA",
        ])
        .unwrap() else {
            panic!("expected proxy create");
        };
        assert_eq!(request.locale.as_deref(), Some("fr-CA"));
        for (options, expected) in [
            (
                vec!["--locale", "fr-CA"],
                serde_json::json!({"locale":"fr-CA"}),
            ),
            (vec!["--clear-locale"], serde_json::json!({"locale":null})),
            (
                vec!["--upstream-port", "8081"],
                serde_json::json!({"upstream_port":8081}),
            ),
        ] {
            let mut args = vec!["proxy", "update", "canada"];
            args.extend(options);
            let CliCommand::ProxyUpdate { request, .. } = parse(&args).unwrap() else {
                panic!("expected proxy update");
            };
            assert_eq!(serde_json::to_value(request).unwrap(), expected);
        }
        assert!(parse(&[
            "proxy",
            "update",
            "canada",
            "--locale",
            "fr-CA",
            "--clear-locale"
        ])
        .is_err());
    }

    #[test]
    fn proxy_tls_options_are_explicit_and_mutually_exclusive() {
        for (option, mode) in [("system", "system"), ("bright-data", "bright_data")] {
            let CliCommand::ProxyUpdate { request, .. } =
                parse(&["proxy", "update", "office", "--tls", option]).unwrap()
            else {
                panic!("expected proxy update");
            };
            assert_eq!(
                serde_json::to_value(request).unwrap(),
                serde_json::json!({"tls":{"mode":mode}})
            );
        }
        assert!(parse(&["proxy", "update", "office", "--tls", "insecure"]).is_err());
        assert!(parse(&[
            "proxy",
            "update",
            "office",
            "--tls",
            "system",
            "--ca-cert",
            "missing.pem"
        ])
        .is_err());
        assert!(parse(&["proxy", "update", "office", "--ca-cert", "missing.pem"]).is_err());
        let path = std::env::temp_dir().join(format!("rel-test-ca-{}.pem", Uuid::new_v4()));
        fs::write(&path, "test PEM contents").unwrap();
        let CliCommand::ProxyUpdate { request, .. } = parse(&[
            "proxy",
            "update",
            "office",
            "--ca-cert",
            path.to_str().unwrap(),
        ])
        .unwrap() else {
            panic!("expected proxy update");
        };
        fs::remove_file(path).unwrap();
        assert_eq!(
            request.tls,
            Some(ProxyTls::Custom {
                certificate_pem: "test PEM contents".into()
            })
        );
    }

    #[test]
    fn update_clear_options_serialize_as_explicit_nulls() {
        let CliCommand::ProxyUpdate { request, .. } = parse(&[
            "proxy",
            "update",
            "1",
            "--clear-username",
            "--clear-oxylabs-location",
        ])
        .unwrap() else {
            panic!("expected proxy update");
        };
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({
                "username": null,
                "oxylabs_location_parameter": null,
                "oxylabs_location_value": null
            })
        );

        let CliCommand::SessionUpdate { request, .. } =
            parse(&["session", "update", "machine-x.Session2", "--direct"]).unwrap()
        else {
            panic!("expected session update");
        };
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({"proxy_alias": null})
        );

        let CliCommand::SessionCreate { request, id_only } =
            parse(&["session", "create", "--direct", "--id-only"]).unwrap()
        else {
            panic!("expected session create");
        };
        assert!(id_only);
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({"proxy_alias": null})
        );

        let CliCommand::SessionCreate { request, id_only } =
            parse(&["session", "create", "--name", "Research", "--id-only"]).unwrap()
        else {
            panic!("expected session create");
        };
        assert!(id_only);
        assert_eq!(request.name.as_deref(), Some("Research"));

        assert!(parse(&["session", "create", "--proxy", "work-proxy", "--direct",]).is_err());
    }

    #[test]
    fn session_create_id_only_prints_a_shell_safe_identifier() {
        let response = client::RpcResponse {
            status: "ok".to_string(),
            request_id: "req_session".to_string(),
            data: client::SessionData {
                session: client::Session {
                    id: "machine-test.Session12".to_string(),
                    name: "Research".to_string(),
                    profile: "BandwidthSaver".to_string(),
                    profile_data_id: None,
                    group: Some("pgm".to_string()),
                    proxy_alias: Some("office".to_string()),
                    proxy_locale: Some("fr-CA".to_string()),
                    adblock_enabled: true,
                    image_blocking_mode: ImageBlockingMode::OverLimit,
                    image_size_limit_kb: 100,
                    created_at: 1,
                },
                closed_session_ids: Vec::new(),
            },
        };

        let mut id_output = Vec::new();
        write_session_create_response(&mut id_output, &response, true).unwrap();
        assert_eq!(id_output, b"machine-test.Session12\n");

        let mut json_output = Vec::new();
        write_session_create_response(&mut json_output, &response, false).unwrap();
        let decoded: serde_json::Value = serde_json::from_slice(&json_output).unwrap();
        assert_eq!(decoded["data"]["session"]["id"], "machine-test.Session12");
        assert_eq!(decoded["request_id"], "req_session");
    }

    #[test]
    fn rejects_removed_cli_surfaces() {
        for args in [vec!["ping"], vec!["logs"], vec!["--rotate-proxy-session=1"]] {
            assert!(parse(&args).is_err(), "accepted {args:?}");
        }
    }
}
