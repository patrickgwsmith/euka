mod agent_cli;
mod claude;
mod codex;
mod feed;
mod fm;
mod http;
mod inline;
mod live;
mod pi;
mod session;
mod shell;
mod terminal;

use agent_cli::AgentUser;
use session::{
    AgentAccess, DiffMode, Event, FeedSelection, Input, LiveCommand, LiveTarget, PromptMode,
    Resource, Session,
};
use shell::Outcome;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};
use terminal::ReadResult;

struct AgentUpdate {
    id: usize,
    model: String,
    access: AgentAccess,
    result: Result<String, String>,
}

struct QueuedAgent {
    id: usize,
    model: String,
    task: String,
    cwd: PathBuf,
    answer_chars: usize,
    access: AgentAccess,
    user: AgentUser,
}

#[derive(Clone, Debug)]
struct ResolvedReferences {
    text: String,
    ids: BTreeSet<usize>,
}

enum ReferenceResolution {
    Ready(ResolvedReferences),
    Pending(Vec<usize>),
}

#[derive(Clone, Copy)]
enum AgentDisplay<'a> {
    Working,
    Answer(&'a str),
    FullAnswer(&'a str),
    Error(&'a str),
}

fn render_agent(model: &str, id: usize, display: AgentDisplay<'_>) -> String {
    let label = format!("{model}#{id}:");
    match display {
        AgentDisplay::Working => format!("{label} …"),
        AgentDisplay::FullAnswer(text) => format!("{label} {text}"),
        AgentDisplay::Answer(text) | AgentDisplay::Error(text) => {
            let prefix = if matches!(display, AgentDisplay::Error(_)) {
                format!("{label} error:")
            } else {
                label
            };
            if matches!(model, "claude" | "fable" | "opus" | "sonnet" | "haiku") {
                let budget = terminal::columns().saturating_sub(prefix.chars().count() + 1);
                format!("{prefix} {}", one_line(text, budget))
            } else {
                format!("{prefix} {text}")
            }
        }
    }
}

fn output_color() -> bool {
    (unsafe { libc::isatty(1) == 1 })
        && !std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
        && std::env::var("TERM").ok().as_deref() != Some("dumb")
}

fn render_agent_output(model: &str, id: usize, display: AgentDisplay<'_>) -> String {
    let plain = render_agent(model, id, display);
    if !output_color() || matches!(display, AgentDisplay::Working) {
        return plain;
    }
    let label = format!("{model}#{id}:");
    if let Some(body) = plain.strip_prefix(&format!("{label} error: ")) {
        return format!("\x1b[1;31m{label} error:\x1b[0m {body}");
    }
    if let Some(body) = plain.strip_prefix(&format!("{label} ")) {
        return format!("\x1b[1;35m{label}\x1b[0m {}", highlight_answer(body));
    }
    plain
}

fn highlight_answer(text: &str) -> String {
    let mut result = String::new();
    for line in text.split_inclusive('\n') {
        let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
        result.push_str(&line[..indent]);
        let rest = &line[indent..];
        if let Some(body) = rest.strip_prefix("- ").or_else(|| rest.strip_prefix("* ")) {
            result.push_str("\x1b[36m•\x1b[0m ");
            highlight_inline(body, &mut result);
        } else {
            highlight_inline(rest, &mut result);
        }
    }
    result
}

fn highlight_inline(mut text: &str, output: &mut String) {
    while !text.is_empty() {
        let bold = text.find("**").map(|at| (at, "**", "\x1b[1;36m"));
        let code = text.find('`').map(|at| (at, "`", "\x1b[36m"));
        let marker = match (bold, code) {
            (Some(bold), Some(code)) if bold.0 <= code.0 => Some(bold),
            (Some(_), Some(code)) => Some(code),
            (Some(bold), None) => Some(bold),
            (None, Some(code)) => Some(code),
            (None, None) => None,
        };
        let Some((at, delimiter, style)) = marker else {
            output.push_str(text);
            return;
        };
        output.push_str(&text[..at]);
        let after = &text[at + delimiter.len()..];
        if let Some(end) = after.find(delimiter) {
            output.push_str(style);
            output.push_str(&after[..end]);
            output.push_str("\x1b[0m");
            text = &after[end + delimiter.len()..];
        } else {
            output.push_str(delimiter);
            text = after;
        }
    }
}

struct HttpRequest {
    id: usize,
    url: String,
    head: bool,
}

struct HttpUpdate {
    id: usize,
    result: Result<HttpResult, String>,
}

enum HttpResult {
    Resource(Resource),
    Head { url: String, output: String },
}

enum WorkerUpdate {
    Agent(AgentUpdate),
    Http(HttpUpdate),
}

struct LiveUpdate {
    id: usize,
    started_at: Instant,
    result: Result<String, String>,
}

struct Workers {
    worker_tx: Sender<WorkerUpdate>,
    worker_rx: Receiver<WorkerUpdate>,
    live_tx: Sender<LiveUpdate>,
    live_rx: Receiver<LiveUpdate>,
    next_agent_id: usize,
    active_agents: usize,
    pending_agents: BTreeMap<usize, String>,
    queued_agents: BTreeMap<usize, QueuedAgent>,
    interactive: bool,
    next_http_id: usize,
    active_http: usize,
    active_live: usize,
    hosts: HashMap<String, Sender<HttpRequest>>,
}

impl Workers {
    fn new(interactive: bool) -> Self {
        let (worker_tx, worker_rx) = mpsc::channel();
        let (live_tx, live_rx) = mpsc::channel();
        Self {
            worker_tx,
            worker_rx,
            live_tx,
            live_rx,
            next_agent_id: 1,
            active_agents: 0,
            pending_agents: BTreeMap::new(),
            queued_agents: BTreeMap::new(),
            interactive,
            next_http_id: 1,
            active_http: 0,
            active_live: 0,
            hosts: HashMap::new(),
        }
    }

    fn next_result_id(&mut self) -> usize {
        let id = self.next_agent_id.max(self.next_http_id);
        self.next_agent_id = id + 1;
        self.next_http_id = id + 1;
        id
    }
}

fn main() {
    if std::env::args_os().skip(1).any(|arg| arg == "--help") {
        print_help();
        return;
    }
    let mut session = match Session::new() {
        Ok(session) => session,
        Err(error) => {
            eprintln!("euka: {error}");
            std::process::exit(1);
        }
    };
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
    }
    let interactive = unsafe { libc::isatty(0) == 1 };
    let mut terminal = terminal::Terminal::new();
    let mut workers = Workers::new(interactive);
    if interactive {
        if unsafe { libc::isatty(1) == 1 } {
            if let Err(error) = terminal::clear_screen() {
                eprintln!("euka: terminal: {error}");
                std::process::exit(1);
            }
        }
        loop {
            let color = output_color();
            let short_prompt = prompt(session.mode, &session.cwd, false, color);
            let full_prompt = prompt(session.mode, &session.cwd, true, color);
            let statuses = workers.pending_agents.values().cloned().collect();
            let line =
                match terminal.read_line(&short_prompt, &full_prompt, color, statuses, || {
                    let updates = drain_updates(&mut workers, &mut session);
                    let statuses = workers.pending_agents.values().cloned().collect();
                    (updates, statuses)
                }) {
                    Ok(ReadResult::Line(line)) => line,
                    Ok(ReadResult::Interrupt) => {
                        session.mode = PromptMode::Shell;
                        continue;
                    }
                    Ok(ReadResult::Eof) => break,
                    Err(error) => {
                        eprintln!("euka: terminal: {error}");
                        break;
                    }
                };
            if let Some(status) = handle(&line, &mut session, &mut workers, &mut terminal) {
                std::process::exit(status);
            }
        }
    } else {
        for line in io::stdin().lock().lines() {
            match line {
                Ok(line) => {
                    for update in drain_updates(&mut workers, &mut session) {
                        println!("{update}");
                    }
                    if let Some(status) = handle(&line, &mut session, &mut workers, &mut terminal) {
                        std::process::exit(status);
                    }
                }
                Err(error) => {
                    eprintln!("euka: input: {error}");
                    std::process::exit(1);
                }
            }
        }
        while workers.active_agents + workers.active_http + workers.active_live > 0 {
            match workers.worker_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(update) => {
                    println!(
                        "{}",
                        format_worker_update(update, &mut session, &mut workers)
                    );
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            for line in drain_updates(&mut workers, &mut session) {
                println!("{line}");
            }
        }
    }
    std::process::exit(session.last_status);
}

fn prompt(mode: PromptMode, cwd: &Path, full: bool, color: bool) -> String {
    let location = if full {
        cwd.to_string_lossy()
            .chars()
            .map(|character| {
                if character.is_control() {
                    '?'
                } else {
                    character
                }
            })
            .collect()
    } else {
        let home = std::env::var_os("HOME");
        shortened_path(cwd, home.as_deref().map(Path::new))
    };
    match (mode, color) {
        (PromptMode::Shell, false) => format!("{location}> "),
        (PromptMode::FmReadOnly, false) => format!("fm? {location}> "),
        (PromptMode::Shell, true) => format!("\x1b[1;36m{location}>\x1b[0m "),
        (PromptMode::FmReadOnly, true) => {
            format!("\x1b[1;35mfm?\x1b[0m \x1b[1;36m{location}>\x1b[0m ")
        }
    }
}

fn shortened_path(cwd: &Path, home: Option<&Path>) -> String {
    let (prefix, relative) = match home.and_then(|home| cwd.strip_prefix(home).ok()) {
        Some(relative) => ("~", relative),
        None if cwd.is_absolute() => ("/", cwd.strip_prefix("/").unwrap_or(cwd)),
        None => ("", cwd),
    };
    let components: Vec<String> = relative
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(name) => Some(
                name.to_string_lossy()
                    .chars()
                    .map(|character| {
                        if character.is_control() {
                            '?'
                        } else {
                            character
                        }
                    })
                    .collect(),
            ),
            _ => None,
        })
        .collect();
    if components.is_empty() {
        return if prefix.is_empty() { "." } else { prefix }.to_owned();
    }
    let last = components.len() - 1;
    let parts: Vec<String> = components
        .into_iter()
        .enumerate()
        .map(|(index, component)| {
            if index == last {
                component
            } else if let Some(hidden) = component.strip_prefix('.') {
                format!(".{}", hidden.chars().next().unwrap_or('.'))
            } else {
                component.chars().next().unwrap_or('?').to_string()
            }
        })
        .collect();
    match prefix {
        "~" => format!("~/{}", parts.join("/")),
        "/" => format!("/{}", parts.join("/")),
        _ => parts.join("/"),
    }
}

