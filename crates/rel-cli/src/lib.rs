use rel_client::{
    self as client, Action, CaptureEvent, CaptureRequest, Change, ImageBlockingMode,
    NavigateRequest, PageActionRequest, PageAttachRequest, PageCaptureRequest, PerformRequest,
    ProxyCreateRequest, ProxyUpdateRequest, RelClient, SessionCreateRequest, SessionUpdateRequest,
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

pub fn main_exit_code(args: Vec<OsString>) -> i32 {
    main_exit_code_with_version(args, env!("CARGO_PKG_VERSION"))
}

pub fn main_exit_code_with_version(args: Vec<OsString>, product_version: &str) -> i32 {
    let args = match utf8_args(args) {
        Ok(args) => args,
        Err(error) => return print_cli_error(error),
    };
    let mut command = match parse_command(args) {
        Ok(command) => command,
        Err(CliError::Help(help)) => {
            println!("{help}");
            return 0;
        }
        Err(CliError::Version) => {
            println!("rel {}", env!("CARGO_PKG_VERSION"));
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

    match run_command(RelClient::local(), command, product_version) {
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

fn run_command(
    client: RelClient,
    command: CliCommand,
    product_version: &str,
) -> Result<i32, CliError> {
    match command {
        CliCommand::Mcp => mcp::serve_stdio(client, product_version)
            .map(|()| 0)
            .map_err(CliError::Message),
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
                    "Rel capture stream ended before capture.finished".to_string(),
                ));
            }
            if completed {
                if let Some(output) = &temporary_output {
                    output.write_to(&mut stdout)?;
                }
            } else if capture_to_stdout && stream.exit_code() == Some(0) {
                return Err(CliError::Message(
                    "Rel capture finished successfully without capture.completed".to_string(),
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
        CliCommand::SessionDelete(id) => {
            print_json(&client.delete_session(&id)?)?;
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
    Mcp,
    Health,
    Status,
    Navigate(NavigateRequest),
    Perform(PerformRequest),
    CaptureCurrent(PageCaptureRequest),
    Capture(CaptureRequest),
    PageAttach(PageAttachRequest),
    PageAction {
        page_id: String,
        request: PageActionRequest,
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
    SessionDelete(String),
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
        "mcp" => {
            parse_no_options(&mut args, "mcp", root_help())?;
            Ok(CliCommand::Mcp)
        }
        "navigate" => parse_navigate(args),
        "perform" => parse_perform(args),
        "capture" => parse_capture(args),
        "page" => parse_page(args),
        "proxy" => parse_proxy(args),
        "session" | "tab" => parse_session(args),
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

fn apply_session_id_environment_default(
    command: &mut CliCommand,
    environment_value: Option<OsString>,
) -> Result<(), CliError> {
    let session_id = match command {
        CliCommand::Navigate(request) if request.session_id.is_none() => &mut request.session_id,
        CliCommand::Capture(request) if request.session_id.is_none() => &mut request.session_id,
        CliCommand::CaptureCurrent(request) if request.session_id.is_none() => {
            &mut request.session_id
        }
        CliCommand::Perform(request) if request.session_id.is_none() => &mut request.session_id,
        CliCommand::PageAttach(request) if request.session_id.is_none() => &mut request.session_id,
        _ => return Ok(()),
    };
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
            "--proxy" => {
                request.proxy = Some(parse_proxy_selector(&args.option_value(&option, inline)?))
            }
            "-h" | "--help" => return Err(CliError::Help(navigate_help())),
            _ => return Err(args.unknown_option(&option, "navigate")),
        }
    }
    Ok(CliCommand::Navigate(request))
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
            "--proxy" => {
                request.proxy = Some(parse_proxy_selector(&args.option_value(&option, inline)?))
            }
            "-h" | "--help" => return Err(CliError::Help(page_help())),
            _ => return Err(args.unknown_option(&option, "page attach")),
        }
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
        subcommand => Err(CliError::Message(format!(
            "unknown proxy subcommand {subcommand:?}; run `rel proxy --help`"
        ))),
    }
}

fn parse_proxy_create(mut args: Arguments) -> Result<CliCommand, CliError> {
    let mut request = ProxyCreateRequest::default();
    while let Some((option, inline)) = args.pop_option()? {
        match option.as_str() {
            "--alias" => request.alias = args.option_value(&option, inline)?,
            "--upstream-host" => request.upstream_host = args.option_value(&option, inline)?,
            "--upstream-port" => request.upstream_port = args.integer(&option, inline)?,
            "--username" => request.username = Some(args.option_value(&option, inline)?),
            "--password" => request.password = Some(args.option_value(&option, inline)?),
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
        "delete" => Ok(CliCommand::SessionDelete(parse_session_id(&mut args)?)),
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
    request.upstream_host.is_none()
        && request.upstream_port.is_none()
        && request.username.is_unchanged()
        && request.password.is_unchanged()
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
    "Rel CLI — typed client for Rel RPC v1\n\n\
Usage:\n  \
rel URL [options]\n  \
rel health\n  \
rel status\n  \
rel mcp\n  \
rel navigate URL [options]\n  \
rel perform ACTIONS [options]\n  \
rel capture [options]           Capture the current shorthand page\n  \
rel capture URL [options]       Explicit equivalent of `rel URL`\n  \
rel page attach URL [options]\n  \
rel page action PAGE_ID --action JSON [options]\n  \
rel proxy <list|get|create|update|delete|rotate> ...\n  \
rel session <list|get|create|update|delete> ...\n  \
rel tab <list|get|create|update|delete> ...    Alias for rel session\n  \
rel --help\n  \
rel --version\n\n\
Ordinary commands print an RPC v1 JSON envelope. Capture writes rendered HTML to\n\
stdout unless --output is supplied, and writes validated NDJSON events to stderr.\n\
`rel mcp` serves MCP over stdio for model and agent clients.\n\
Run `rel navigate --help`, `rel perform --help`, `rel capture --help`,\n\
`rel page --help`, `rel proxy --help`, or\n\
`rel session --help` for resource options. Every `rel session ...` command is
also available as `rel tab ...`. Commands that accept --session-id use
$REL_SESSION_ID when the option is omitted; an explicit option wins."
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
--session-id ID              Default: $REL_SESSION_ID when set\n  \
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
rel navigate URL [--session-id ID] [--proxy ALIAS] [--output PATH] [--timeout S] [--wait S]\n\n\
Navigates the current shorthand page. The first call reuses a persisted session,\n\
creating one only when none exists; later calls reuse that page and session.
--session-id defaults to $REL_SESSION_ID when set; an explicit value wins."
        .to_string()
}

fn perform_help() -> String {
    "Usage:\n  \
rel perform ACTIONS [--session-id ID] [--output PATH] [--timeout S] [--wait S]\n\n\
ACTIONS is a non-empty JSON array of canonical action objects. Actions run in\n\
array order. Run `rel navigate URL` first. --session-id defaults to
\
$REL_SESSION_ID when set; an explicit value wins."
        .to_string()
}

fn page_help() -> String {
    "Usage:\n  \
rel page attach URL [--session-id ID] [--proxy ALIAS] [--output PATH] [--timeout S] [--wait S]\n  \
rel page action PAGE_ID --action JSON [--output PATH] [--timeout S] [--wait S]\n\n\
For page attach, --session-id defaults to $REL_SESSION_ID when set; an explicit
value wins."
        .to_string()
}

fn proxy_help() -> String {
    "Usage:\n  \
rel proxy list\n  \
rel proxy get ALIAS\n  \
rel proxy create --alias ALIAS --upstream-host HOST --upstream-port PORT [options]\n  \
rel proxy update ALIAS [options]\n  \
rel proxy delete ALIAS\n  \
rel proxy rotate ALIAS\n\n\
Write options:\n  \
--alias ALIAS --upstream-host HOST --upstream-port PORT\n  \
--username USER --password PASS --oxylabs-enabled true|false\n  \
--oxylabs-location-parameter cc|country|st --oxylabs-location-value VALUE\n\
Update clear options:\n  \
--clear-username --clear-password --clear-oxylabs-location"
        .to_string()
}

fn session_help() -> String {
    "Usage:\n  \
rel session list\n  \
rel session get SESSION_ID\n  \
rel session create [options]\n  \
rel session update SESSION_ID [options]\n  \
rel session delete SESSION_ID\n\n\
Options:\n  \
--name NAME --proxy ALIAS --adblock-enabled true|false\n  \
--image-blocking-mode none|all|over_limit --image-size-limit-kb KB\n  \
--direct                       Use a direct connection instead of the default proxy\n  \
--id-only                     For create, print only the new session ID\n\n\
`rel tab` is an alias for `rel session`."
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

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
            perform_help(),
            page_help(),
        ] {
            assert!(help.contains("REL_SESSION_ID"));
        }
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
                    selector: "#more".to_string()
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
                    selector: "#loaded".to_string()
                },
                Action::Click {
                    selector: "#more".to_string()
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
        assert_eq!(parse(&["mcp"]).unwrap(), CliCommand::Mcp);
        assert!(CliCommand::Mcp.starts_app());
        assert!(parse(&["mcp", "extra"]).is_err());
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
        assert_eq!(parse(&["tab", "list"]).unwrap(), CliCommand::SessionList);
        assert_eq!(
            parse(&["tab", "get", "machine-x.Session4"]).unwrap(),
            CliCommand::SessionGet("machine-x.Session4".to_string())
        );
        assert_eq!(
            parse(&["tab", "delete", "machine-x.Session4"]).unwrap(),
            CliCommand::SessionDelete("machine-x.Session4".to_string())
        );
        assert_eq!(
            parse(&["tab", "update", "machine-x.Session4", "--name", "Research"]).unwrap(),
            parse(&[
                "session",
                "update",
                "machine-x.Session4",
                "--name",
                "Research",
            ])
            .unwrap()
        );
        let CliCommand::SessionCreate { request, .. } = parse(&[
            "session",
            "create",
            "--adblock-enabled",
            "true",
            "--image-blocking-mode",
            "none",
        ])
        .unwrap() else {
            panic!("expected session create");
        };
        assert_eq!(request.adblock_enabled, Some(true));
        assert_eq!(request.image_blocking_mode, Some(ImageBlockingMode::None));
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
            parse(&["tab", "create", "--name", "Research", "--id-only"]).unwrap()
        else {
            panic!("expected tab create alias");
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
                    proxy_alias: Some("office".to_string()),
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