fn reset_state(
    session: &mut Session,
    workers: &mut Workers,
    terminal: &mut terminal::Terminal,
) -> io::Result<()> {
    let fresh_session = Session::new()?;
    let interactive = workers.interactive;
    *workers = Workers::new(interactive);
    *session = fresh_session;
    *terminal = terminal::Terminal::new();
    if interactive && unsafe { libc::isatty(1) == 1 } {
        terminal::clear_screen()?;
    }
    Ok(())
}

fn handle(
    line: &str,
    session: &mut Session,
    workers: &mut Workers,
    terminal: &mut terminal::Terminal,
) -> Option<i32> {
    if line.trim() == "reset" {
        if let Err(error) = reset_state(session, workers, terminal) {
            eprintln!("euka: reset: {error}");
            session.last_status = 1;
        }
        return None;
    }
    for update in drain_updates(workers, session) {
        println!("{update}");
    }
    if session.mode == PromptMode::FmReadOnly && line.trim() == "?" {
        print_help();
        return None;
    }
    if session.mode == PromptMode::FmReadOnly && matches!(line.trim(), "." | "exit") {
        session.mode = PromptMode::Shell;
        return None;
    }
    if session.mode == PromptMode::FmReadOnly && line.trim().is_empty() {
        return None;
    }
    let classified = session::classify(line);
    let input = if session.mode == PromptMode::FmReadOnly
        && !matches!(
            &classified,
            Input::AgentReference { .. } | Input::AgentNumberReference { .. }
        ) {
        Input::Agent {
            task: line.trim(),
            access: AgentAccess::ReadOnly,
            model: Some("fm"),
        }
    } else {
        classified
    };
    match input {
        Input::Help => print_help(),
        Input::Shell(command) => {
            if command.is_empty() {
                return None;
            }
            let resolved = match resolve_inline(command, session, workers) {
                Ok(resolved) => resolved,
                Err(error) => {
                    eprintln!("euka: {error}");
                    session.last_status = 1;
                    return None;
                }
            };
            let outcome = if inline::shell_substitution(&resolved) {
                shell::execute_substitution_shell(&resolved).map(Outcome::Status)
            } else {
                shell::execute(&resolved, session.last_status)
            };
            let status = match outcome {
                Ok(Outcome::Status(status)) => status,
                Ok(Outcome::Exit(status)) => return Some(status),
                Err(error) => {
                    eprintln!("euka: {error}");
                    1
                }
            };
            session.last_status = status;
            if let Ok(cwd) = std::env::current_dir() {
                session.cwd = cwd;
            }
            session.events.push(Event::Command {
                input: command.into(),
                status,
            });
        }
        Input::HostShell {
            program,
            command,
            reader,
        } => {
            let status = match shell::execute_host_shell(program, command, reader) {
                Ok(status) => status,
                Err(error) => {
                    eprintln!("euka: {error}");
                    1
                }
            };
            session.last_status = status;
            session.events.push(Event::Command {
                input: format!("{program}{} {command}", if reader { '?' } else { '!' }),
                status,
            });
        }
        Input::Comment(comment) => session.events.push(Event::Comment(comment.into())),
        Input::AgentReference { model, id } => match agent_response(session, model, id, workers) {
            Ok(text) => {
                println!(
                    "{}",
                    render_agent_output(model, id, AgentDisplay::FullAnswer(text))
                );
                session.last_status = 0;
            }
            Err(error) => {
                eprintln!("euka: {error}");
                session.last_status = 1;
            }
        },
        Input::AgentNumberReference { id } if agent_model_by_id(session, id).is_none() => {
            match watch_response_by_id(session, id, None)
                .or_else(|| http_response_by_id(session, id))
            {
                Some(text) => {
                    println!("{text}");
                    session.last_status = 0;
                }
                None => {
                    if let Some(watch) = session.live_commands.iter().find(|watch| watch.id == id) {
                        eprintln!("euka: {}", watch_reference_error(watch, None));
                    } else {
                        eprintln!("euka: no reply found for #{id}");
                    }
                    session.last_status = 1;
                }
            }
        }
        Input::WatchRevisionReference { id, revision } => {
            match watch_response_by_id(session, id, Some(revision)) {
                Some(text) => {
                    println!("{text}");
                    session.last_status = 0;
                }
                None => {
                    let error = session
                        .live_commands
                        .iter()
                        .find(|watch| watch.id == id)
                        .map(|watch| watch_reference_error(watch, Some(revision)))
                        .unwrap_or_else(|| format!("no watch found for #{id}"));
                    eprintln!("euka: {error}");
                    session.last_status = 1;
                }
            }
        }
        Input::AgentNumberReference { id } => match agent_response_by_id(session, id, workers) {
            Ok((model, text)) => {
                println!(
                    "{}",
                    render_agent_output(model, id, AgentDisplay::FullAnswer(text))
                );
                session.last_status = 0;
            }
            Err(error) => {
                eprintln!("euka: {error}");
                session.last_status = 1;
            }
        },
        Input::Todo(todo) => {
            session.todos.push(todo.into());
            println!("todo {}: {}", session.todos.len(), todo);
        }
        Input::LiveHostShell { program, command } => {
            start_live_command(program, command, session, workers)
        }
        Input::LiveHead(url) => start_live_head(url, session, workers),
        Input::LiveFeed(url, selection) => start_live_feed(url, selection, session, workers),
        Input::LiveDiff(mode) => show_live_diff(session, workers, mode),
        Input::EnterFm => {
            session.mode = PromptMode::FmReadOnly;
            println!("[fm read-only mode; ., exit, or Ctrl-C returns to the shell]");
        }
        Input::Agent {
            task,
            access,
            model,
        } => start_agents(
            task,
            access,
            &model.into_iter().collect::<Vec<_>>(),
            AgentUser::Current,
            session,
            workers,
        ),
        Input::Agents { task, models } => start_agents(
            task,
            AgentAccess::ReadOnly,
            &models,
            AgentUser::Current,
            session,
            workers,
        ),
        Input::AsStaffer(input) => match *input {
            Input::Agent {
                task,
                access,
                model,
            } => start_agents(
                task,
                access,
                &model.into_iter().collect::<Vec<_>>(),
                AgentUser::Staffer,
                session,
                workers,
            ),
            Input::Agents { task, models } => start_agents(
                task,
                AgentAccess::ReadOnly,
                &models,
                AgentUser::Staffer,
                session,
                workers,
            ),
            _ => unreachable!(),
        },
        Input::Url(url) => start_http(url, false, session, workers),
        Input::Head(url) => start_http(url, true, session, workers),
        Input::UnavailableTarget { name, access } => {
            eprintln!("euka: target '{name}{}' is not available", access.suffix());
            session.last_status = 1;
        }
        Input::Reserved(input) => {
            eprintln!("euka: reserved syntax is not available yet: {input}");
            session.last_status = 1;
        }
    }
    let _ = io::stdout().flush();
    None
}

fn run_agent(
    model: &str,
    task: &str,
    cwd: &Path,
    context: &str,
    references: &str,
    answer_chars: usize,
    access: AgentAccess,
    user: AgentUser,
) -> Result<String, String> {
    match model {
        "fm" => fm::run(task, cwd, context, references, access),
        "claude" => claude::run(task, cwd, context, answer_chars, None, access, user),
        "fable" | "opus" | "sonnet" | "haiku" => {
            claude::run(task, cwd, context, answer_chars, Some(model), access, user)
        }
        "codex" => codex::run(task, cwd, context, None, access, user),
        "sol" => codex::run(task, cwd, context, Some("gpt-6-sol"), access, user),
        "luna" => codex::run(task, cwd, context, Some("gpt-6-luna"), access, user),
        "terra" => codex::run(task, cwd, context, Some("gpt-5.6-terra"), access, user),
        "pi" => pi::run(task, cwd, context, access, user),
        _ => unreachable!(),
    }
}

fn resolve_inline(
    command: &str,
    session: &mut Session,
    workers: &mut Workers,
) -> Result<String, String> {
    let Some(parts) = inline::parse(command)? else {
        return Ok(command.to_owned());
    };
    let mut resolved = String::new();
    for part in parts {
        match part {
            inline::Part::Literal(text) => resolved.push_str(text),
            inline::Part::Agent { model, task } => {
                let references = ready_references(task, session, workers)?;
                let id = workers.next_result_id();
                let mut context =
                    session_context_filtered(session, &references.ids, None, &session.cwd);
                context.push_str(&format!(
                    "request ({model}#{id}, read-only, current user): {task}\n"
                ));
                context.push_str(&references.text);
                session.events.push(Event::AgentRequest {
                    id,
                    input: task.to_owned(),
                    access: AgentAccess::ReadOnly,
                    model: Some(model.to_owned()),
                });
                println!("{}", render_agent(model, id, AgentDisplay::Working));
                let _ = io::stdout().flush();
                let answer_chars = terminal::columns()
                    .saturating_sub(format!("{model}#{id}: ").chars().count() + 1)
                    .max(1);
                let answer = run_agent(
                    model,
                    task,
                    &session.cwd,
                    &context,
                    &references.text,
                    answer_chars,
                    AgentAccess::ReadOnly,
                    AgentUser::Current,
                )
                .map_err(|error| format!("{model}#{id}: {error}"))?;
                let quoted = inline::quote_argument(&answer)?;
                session.events.push(Event::AgentResponse {
                    id,
                    model: model.to_owned(),
                    text: answer.clone(),
                });
                println!(
                    "{}",
                    render_agent_output(model, id, AgentDisplay::Answer(&answer))
                );
                resolved.push_str(&quoted);
            }
        }
    }
    Ok(resolved)
}

fn start_agents(
    task: &str,
    access: AgentAccess,
    models: &[&str],
    user: AgentUser,
    session: &mut Session,
    workers: &mut Workers,
) {
    if task.is_empty()
        || models.is_empty()
        || !models.iter().all(|model| session::is_agent_model(model))
    {
        eprintln!("euka: agent unavailable or task is empty");
        session.last_status = 1;
        return;
    }
    if user == AgentUser::Staffer && models.contains(&"fm") {
        eprintln!("euka: fm cannot run as staffer");
        session.last_status = 1;
        return;
    }
    let references = match resolve_references(task, session, workers) {
        Ok(references) => references,
        Err(error) => {
            eprintln!("euka: {error}");
            session.last_status = 1;
            return;
        }
    };
    for &model in models {
        let id = workers.next_result_id();
        workers.active_agents += 1;
        session.events.push(Event::AgentRequest {
            id,
            input: task.into(),
            access,
            model: Some(model.to_owned()),
        });
        let answer_chars = terminal::columns()
            .saturating_sub(format!("{model}#{id}: ").chars().count() + 1)
            .max(1);
        let request = QueuedAgent {
            id,
            model: model.to_owned(),
            task: task.to_owned(),
            cwd: session.cwd.clone(),
            answer_chars,
            access,
            user,
        };
        let status = match &references {
            ReferenceResolution::Ready(references) => {
                launch_agent(request, references.clone(), session, workers);
                render_agent(model, id, AgentDisplay::Working)
            }
            ReferenceResolution::Pending(ids) => {
                let waiting_for = ids
                    .iter()
                    .map(|id| format!("#{id}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let status = format!("{model}#{id}: waiting for {waiting_for}");
                workers.pending_agents.insert(id, status.clone());
                workers.queued_agents.insert(id, request);
                status
            }
        };
        if !workers.interactive {
            println!("{status}");
        }
    }
    session.last_status = 0;
}

fn launch_agent(
    request: QueuedAgent,
    references: ResolvedReferences,
    session: &Session,
    workers: &mut Workers,
) {
    let context = agent_context(session, &request, &references);
    let QueuedAgent {
        id,
        model,
        task,
        cwd,
        answer_chars,
        access,
        user,
    } = request;
    let references = references.text;
    workers
        .pending_agents
        .insert(id, render_agent(&model, id, AgentDisplay::Working));
    let tx = workers.worker_tx.clone();
    std::thread::spawn(move || {
        let result = run_agent(
            &model,
            &task,
            &cwd,
            &context,
            &references,
            answer_chars,
            access,
            user,
        );
        let _ = tx.send(WorkerUpdate::Agent(AgentUpdate {
            id,
            model,
            access,
            result,
        }));
    });
}

fn agent_context(
    session: &Session,
    request: &QueuedAgent,
    references: &ResolvedReferences,
) -> String {
    let mut context =
        session_context_filtered(session, &references.ids, Some(request.id), &request.cwd);
    let username = match request.user {
        AgentUser::Current => "current user",
        AgentUser::Staffer => "staffer",
    };
    context.push_str(&format!(
        "request ({}#{}, {}, {username}): {}\n",
        request.model,
        request.id,
        request.access.description(),
        request.task
    ));
    context.push_str(&references.text);
    context
}

fn start_live_command(
    program: &'static str,
    command: &str,
    session: &mut Session,
    workers: &mut Workers,
) {
    let command = command.trim();
    if command.is_empty() {
        eprintln!("euka: + {program}?: expected a command");
        session.last_status = 1;
        return;
    }
    start_live(
        LiveTarget::Shell {
            program,
            command: command.to_owned(),
        },
        session,
        workers,
    );
}

fn start_live_head(url: &str, session: &mut Session, workers: &mut Workers) {
    if let Err(error) = http::origin(url) {
        eprintln!("euka: {error}");
        session.last_status = 1;
        return;
    }
    start_live(
        LiveTarget::Head {
            url: url.to_owned(),
        },
        session,
        workers,
    );
}

fn start_live_feed(
    url: &str,
    selection: FeedSelection,
    session: &mut Session,
    workers: &mut Workers,
) {
    if let Err(error) = http::origin(url) {
        eprintln!("euka: {error}");
        session.last_status = 1;
        return;
    }
    start_live(
        LiveTarget::Feed {
            url: url.to_owned(),
            selection,
        },
        session,
        workers,
    );
}

fn start_live(target: LiveTarget, session: &mut Session, workers: &mut Workers) {
    if let Some(existing) = session
        .live_commands
        .iter()
        .find(|live| live.cwd == session.cwd && live.target == target)
    {
        println!(
            "[watch #{}] already watching {}; enter + to refresh",
            existing.id,
            existing.label()
        );
        session.last_status = 0;
        return;
    }
    let id = workers.next_result_id();
    let cwd = session.cwd.clone();
    session.live_commands.push(LiveCommand {
        id,
        cwd: cwd.clone(),
        target: target.clone(),
        output: String::new(),
        error: None,
        last_run: None,
        last_refresh: Instant::now(),
        revisions: Vec::new(),
    });
    println!(
        "[watch #{id} loading] {}",
        session.live_commands.last().unwrap().label()
    );
    workers.active_live += 1;
    let tx = workers.live_tx.clone();
    std::thread::spawn(move || {
        let client = http::agent();
        let started_at = Instant::now();
        if tx
            .send(LiveUpdate {
                id,
                started_at,
                result: run_live_target(&target, &cwd, &client),
            })
            .is_err()
        {
            return;
        }
    });
    session.last_status = 0;
}

fn run_live_target(
    target: &LiveTarget,
    cwd: &Path,
    client: &ureq::Agent,
) -> Result<String, String> {
    match target {
        LiveTarget::Shell { program, command } => live::run(program, command, cwd),
        LiveTarget::Head { url } => http::head(client, url),
        LiveTarget::Feed { url, selection } => {
            let resource = http::load(client, url)?;
            if *selection == FeedSelection::ContentType
                && !feed::is_atom_content_type(&resource.content_type)
            {
                return Err(format!("URL is not an Atom feed (Content-Type: {}); use + FEED URL to parse an XML feed explicitly", resource.content_type));
            }
            feed::render(&resource)
        }
    }
}

fn show_live_diff(session: &mut Session, workers: &mut Workers, mode: DiffMode) {
    if session.live_commands.is_empty() {
        println!(
            "[+] no live values; register one with + bash? command, + HEAD URL, or + FEED URL"
        );
        return;
    }
    let mut failed = false;
    let mut showed_lines = false;
    let client = http::agent();
    for live in &mut session.live_commands {
        let previous = live.last_run.clone();
        let result = run_live_target(&live.target, &live.cwd, &client);
        failed |= result.is_err();
        let _ = update_live_command(live, result, Instant::now());
        let current = live_current_text(live);
        if let Some(previous) = previous {
            let lines = if matches!(live.target, LiveTarget::Feed { .. }) {
                feed::diff(&previous, &current, mode)
            } else {
                diff_lines(&previous, &current, mode)
            };
            if !lines.is_empty() {
                showed_lines = true;
                println!(
                    "[watch {} {} in {}]\n{lines}",
                    watch_revision_label(live),
                    live.label(),
                    live.cwd.display()
                );
            }
        }
        live.last_run = Some(current);
    }
    if !showed_lines {
        match mode {
            DiffMode::Added => println!("[+] no added lines"),
            DiffMode::AddedAndRemoved => println!("[+-] no changed lines"),
        }
    }
    resume_queued_agents(session, workers);
    session.last_status = if failed { 1 } else { 0 };
}

fn live_current_text(live: &LiveCommand) -> String {
    match &live.error {
        Some(error) => format!("error: {error}"),
        None => live.output.clone(),
    }
}

fn diff_lines(previous: &str, current: &str, mode: DiffMode) -> String {
    let old: Vec<_> = previous.lines().collect();
    let new: Vec<_> = current.lines().collect();
    let mut common = vec![vec![0; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            common[i][j] = if old[i] == new[j] {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let mut lines = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            i += 1;
            j += 1;
        } else if i < old.len() && (j == new.len() || common[i + 1][j] >= common[i][j + 1]) {
            if mode == DiffMode::AddedAndRemoved {
                lines.push(format!("-{}", old[i]));
            }
            i += 1;
        } else {
            lines.push(format!("+{}", new[j]));
            j += 1;
        }
    }
    lines.join("\n")
}

fn start_http(url: &str, head: bool, session: &mut Session, workers: &mut Workers) {
    let origin = match http::origin(url) {
        Ok(origin) => origin,
        Err(error) => {
            eprintln!("euka: {error}");
            session.last_status = 1;
            return;
        }
    };
    let id = workers.next_result_id();
    let sender = workers.hosts.entry(origin).or_insert_with(|| {
        let (sender, receiver) = mpsc::channel::<HttpRequest>();
        let updates = workers.worker_tx.clone();
        std::thread::spawn(move || {
            let client = http::agent();
            while let Ok(request) = receiver.recv() {
                let result = if request.head {
                    http::head(&client, &request.url).map(|output| HttpResult::Head {
                        url: request.url,
                        output,
                    })
                } else {
                    http::load(&client, &request.url).map(HttpResult::Resource)
                };
                if updates
                    .send(WorkerUpdate::Http(HttpUpdate {
                        id: request.id,
                        result,
                    }))
                    .is_err()
                {
                    break;
                }
            }
        });
        sender
    });
    if sender
        .send(HttpRequest {
            id,
            url: url.to_owned(),
            head,
        })
        .is_err()
    {
        eprintln!("euka: URL worker stopped");
        session.last_status = 1;
    } else {
        workers.active_http += 1;
        session.last_status = 0;
        if !head {
            println!("[url #{id} loading] {url}");
        }
    }
}

fn print_help() {
    println!("Euka runs ordinary commands directly as a shell. Enter ? for this help.");
    println!("Shell:");
    println!("  git status, cargo test, vim file   Run directly in the foreground");
    println!("  cd, export, unset, exit; NAME=value are built-ins");
    println!("  reset                Clear session, history, watches, and request IDs");
    println!("  bash! command       Run a command through Bash as your user");
    println!("  zsh! command        Run a command through Zsh as your user");
    println!("  bash? command       Run Bash as staffer (asks for sudo authorization)");
    println!("  zsh? command        Run Zsh as staffer (asks for sudo authorization)");
    println!("Live values:");
    println!("  + bash? command     Run and refresh a Bash command as staffer");
    println!("  + zsh? command      Run and refresh a Zsh command as staffer");
    println!("  + HEAD https://...  Check headers now and again when you enter +");
    println!("  + FEED https://...  Watch an Atom feed; refresh when you enter +");
    println!("  + https://...       Watch an Atom URL if its Content-Type identifies a feed");
    println!("  +                   Show added lines from all live values");
    println!("  +-                  Show added and removed lines from all live values");
    println!("  #3, #3.2            Show the latest watch result or revision 2 of watch #3");
    println!("Shared context:");
    println!("  https://...          Load a text resource into session context (background)");
    println!("  HEAD https://...     Show response headers and time to receive them");
    println!("  # comment           Keep context for later agent requests");
    println!("  - [ ] todo          Add a session todo");
    println!("Agent:");
    println!("  claude? task        Ask Claude Code to inspect the project (background)");
    println!("  fable? task         Ask Claude Code with Fable (background)");
    println!("  opus? task          Ask Claude Code with Opus (background)");
    println!("  sonnet? task        Ask Claude Code with Sonnet (background)");
    println!("  haiku? task         Ask Claude Code with Haiku (background)");
    println!("  codex? task         Ask Codex to inspect the project (background)");
    println!("  sol? task           Ask GPT-6 Sol through Codex (background)");
    println!("  luna? task          Ask GPT-6 Luna through Codex (background)");
    println!("  terra? task         Ask GPT-5.6 Terra through Codex (background)");
    println!("  pi? task            Ask Pi to inspect the project (background)");
    println!("  fm? task            Ask Apple Foundation Models to investigate (background)");
    println!("  fm! task            Ask Apple Foundation Models to change files (background)");
    println!("  name! task          Ask a named agent to change files as your user");
    println!("  opus/luna? task     Ask both agents the same task (background)");
    println!("  @staffer opus? task Ask a CLI agent as staffer (requires sudo access)");
    println!("  fm?                 Enter fm read-only mode; ., exit, or Ctrl-C returns");
    println!("  git commit -m luna?(Suggest a message)  Use one agent reply as an argument");
    println!("  echo $(date)        Run shell command substitution via Bash or Zsh");
    println!("  #3, codex#3         Show a numbered result or full agent reply");
    println!("  claude? agree? #3   Include a numbered result; queue if an agent is still working");
    println!("  ? task, ! task, and other + forms are not available yet");
    println!("  Unknown name? or name! targets report an Euka error");
    #[cfg(target_os = "macos")]
    {
        println!();
        println!("# macOS reader account for bash? and zsh?:");
        println!("  sudo sysadminctl -addUser staffer -fullName \"Staff reader\" -GID 20 -shell /usr/bin/false");
        println!("  id staffer  # Check that gid=20(staff) and admin is absent");
        println!("  # Give staff read/search access to the current tree:");
        println!("    chgrp -R staff ./");
        println!("    chmod -R 750 ./    # owner 7, staff 5, others 0");
        println!("  # This also makes regular files executable; check any ACLs.");
        println!("  # To skip sudo password prompts, run sudo visudo and add at the end:");
        if let Some(username) = current_username() {
            println!("    {username} ALL=(staffer) NOPASSWD: ALL");
        } else {
            println!("    <your-username> ALL=(staffer) NOPASSWD: ALL");
            println!("    # Replace <your-username> with the output of whoami.");
        }
        println!("  # This allows your user to run any command as staffer without a password.");
        println!("  # staffer can still write wherever its Unix permissions allow.");
    }
}

#[cfg(target_os = "macos")]
fn current_username() -> Option<String> {
    let output = Command::new("/usr/bin/id").arg("-un").output().ok()?;
    let username = std::str::from_utf8(&output.stdout).ok()?.trim();
    if output.status.success()
        && !username.is_empty()
        && !username.starts_with('-')
        && username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        Some(username.to_owned())
    } else {
        None
    }
}

#[cfg(test)]
fn session_context(session: &Session) -> String {
    session_context_filtered(session, &BTreeSet::new(), None, &session.cwd)
}

fn session_context_filtered(
    session: &Session,
    referenced_ids: &BTreeSet<usize>,
    current_request_id: Option<usize>,
    cwd: &Path,
) -> String {
    let mut context = format!("cwd: {}\n", cwd.display());
    for live in &session.live_commands {
        if referenced_ids.contains(&live.id) {
            continue;
        }
        context.push_str(&format!(
            "watch {} {} in {}:\n{}\n",
            watch_revision_label(live),
            live.label(),
            live.cwd.display(),
            live_current_text(live)
        ));
    }
    let recent_events: Vec<_> = session
        .events
        .iter()
        .rev()
        .filter(|event| match event {
            Event::AgentRequest { id, .. } => Some(*id) != current_request_id,
            Event::AgentResponse { id, .. } => !referenced_ids.contains(id),
            Event::Resource { id, .. } | Event::HttpHead { id, .. } => !referenced_ids.contains(id),
            _ => true,
        })
        .take(20)
        .collect();
    for event in recent_events.into_iter().rev() {
        match event {
            Event::Command { input, status } => {
                context.push_str(&format!("command: {input} (exit {status})\n"))
            }
            Event::Comment(text) => context.push_str(&format!("comment: {text}\n")),
            Event::AgentRequest {
                id,
                input,
                access,
                model,
            } => context.push_str(&format!(
                "request ({}#{id}, {}): {input}\n",
                model.as_deref().unwrap_or("default"),
                access.description()
            )),
            Event::AgentResponse { id, model, text } => {
                context.push_str(&format!("response ({model}#{id}): {text}\n"))
            }
            Event::Resource { resource, .. } => {
                context.push_str(&format!(
                    "resource: {} ({})\n",
                    resource.url, resource.content_type
                ));
                context.extend(resource.content.chars().take(8_000));
                context.push('\n');
            }
            Event::HttpHead { url, output, .. } => {
                context.push_str(&format!("HEAD {url}:\n{output}\n"));
            }
        }
    }
    for todo in &session.todos {
        context.push_str(&format!("todo: {todo}\n"));
    }
    if context.len() > 12_000 {
        context = fm::tail(&context, 12_000);
    }
    context
}

fn agent_response<'a>(
    session: &'a Session,
    model: &str,
    id: usize,
    workers: &Workers,
) -> Result<&'a str, String> {
    if let Some(text) = session.events.iter().find_map(|event| match event {
        Event::AgentResponse {
            id: response_id,
            model: response_model,
            text,
        } if *response_id == id && response_model == model => Some(text.as_str()),
        _ => None,
    }) {
        return Ok(text);
    }
    let reference = format!("{model}#{id}");
    let requested = session.events.iter().any(|event| {
        matches!(event,
            Event::AgentRequest { id: request_id, model: Some(request_model), .. }
                if *request_id == id && request_model == model
        )
    });
    if requested && workers.pending_agents.contains_key(&id) {
        Err(format!("{reference} is still working"))
    } else if requested {
        Err(format!("{reference} has no reply"))
    } else {
        Err(format!("no reply found for {reference}"))
    }
}

fn agent_response_by_id<'a>(
    session: &'a Session,
    id: usize,
    workers: &Workers,
) -> Result<(&'a str, &'a str), String> {
    let model =
        agent_model_by_id(session, id).ok_or_else(|| format!("no reply found for #{id}"))?;
    let text = agent_response(session, model, id, workers)?;
    Ok((model, text))
}

fn agent_model_by_id(session: &Session, id: usize) -> Option<&str> {
    session.events.iter().find_map(|event| match event {
        Event::AgentRequest {
            id: request_id,
            model: Some(model),
            ..
        } if *request_id == id => Some(model.as_str()),
        _ => None,
    })
}

fn http_response_by_id(session: &Session, id: usize) -> Option<String> {
    session.events.iter().find_map(|event| match event {
        Event::HttpHead {
            id: event_id,
            url,
            output,
        } if *event_id == id => Some(format!("HEAD {url}:\n{output}")),
        Event::Resource {
            id: event_id,
            resource,
        } if *event_id == id => Some(format!(
            "{} ({}):\n{}",
            resource.url, resource.content_type, resource.content
        )),
        _ => None,
    })
}

fn watch_response_by_id(session: &Session, id: usize, revision: Option<usize>) -> Option<String> {
    let watch = session.live_commands.iter().find(|watch| watch.id == id)?;
    let revision = revision.unwrap_or(watch.revisions.len());
    let output = watch.revisions.get(revision.checked_sub(1)?)?;
    Some(format!(
        "watch #{id}.{revision} {} in {}:\n{output}",
        watch.label(),
        watch.cwd.display()
    ))
}

fn watch_revision_label(watch: &LiveCommand) -> String {
    if watch.revisions.is_empty() {
        format!("#{}", watch.id)
    } else {
        format!("#{}.{}", watch.id, watch.revisions.len())
    }
}

fn watch_reference_error(watch: &LiveCommand, revision: Option<usize>) -> String {
    match (revision, watch.revisions.len(), &watch.error) {
        (None | Some(1), 0, None) => format!("watch #{} is still loading", watch.id),
        (_, 0, Some(error)) => format!("watch #{} has no result: {error}", watch.id),
        (Some(revision), _, _) => format!("watch #{} has no revision {revision}", watch.id),
        (None, _, _) => unreachable!(),
    }
}

fn ready_references(
    task: &str,
    session: &Session,
    workers: &Workers,
) -> Result<ResolvedReferences, String> {
    match resolve_references(task, session, workers)? {
        ReferenceResolution::Ready(references) => Ok(references),
        ReferenceResolution::Pending(ids) => {
            let id = ids[0];
            if let Some(model) = agent_model_by_id(session, id) {
                Err(format!("{model}#{id} is still working"))
            } else {
                Err(format!("watch #{id} is still loading"))
            }
        }
    }
}

fn resolve_references(
    task: &str,
    session: &Session,
    workers: &Workers,
) -> Result<ReferenceResolution, String> {
    let mut context = String::new();
    let mut seen = BTreeSet::new();
    let mut seen_watch_revisions = BTreeSet::new();
    let mut pending = BTreeSet::new();
    for raw_token in task.split(|character: char| {
        !(character.is_ascii_alphanumeric()
            || character == '#'
            || character == '.'
            || character == '_'
            || character == '-')
    }) {
        let token = raw_token.trim_end_matches('.');
        let reference = if let Some((id, revision)) = session::watch_revision_reference(token) {
            let watch = session
                .live_commands
                .iter()
                .find(|watch| watch.id == id)
                .ok_or_else(|| format!("no watch found for #{id}"))?;
            if let Some(text) = watch_response_by_id(session, id, Some(revision)) {
                if seen_watch_revisions.insert((id, revision)) {
                    seen.insert(id);
                    context.push_str(&format!("\nReferenced {text}\n"));
                }
            } else if watch.revisions.is_empty() && watch.error.is_none() && revision == 1 {
                pending.insert(id);
            } else {
                return Err(watch_reference_error(watch, Some(revision)));
            }
            None
        } else if let Some((model, id)) = session::agent_reference(token) {
            if agent_model_by_id(session, id) != Some(model) {
                return Err(format!("no reply found for {model}#{id}"));
            }
            Some((model, id))
        } else if let Some(id) = session::agent_number_reference(token) {
            if let Some(model) = agent_model_by_id(session, id) {
                Some((model, id))
            } else if let Some(watch) = session.live_commands.iter().find(|watch| watch.id == id) {
                if let Some(text) = watch_response_by_id(session, id, None) {
                    let revision = watch.revisions.len();
                    if seen_watch_revisions.insert((id, revision)) {
                        seen.insert(id);
                        context.push_str(&format!("\nReferenced {text}\n"));
                    }
                } else if watch.error.is_none() {
                    pending.insert(id);
                } else {
                    return Err(watch_reference_error(watch, None));
                }
                None
            } else if let Some(text) = http_response_by_id(session, id) {
                if seen.insert(id) {
                    context.push_str(&format!("\nReferenced HTTP result #{id}:\n{text}\n"));
                }
                None
            } else {
                return Err(format!("no reply found for #{id}"));
            }
        } else {
            None
        };
        if let Some((model, id)) = reference {
            if seen.insert(id) {
                match agent_response(session, model, id, workers) {
                    Ok(text) => {
                        context.push_str(&format!("\nReferenced reply {model}#{id}:\n{text}\n"));
                    }
                    Err(_) if workers.pending_agents.contains_key(&id) => {
                        pending.insert(id);
                    }
                    Err(error) => return Err(error),
                }
            }
        }
    }
    if pending.is_empty() {
        Ok(ReferenceResolution::Ready(ResolvedReferences {
            text: context,
            ids: seen,
        }))
    } else {
        Ok(ReferenceResolution::Pending(pending.into_iter().collect()))
    }
}

fn drain_updates(workers: &mut Workers, session: &mut Session) -> Vec<String> {
    let mut lines = Vec::new();
    while let Ok(update) = workers.worker_rx.try_recv() {
        lines.push(format_worker_update(update, session, workers));
    }
    let mut watch_updated = false;
    while let Ok(update) = workers.live_rx.try_recv() {
        workers.active_live = workers.active_live.saturating_sub(1);
        if let Some(live) = session
            .live_commands
            .iter_mut()
            .find(|live| live.id == update.id)
        {
            if update.started_at >= live.last_refresh {
                watch_updated = true;
                if let Some(line) = update_live_command(live, update.result, Instant::now()) {
                    lines.push(line);
                }
            }
        }
    }
    if watch_updated {
        resume_queued_agents(session, workers);
    }
    lines
}

fn update_live_command(
    live: &mut LiveCommand,
    result: Result<String, String>,
    checked_at: Instant,
) -> Option<String> {
    live.last_refresh = checked_at;
    match result {
        Ok(output) => {
            let first_result = live.revisions.is_empty();
            let changed = first_result || output != live.output || live.error.take().is_some();
            if live.revisions.last() != Some(&output) {
                live.revisions.push(output.clone());
            }
            live.output = output.clone();
            if live.last_run.is_none() {
                live.last_run = Some(output.clone());
            }
            changed.then(|| {
                format!(
                    "[watch #{}.{} {}] {output}",
                    live.id,
                    live.revisions.len(),
                    live.label()
                )
            })
        }
        Err(error) if live.error.as_deref() != Some(&error) => {
            live.error = Some(error.clone());
            if live.last_run.is_none() {
                live.last_run = Some(format!("error: {error}"));
            }
            Some(format!(
                "[watch #{} {} error] {error}",
                live.id,
                live.label()
            ))
        }
        Err(_) => None,
    }
}

fn format_worker_update(
    update: WorkerUpdate,
    session: &mut Session,
    workers: &mut Workers,
) -> String {
    match update {
        WorkerUpdate::Agent(update) => format_update(update, session, workers),
        WorkerUpdate::Http(update) => {
            workers.active_http = workers.active_http.saturating_sub(1);
            match update.result {
                Ok(HttpResult::Resource(resource)) => {
                    let message = format!(
                        "[url #{}] loaded {} bytes from {} ({})",
                        update.id,
                        resource.content.len(),
                        resource.url,
                        resource.content_type
                    );
                    session.events.push(Event::Resource {
                        id: update.id,
                        resource,
                    });
                    message
                }
                Ok(HttpResult::Head { url, output }) => {
                    let message = format!("[head #{}] {url}\n{output}", update.id);
                    session.events.push(Event::HttpHead {
                        id: update.id,
                        url,
                        output,
                    });
                    message
                }
                Err(error) => {
                    session.last_status = 1;
                    format!("[url #{} error] {error}", update.id)
                }
            }
        }
    }
}

fn format_update(update: AgentUpdate, session: &mut Session, workers: &mut Workers) -> String {
    workers.active_agents = workers.active_agents.saturating_sub(1);
    workers.pending_agents.remove(&update.id);
    let model = update.model;
    let line = match update.result {
        Ok(text) => {
            session.events.push(Event::AgentResponse {
                id: update.id,
                model: model.clone(),
                text: text.clone(),
            });
            let display = match update.access {
                AgentAccess::ReadOnly => AgentDisplay::Answer(&text),
                AgentAccess::ReadWrite => AgentDisplay::FullAnswer(&text),
            };
            render_agent_output(&model, update.id, display)
        }
        Err(error) => render_agent_output(&model, update.id, AgentDisplay::Error(&error)),
    };
    resume_queued_agents(session, workers);
    line
}

fn resume_queued_agents(session: &Session, workers: &mut Workers) {
    let ids: Vec<usize> = workers.queued_agents.keys().copied().collect();
    for id in ids {
        let Some(request) = workers.queued_agents.get(&id) else {
            continue;
        };
        let resolution = resolve_references(&request.task, session, workers);
        match resolution {
            Ok(ReferenceResolution::Ready(references)) => {
                let request = workers.queued_agents.remove(&id).unwrap();
                launch_agent(request, references, session, workers);
            }
            Ok(ReferenceResolution::Pending(ids)) => {
                let status = ids
                    .iter()
                    .map(|id| format!("#{id}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                workers
                    .pending_agents
                    .insert(id, format!("{}#{id}: waiting for {status}", request.model));
            }
            Err(error) => {
                let request = workers.queued_agents.remove(&id).unwrap();
                let _ = workers.worker_tx.send(WorkerUpdate::Agent(AgentUpdate {
                    id,
                    model: request.model,
                    access: request.access,
                    result: Err(format!("referenced reply unavailable: {error}")),
                }));
            }
        }
    }
}

fn one_line(text: &str, max_chars: usize) -> String {
    let collapsed = text
        .chars()
        .filter(|character| !character.is_control() || character.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.chars().count() <= max_chars {
        return collapsed;
    }
    if max_chars == 0 {
        return String::new();
    }
    let mut summary = collapsed.chars().take(max_chars - 1).collect::<String>();
    summary.push('…');
    summary
}

#[cfg(test)]
mod tests {
    use super::{
        agent_context, agent_response, agent_response_by_id, diff_lines, format_update,
        highlight_answer, one_line, prompt, ready_references, render_agent, reset_state,
        resolve_references, session_context, shortened_path, start_agents, AgentDisplay,
        AgentUpdate, ReferenceResolution, Workers,
    };
    use crate::agent_cli::AgentUser;
    use crate::session::{AgentAccess, DiffMode, Event, PromptMode, Session};
    use crate::terminal::Terminal;
    use std::path::Path;

    #[test]
    fn reset_restarts_counters_and_discards_old_worker_updates() {
        let mut session = Session::new().unwrap();
        session.events.push(Event::Comment("old context".into()));
        session.todos.push("old todo".into());
        session.mode = PromptMode::FmReadOnly;
        session.last_status = 7;
        let mut workers = Workers::new(false);
        workers.next_agent_id = 4;
        workers.next_http_id = 3;
        let old_sender = workers.worker_tx.clone();
        let mut terminal = Terminal::new();

        reset_state(&mut session, &mut workers, &mut terminal).unwrap();

        assert_eq!(workers.next_agent_id, 1);
        assert_eq!(workers.next_http_id, 1);
        assert!(session.events.is_empty());
        assert!(session.todos.is_empty());
        assert!(session.live_commands.is_empty());
        assert!(session.mode == PromptMode::Shell);
        assert_eq!(session.last_status, 0);
        assert!(old_sender
            .send(super::WorkerUpdate::Agent(super::AgentUpdate {
                id: 1,
                model: "codex".into(),
                access: AgentAccess::ReadOnly,
                result: Ok("stale".into()),
            }))
            .is_err());
    }

    #[test]
    fn prompt_path_uses_fish_style_abbreviation() {
        let home = Path::new("/Users/pgwsmith");
        assert_eq!(shortened_path(home, Some(home)), "~");
        assert_eq!(
            shortened_path(Path::new("/Users/pgwsmith/Collected/euka"), Some(home)),
            "~/C/euka"
        );
        assert_eq!(
            shortened_path(Path::new("/Users/pgwsmith/.config/fish"), Some(home)),
            "~/.c/fish"
        );
        assert_eq!(
            shortened_path(Path::new("/tmp/banana/sausage"), Some(home)),
            "/t/b/sausage"
        );
        assert_eq!(shortened_path(Path::new("/"), Some(home)), "/");
        assert_eq!(
            shortened_path(Path::new("/Users/pgwsmith2/euka"), Some(home)),
            "/U/p/euka"
        );
        assert_eq!(
            prompt(PromptMode::Shell, Path::new("/tmp/example"), true, false),
            "/tmp/example> "
        );
        assert_eq!(
            prompt(
                PromptMode::FmReadOnly,
                Path::new("/tmp/example"),
                true,
                false
            ),
            "fm? /tmp/example> "
        );
        assert_eq!(
            prompt(PromptMode::Shell, Path::new("/tmp/example"), true, true),
            "\x1b[1;36m/tmp/example>\x1b[0m "
        );
    }

    #[test]
    fn every_agent_uses_the_same_reply_label() {
        for model in ["fm", "claude", "codex", "pi"] {
            assert_eq!(
                render_agent(model, 4, AgentDisplay::Working),
                format!("{model}#4: …")
            );
            assert_eq!(
                render_agent(model, 4, AgentDisplay::Answer("done")),
                format!("{model}#4: done")
            );
            assert_eq!(
                render_agent(model, 4, AgentDisplay::Error("failed")),
                format!("{model}#4: error: failed")
            );
            assert_eq!(
                render_agent(model, 4, AgentDisplay::FullAnswer("line one\nline two")),
                format!("{model}#4: line one\nline two")
            );
        }
    }

    #[test]
    fn named_reply_references_include_full_text() {
        let mut session = Session::new().unwrap();
        let workers = Workers::new(false);
        session.events.push(Event::AgentRequest {
            id: 3,
            input: "inspect".into(),
            access: AgentAccess::ReadOnly,
            model: Some("codex".into()),
        });
        session.events.push(Event::AgentResponse {
            id: 3,
            model: "codex".into(),
            text: "the full\nmultiline finding".into(),
        });
        for _ in 0..25 {
            session.events.push(Event::Comment("later context".into()));
        }
        assert!(!session_context(&session).contains("multiline finding"));
        assert_eq!(
            ready_references("agree? codex#3, codex#3", &session, &workers)
                .unwrap()
                .text,
            "\nReferenced reply codex#3:\nthe full\nmultiline finding\n"
        );
        assert_eq!(
            ready_references("agree? #3, codex#3", &session, &workers)
                .unwrap()
                .text,
            "\nReferenced reply codex#3:\nthe full\nmultiline finding\n"
        );
        assert_eq!(
            agent_response_by_id(&session, 3, &workers).unwrap(),
            ("codex", "the full\nmultiline finding")
        );
        assert_eq!(
            agent_response(&session, "codex", 3, &workers).unwrap(),
            "the full\nmultiline finding"
        );
        assert!(agent_response(&session, "claude", 3, &workers).is_err());
    }

    #[test]
    fn pending_reply_reference_reports_pending() {
        let mut session = Session::new().unwrap();
        let mut workers = Workers::new(false);
        session.events.push(Event::AgentRequest {
            id: 3,
            input: "inspect".into(),
            access: AgentAccess::ReadOnly,
            model: Some("codex".into()),
        });
        workers.pending_agents.insert(3, "codex#3: …".into());
        assert_eq!(
            ready_references("agree? codex#3", &session, &workers).unwrap_err(),
            "codex#3 is still working"
        );
        assert_eq!(
            ready_references("agree? #3", &session, &workers).unwrap_err(),
            "codex#3 is still working"
        );
        assert_eq!(
            agent_response_by_id(&session, 99, &workers).unwrap_err(),
            "no reply found for #99"
        );
        assert_eq!(
            ready_references("agree? claude#3", &session, &workers).unwrap_err(),
            "no reply found for claude#3"
        );
    }

    #[test]
    fn pending_reply_queues_followup_and_failed_reply_cancels_it() {
        let mut session = Session::new().unwrap();
        let mut workers = Workers::new(false);
        workers.next_agent_id = 3;
        workers.active_agents = 1;
        workers.pending_agents.insert(2, "sol#2: …".into());
        session.events.push(Event::AgentRequest {
            id: 2,
            input: "inspect".into(),
            access: AgentAccess::ReadOnly,
            model: Some("sol".into()),
        });

        start_agents(
            "summarize #2",
            AgentAccess::ReadOnly,
            &["luna"],
            AgentUser::Current,
            &mut session,
            &mut workers,
        );
        assert_eq!(
            workers.pending_agents.get(&3).unwrap(),
            "luna#3: waiting for #2"
        );
        assert!(workers.queued_agents.contains_key(&3));
        assert_eq!(workers.active_agents, 2);

        format_update(
            AgentUpdate {
                id: 2,
                model: "sol".into(),
                access: AgentAccess::ReadOnly,
                result: Err("failed".into()),
            },
            &mut session,
            &mut workers,
        );
        assert!(!workers.queued_agents.contains_key(&3));
        let update = workers.worker_rx.try_recv().unwrap();
        let line = super::format_worker_update(update, &mut session, &mut workers);
        assert!(line.contains("referenced reply unavailable: sol#2 has no reply"));
        assert!(workers.pending_agents.is_empty());
        assert_eq!(workers.active_agents, 0);
    }

    #[test]
    fn queued_request_waits_for_every_referenced_reply() {
        let mut session = Session::new().unwrap();
        let mut workers = Workers::new(false);
        for id in [1, 2] {
            session.events.push(Event::AgentRequest {
                id,
                input: "inspect".into(),
                access: AgentAccess::ReadOnly,
                model: Some("sol".into()),
            });
            workers.pending_agents.insert(id, format!("sol#{id}: …"));
        }
        let task = "compare #1 and #2";
        assert!(matches!(
            resolve_references(task, &session, &workers).unwrap(),
            ReferenceResolution::Pending(ids) if ids == [1, 2]
        ));
        workers.pending_agents.remove(&1);
        session.events.push(Event::AgentResponse {
            id: 1,
            model: "sol".into(),
            text: "first".into(),
        });
        assert!(matches!(
            resolve_references(task, &session, &workers).unwrap(),
            ReferenceResolution::Pending(ids) if ids == [2]
        ));
        workers.pending_agents.remove(&2);
        session.events.push(Event::AgentResponse {
            id: 2,
            model: "sol".into(),
            text: "second".into(),
        });
        assert!(matches!(
            resolve_references(task, &session, &workers).unwrap(),
            ReferenceResolution::Ready(references)
                if references.text.contains("Referenced reply sol#1:\nfirst")
                    && references.text.contains("Referenced reply sol#2:\nsecond")
        ));
    }

    #[test]
    fn queued_agent_builds_context_after_reply_and_includes_it_once() {
        let mut session = Session::new().unwrap();
        let mut workers = Workers::new(false);
        workers.next_agent_id = 3;
        session.events.push(Event::AgentRequest {
            id: 2,
            input: "inspect".into(),
            access: AgentAccess::ReadOnly,
            model: Some("sol".into()),
        });
        workers.pending_agents.insert(2, "sol#2: …".into());
        start_agents(
            "summarize #2",
            AgentAccess::ReadOnly,
            &["luna"],
            AgentUser::Current,
            &mut session,
            &mut workers,
        );
        let request_cwd = session.cwd.clone();
        session.cwd = Path::new("/tmp/changed-while-waiting").into();
        session
            .events
            .push(Event::Comment("added while waiting".into()));
        session.events.push(Event::AgentResponse {
            id: 2,
            model: "sol".into(),
            text: "unique reply".into(),
        });
        workers.pending_agents.remove(&2);
        let ReferenceResolution::Ready(references) =
            resolve_references("summarize #2", &session, &workers).unwrap()
        else {
            panic!("reference should be ready");
        };
        let context = agent_context(&session, &workers.queued_agents[&3], &references);
        assert!(context.starts_with(&format!("cwd: {}\n", request_cwd.display())));
        assert!(context.contains("comment: added while waiting"));
        assert_eq!(context.matches("unique reply").count(), 1);
        assert!(!context.contains("response (sol#2):"));
        assert_eq!(context.matches("request (luna#3").count(), 1);
    }

    #[test]
    fn agent_results_highlight_bold_bullets_and_code() {
        assert_eq!(
            highlight_answer("**Checks and additions**\n- I checked `src/terminal.rs`"),
            "\x1b[1;36mChecks and additions\x1b[0m\n\x1b[36m•\x1b[0m I checked \x1b[36msrc/terminal.rs\x1b[0m"
        );
    }

    #[test]
    fn claude_answers_fit_one_line() {
        assert_eq!(
            one_line(" first\n second\tthird ", 30),
            "first second third"
        );
        assert_eq!(one_line("abcdefghijk", 6), "abcde…");
    }

    #[test]
    fn live_diff_modes_show_only_requested_lines() {
        assert_eq!(
            diff_lines("## main\n?? old", "## main\n?? new", DiffMode::Added),
            "+?? new"
        );
        assert_eq!(
            diff_lines(
                "## main\n?? old",
                "## main\n?? new",
                DiffMode::AddedAndRemoved
            ),
            "-?? old\n+?? new"
        );
        assert_eq!(diff_lines("old", "", DiffMode::Added), "");
        assert_eq!(diff_lines("old", "", DiffMode::AddedAndRemoved), "-old");
    }

    #[test]
    fn completed_head_result_is_available_to_agent_references() {
        let mut session = Session::new().unwrap();
        session.events.push(Event::HttpHead {
            id: 2,
            url: "https://example.com/feed.atom".into(),
            output: "HTTP 200\ncontent-type: application/atom+xml".into(),
        });
        let workers = Workers::new(false);
        let references = ready_references("Summarize #2", &session, &workers).unwrap();
        assert!(references.text.contains("Referenced HTTP result #2:"));
        assert!(references.text.contains("HTTP 200"));
        assert!(super::http_response_by_id(&session, 2)
            .unwrap()
            .contains("feed.atom"));
        let context =
            super::session_context_filtered(&session, &references.ids, None, &session.cwd);
        assert!(!context.contains("HEAD https://example.com/feed.atom"));
    }

    #[test]
    fn agents_and_http_results_share_numbers() {
        let mut workers = Workers::new(false);
        assert_eq!(workers.next_result_id(), 1);
        assert_eq!(workers.next_result_id(), 2);
        assert_eq!(workers.next_agent_id, 3);
        assert_eq!(workers.next_http_id, 3);
    }

    #[test]
    fn watch_results_keep_numbered_revisions_and_support_references() {
        let mut session = Session::new().unwrap();
        session.live_commands.push(crate::session::LiveCommand {
            id: 1,
            cwd: session.cwd.clone(),
            target: crate::session::LiveTarget::Shell {
                program: "bash",
                command: "printf result".into(),
            },
            output: String::new(),
            error: None,
            last_run: None,
            last_refresh: std::time::Instant::now(),
            revisions: Vec::new(),
        });
        let workers = Workers::new(false);
        assert!(matches!(
            resolve_references("summarize #1", &session, &workers).unwrap(),
            ReferenceResolution::Pending(ids) if ids == [1]
        ));

        let watch = &mut session.live_commands[0];
        assert!(
            super::update_live_command(watch, Ok("first".into()), std::time::Instant::now())
                .unwrap()
                .contains("#1.1")
        );
        assert!(
            super::update_live_command(watch, Ok("first".into()), std::time::Instant::now())
                .is_none()
        );
        assert_eq!(watch.revisions.len(), 1);
        assert!(
            super::update_live_command(watch, Ok("second".into()), std::time::Instant::now())
                .unwrap()
                .contains("#1.2")
        );
        assert_eq!(watch.revisions.len(), 2);
        super::update_live_command(
            watch,
            Err("temporary failure".into()),
            std::time::Instant::now(),
        );
        assert_eq!(watch.revisions.len(), 2);

        assert!(super::watch_response_by_id(&session, 1, None)
            .unwrap()
            .contains("second"));
        assert!(super::watch_response_by_id(&session, 1, Some(1))
            .unwrap()
            .contains("first"));
        let ReferenceResolution::Ready(references) =
            resolve_references("compare #1.1 and #1.2 with #1.", &session, &workers).unwrap()
        else {
            panic!("watch results should be ready")
        };
        assert_eq!(references.text.matches("Referenced watch #1.1").count(), 1);
        assert_eq!(references.text.matches("Referenced watch #1.2").count(), 1);
        assert_eq!(references.ids.into_iter().collect::<Vec<_>>(), vec![1]);
    }
}
