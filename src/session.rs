use crate::shell::ShellExecution;
use std::path::PathBuf;
use std::time::Instant;

pub struct Session {
    pub cwd: PathBuf,
    pub events: Vec<Event>,
    pub todos: Vec<String>,
    pub last_status: i32,
    pub live_commands: Vec<LiveCommand>,
    pub mode: PromptMode,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PromptMode {
    Shell,
    FmReadOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffMode {
    Added,
    AddedAndRemoved,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FeedSelection {
    Explicit,
    ContentType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentAccess {
    ReadOnly,
    ReadWrite,
}

impl AgentAccess {
    pub fn suffix(self) -> char {
        match self {
            Self::ReadOnly => '?',
            Self::ReadWrite => '!',
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::ReadWrite => "read-write",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentModel {
    Fm,
    Claude,
    Fable,
    Opus,
    Sonnet,
    Haiku,
    Codex,
    Sol,
    Luna,
    Terra,
    Pi,
    Jsc,
}

impl AgentModel {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "fm" => Some(Self::Fm),
            "claude" => Some(Self::Claude),
            "fable" => Some(Self::Fable),
            "opus" | "Opus" => Some(Self::Opus),
            "sonnet" => Some(Self::Sonnet),
            "haiku" => Some(Self::Haiku),
            "codex" => Some(Self::Codex),
            "sol" => Some(Self::Sol),
            "luna" => Some(Self::Luna),
            "terra" => Some(Self::Terra),
            "pi" => Some(Self::Pi),
            "jsc" if cfg!(target_os = "macos") => Some(Self::Jsc),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Fm => "fm",
            Self::Claude => "claude",
            Self::Fable => "fable",
            Self::Opus => "opus",
            Self::Sonnet => "sonnet",
            Self::Haiku => "haiku",
            Self::Codex => "codex",
            Self::Sol => "sol",
            Self::Luna => "luna",
            Self::Terra => "terra",
            Self::Pi => "pi",
            Self::Jsc => "jsc",
        }
    }

    pub fn from_emoji(selector: &str) -> Option<Self> {
        match selector {
            "🎵" | "🎶" => Some(Self::Opus),
            "☀️" | "☀" => Some(Self::Sol),
            "🌙" => Some(Self::Luna),
            "🥧" => Some(Self::Pi),
            "🍎" => Some(Self::Fm),
            _ => None,
        }
    }
}

pub struct LiveCommand {
    pub id: usize,
    pub cwd: PathBuf,
    pub target: LiveTarget,
    pub output: String,
    pub error: Option<String>,
    pub last_run: Option<String>,
    pub last_refresh: Instant,
    pub revisions: Vec<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub enum LiveTarget {
    Shell {
        program: &'static str,
        command: String,
    },
    Head {
        url: String,
    },
    Feed {
        url: String,
        selection: FeedSelection,
    },
}

impl LiveCommand {
    pub fn label(&self) -> String {
        match &self.target {
            LiveTarget::Shell { program, command } => format!("+ {program}? {command}"),
            LiveTarget::Head { url } => format!("+ HEAD {url}"),
            LiveTarget::Feed { url, selection } => match selection {
                FeedSelection::Explicit => format!("+ FEED {url}"),
                FeedSelection::ContentType => format!("+ {url}"),
            },
        }
    }
}

pub struct Resource {
    pub url: String,
    pub content_type: String,
    pub content: String,
}

#[allow(dead_code)] // Events are retained for later agent context.
pub enum Event {
    Command {
        input: String,
        status: i32,
    },
    Comment(String),
    AgentRequest {
        id: usize,
        input: String,
        access: AgentAccess,
        model: Option<String>,
    },
    AgentResponse {
        id: usize,
        model: String,
        text: String,
    },
    Resource {
        id: usize,
        resource: Resource,
    },
    HttpHead {
        id: usize,
        url: String,
        output: String,
    },
}

pub enum Input<'a> {
    Help,
    Shell(&'a str),
    HostShell {
        program: &'static str,
        command: &'a str,
        execution: ShellExecution,
    },
    Comment(&'a str),
    ShowTodos,
    Todo(&'a str),
    LiveHostShell {
        program: &'static str,
        command: &'a str,
    },
    LiveHead(&'a str),
    LiveFeed(&'a str, FeedSelection),
    LiveDiff(DiffMode),
    Url(&'a str),
    Head(&'a str),
    EnterFm,
    LatestAgentResponse,
    AgentReference {
        model: &'a str,
        access: Option<AgentAccess>,
        id: usize,
    },
    AgentNumberReference {
        id: usize,
    },
    WatchRevisionReference {
        id: usize,
        revision: usize,
    },
    Agent {
        task: &'a str,
        access: AgentAccess,
        model: Option<AgentModel>,
    },
    Agents {
        task: &'a str,
        models: Vec<AgentModel>,
    },
    AsStaffer(Box<Input<'a>>),
    UnavailableTarget {
        name: &'a str,
        access: AgentAccess,
    },
    InteractiveAgent {
        model: AgentModel,
        access: AgentAccess,
    },
    Reserved(&'a str),
}

impl Session {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            cwd: std::env::current_dir()?,
            events: Vec::new(),
            todos: Vec::new(),
            last_status: 0,
            live_commands: Vec::new(),
            mode: PromptMode::Shell,
        })
    }
}

pub fn classify(line: &str) -> Input<'_> {
    let line = line.trim_start();
    if let Some(rest) = line.strip_prefix("@staffer").and_then(agent_task) {
        match classify(rest) {
            input @ (Input::Agent { model: Some(_), .. } | Input::Agents { .. }) => {
                Input::AsStaffer(Box::new(input))
            }
            _ => Input::Reserved(line),
        }
    } else if line.trim_end() == "?" {
        Input::Help
    } else if line.trim_end() == "#" {
        Input::LatestAgentResponse
    } else if line.trim_end() == "+" {
        Input::LiveDiff(DiffMode::Added)
    } else if line.trim_end() == "+-" {
        Input::LiveDiff(DiffMode::AddedAndRemoved)
    } else if let Some(url) = line
        .strip_prefix('+')
        .and_then(|rest| rest.trim_start().strip_prefix("HEAD"))
        .and_then(agent_task)
    {
        Input::LiveHead(url)
    } else if let Some(url) = line
        .strip_prefix('+')
        .and_then(|rest| rest.trim_start().strip_prefix("FEED"))
        .and_then(agent_task)
    {
        Input::LiveFeed(url, FeedSelection::Explicit)
    } else if let Some(url) = line
        .strip_prefix('+')
        .map(str::trim_start)
        .filter(|rest| rest.starts_with("https://") || rest.starts_with("http://"))
    {
        Input::LiveFeed(url.trim_end(), FeedSelection::ContentType)
    } else if line.trim_end() == "-" {
        Input::ShowTodos
    } else if let Some(rest) = line.strip_prefix("- [ ] ") {
        Input::Todo(rest)
    } else if let Some(command) = line
        .strip_prefix('+')
        .and_then(|rest| shell_target(rest.trim_start(), "bash?"))
    {
        Input::LiveHostShell {
            program: "bash",
            command,
        }
    } else if let Some(command) = line
        .strip_prefix('+')
        .and_then(|rest| shell_target(rest.trim_start(), "zsh?"))
    {
        Input::LiveHostShell {
            program: "zsh",
            command,
        }
    } else if line.trim_end() == "fm?" {
        Input::EnterFm
    } else if let Some((model, access, id)) = agent_reference(line.trim_end()) {
        Input::AgentReference { model, access, id }
    } else if let Some((id, revision)) = watch_revision_reference(line.trim_end()) {
        Input::WatchRevisionReference { id, revision }
    } else if let Some(id) = agent_number_reference(line.trim_end()) {
        Input::AgentNumberReference { id }
    } else if let Some((models, task)) = multi_agent_request(line) {
        Input::Agents { task, models }
    } else if let Some((model, access, task)) = agent_request(line) {
        Input::Agent {
            task,
            access,
            model: Some(model),
        }
    } else if let Some(command) = shell_target(line, "bash!") {
        Input::HostShell {
            program: "bash",
            command,
            execution: ShellExecution::CurrentUser,
        }
    } else if let Some(command) = shell_target(line, "zsh!") {
        Input::HostShell {
            program: "zsh",
            command,
            execution: ShellExecution::CurrentUser,
        }
    } else if let Some(command) = shell_target(line, "bash?") {
        Input::HostShell {
            program: "bash",
            command,
            execution: ShellExecution::Staffer,
        }
    } else if let Some(command) = shell_target(line, "zsh?") {
        Input::HostShell {
            program: "zsh",
            command,
            execution: ShellExecution::Staffer,
        }
    } else if let Some(rest) = line.strip_prefix('?') {
        Input::Agent {
            task: rest.trim(),
            access: AgentAccess::ReadOnly,
            model: None,
        }
    } else if let Some(rest) = line.strip_prefix('!') {
        Input::Agent {
            task: rest.trim(),
            access: AgentAccess::ReadWrite,
            model: None,
        }
    } else if let Some(rest) = line.strip_prefix('#') {
        Input::Comment(rest.trim())
    } else if let Some(rest) = line.strip_prefix("HEAD").and_then(agent_task) {
        Input::Head(rest)
    } else if line.starts_with("https://") || line.starts_with("http://") {
        Input::Url(line.trim_end())
    } else if let Some((name, access)) = named_target(line) {
        if let Some(model) =
            AgentModel::parse(name).filter(|_| line.split_whitespace().nth(1).is_none())
        {
            // A bare agent opens interactively: `?` read-only, `!` read-write.
            if model == AgentModel::Fm {
                Input::EnterFm
            } else {
                Input::InteractiveAgent { model, access }
            }
        } else {
            Input::UnavailableTarget { name, access }
        }
    } else if line.starts_with('+') || line.starts_with("-?") || line.starts_with('@') {
        Input::Reserved(line)
    } else {
        Input::Shell(line)
    }
}

fn agent_request(line: &str) -> Option<(AgentModel, AgentAccess, &str)> {
    let (selector, task) = line.split_once(char::is_whitespace)?;
    let (model, access) = if let Some(model) = AgentModel::from_emoji(selector) {
        (model, AgentAccess::ReadOnly)
    } else if let Some(model) = selector.strip_suffix('?') {
        (AgentModel::parse(model)?, AgentAccess::ReadOnly)
    } else {
        (
            AgentModel::parse(selector.strip_suffix('!')?)?,
            AgentAccess::ReadWrite,
        )
    };
    (!task.trim().is_empty()).then_some((model, access, task.trim()))
}

fn multi_agent_request(line: &str) -> Option<(Vec<AgentModel>, &str)> {
    let space = line.find(char::is_whitespace)?;
    let names = line[..space].strip_suffix('?')?;
    let models = names
        .split('/')
        .map(AgentModel::parse)
        .collect::<Option<Vec<_>>>()?;
    let task = line[space..].trim();
    (models.len() > 1 && !task.is_empty()).then_some((models, task))
}

pub fn is_agent_model(model: &str) -> bool {
    AgentModel::parse(model).is_some()
}

fn canonical_agent_model(model: &str) -> &str {
    match AgentModel::parse(model) {
        Some(agent) => agent.name(),
        None => model,
    }
}

pub fn agent_reference(token: &str) -> Option<(&str, Option<AgentAccess>, usize)> {
    let (name, number) = token.split_once('#')?;
    let (model, access) = if let Some(model) = name.strip_suffix('?') {
        (model, Some(AgentAccess::ReadOnly))
    } else if let Some(model) = name.strip_suffix('!') {
        (model, Some(AgentAccess::ReadWrite))
    } else {
        (name, None)
    };
    if !is_agent_model(model) {
        return None;
    }
    Some((
        canonical_agent_model(model),
        access,
        parse_agent_id(number)?,
    ))
}

pub fn agent_number_reference(token: &str) -> Option<usize> {
    parse_agent_id(token.strip_prefix('#')?)
}

pub fn watch_revision_reference(token: &str) -> Option<(usize, usize)> {
    let (id, revision) = token.strip_prefix('#')?.split_once('.')?;
    Some((parse_agent_id(id)?, parse_agent_id(revision)?))
}

fn parse_agent_id(number: &str) -> Option<usize> {
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let id = number.parse().ok()?;
    (id > 0).then_some(id)
}

fn named_target(line: &str) -> Option<(&str, AgentAccess)> {
    let prefix = line.split_whitespace().next()?;
    let (name, access) = if let Some(name) = prefix.strip_suffix('?') {
        (name, AgentAccess::ReadOnly)
    } else {
        (prefix.strip_suffix('!')?, AgentAccess::ReadWrite)
    };
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return None;
    }
    Some((name, access))
}

fn agent_task(rest: &str) -> Option<&str> {
    if rest.starts_with(char::is_whitespace) {
        Some(rest.trim())
    } else {
        None
    }
}

fn shell_target<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(prefix)?;
    if rest.is_empty() || rest.starts_with(char::is_whitespace) {
        Some(rest.trim_start())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{
        agent_number_reference, agent_reference, classify, watch_revision_reference, AgentAccess,
        AgentModel, DiffMode, Input,
    };
    use crate::shell::ShellExecution;

    #[test]
    fn pasted_multiline_agent_task_is_one_request() {
        assert!(matches!(
            classify("opus? Explain this:\nfirst line\nsecond line"),
            Input::Agent {
                task: "Explain this:\nfirst line\nsecond line",
                access: AgentAccess::ReadOnly,
                model: Some(AgentModel::Opus),
            }
        ));
    }

    #[test]
    fn agent_reply_reference_is_explicit_syntax() {
        assert!(matches!(
            classify("codex?#3"),
            Input::AgentReference {
                model: "codex",
                access: Some(AgentAccess::ReadOnly),
                id: 3
            }
        ));
        assert_eq!(
            agent_reference("claude!#12"),
            Some(("claude", Some(AgentAccess::ReadWrite), 12))
        );
        assert_eq!(agent_reference("claude#12"), Some(("claude", None, 12)));
        assert_eq!(agent_reference("codex#0"), None);
        assert_eq!(agent_reference("codex#3x"), None);
        assert_eq!(agent_reference("issue#3"), None);
        assert!(matches!(
            classify("#3"),
            Input::AgentNumberReference { id: 3 }
        ));
        assert!(matches!(classify("#"), Input::LatestAgentResponse));
        assert_eq!(agent_number_reference("#12"), Some(12));
        assert_eq!(agent_number_reference("#0"), None);
        assert_eq!(agent_number_reference("#3x"), None);
        assert_eq!(watch_revision_reference("#3.2"), Some((3, 2)));
        assert_eq!(watch_revision_reference("#3.0"), None);
        assert!(matches!(
            classify("#3.2"),
            Input::WatchRevisionReference { id: 3, revision: 2 }
        ));
        assert!(matches!(classify("# note"), Input::Comment("note")));
    }

    #[test]
    fn bare_dash_lists_todos() {
        assert!(matches!(classify("-"), Input::ShowTodos));
        assert!(matches!(classify("-   "), Input::ShowTodos));
        assert!(matches!(
            classify("- [ ] write docs"),
            Input::Todo("write docs")
        ));
    }

    #[test]
    fn fm_prefixes_are_explicit() {
        assert!(matches!(
            classify("fm? explain"),
            Input::Agent {
                model: Some(AgentModel::Fm),
                access: AgentAccess::ReadOnly,
                task: "explain"
            }
        ));
        assert!(matches!(
            classify("fm! fix"),
            Input::Agent {
                model: Some(AgentModel::Fm),
                access: AgentAccess::ReadWrite,
                task: "fix"
            }
        ));
        assert!(matches!(classify("fm?file"), Input::Shell("fm?file")));
        assert!(matches!(classify("fm!file"), Input::Shell("fm!file")));
    }

    #[test]
    fn claude_question_is_an_agent_request() {
        assert!(matches!(
            classify("claude? explain the parser"),
            Input::Agent {
                task: "explain the parser",
                access: AgentAccess::ReadOnly,
                model: Some(AgentModel::Claude)
            }
        ));
        assert!(matches!(
            classify("claude?file"),
            Input::Shell("claude?file")
        ));
        assert!(matches!(
            classify("claude! fix the parser"),
            Input::Agent {
                model: Some(AgentModel::Claude),
                access: AgentAccess::ReadWrite,
                task: "fix the parser"
            }
        ));
    }

    #[test]
    fn jsc_is_an_agent_only_on_macos() {
        assert_eq!(super::is_agent_model("jsc"), cfg!(target_os = "macos"));
    }

    #[test]
    fn codex_and_pi_questions_are_agent_requests() {
        for name in ["codex", "pi"] {
            let line = format!("{name}? inspect this");
            assert!(matches!(
                classify(&line),
                Input::Agent {
                    task: "inspect this",
                    access: AgentAccess::ReadOnly,
                    model: Some(model)
                } if model.name() == name
            ));
            let writable = format!("{name}! fix this");
            assert!(matches!(
                classify(&writable),
                Input::Agent { task: "fix this", access: AgentAccess::ReadWrite, model: Some(model) } if model.name() == name
            ));
        }
        assert!(matches!(
            classify("codex! Please add LICENSE with Apache-2.0 with my full name"),
            Input::Agent {
                task: "Please add LICENSE with Apache-2.0 with my full name",
                access: AgentAccess::ReadWrite,
                model: Some(AgentModel::Codex)
            }
        ));
    }

    #[test]
    fn claude_model_aliases_are_agent_requests() {
        for name in ["fable", "opus", "sonnet", "haiku"] {
            let line = format!("{name}? inspect this");
            assert!(matches!(
                classify(&line),
                Input::Agent {
                    task: "inspect this",
                    access: AgentAccess::ReadOnly,
                    model: Some(model)
                } if model.name() == name
            ));
            let writable = format!("{name}! fix this");
            assert!(matches!(
                classify(&writable),
                Input::Agent { task: "fix this", access: AgentAccess::ReadWrite, model: Some(model) } if model.name() == name
            ));
        }
    }

    #[test]
    fn capitalized_opus_uses_the_opus_model() {
        assert!(matches!(
            classify("Opus? inspect this"),
            Input::Agent {
                task: "inspect this",
                access: AgentAccess::ReadOnly,
                model: Some(AgentModel::Opus)
            }
        ));
        assert!(matches!(
            classify("Opus! fix this"),
            Input::Agent {
                task: "fix this",
                access: AgentAccess::ReadWrite,
                model: Some(AgentModel::Opus)
            }
        ));
        assert!(matches!(
            classify("Opus?"),
            Input::InteractiveAgent {
                model: AgentModel::Opus,
                access: AgentAccess::ReadOnly
            }
        ));
        assert!(matches!(
            classify("Opus/luna? compare"),
            Input::Agents { models, task: "compare" } if models == [AgentModel::Opus, AgentModel::Luna]
        ));
        assert_eq!(
            agent_reference("Opus?#3"),
            Some(("opus", Some(AgentAccess::ReadOnly), 3))
        );
    }

    #[test]
    fn codex_model_aliases_are_agent_requests() {
        for name in ["sol", "luna", "terra"] {
            let line = format!("{name}? inspect this");
            assert!(matches!(
                classify(&line),
                Input::Agent {
                    task: "inspect this",
                    access: AgentAccess::ReadOnly,
                    model: Some(model)
                } if model.name() == name
            ));
            let writable = format!("{name}! fix this");
            assert!(matches!(
                classify(&writable),
                Input::Agent { task: "fix this", access: AgentAccess::ReadWrite, model: Some(model) } if model.name() == name
            ));
        }
    }

    #[test]
    fn emoji_aliases_are_read_only_agent_requests() {
        for (selector, model) in [
            ("🎵", "opus"),
            ("🎶", "opus"),
            ("☀️", "sol"),
            ("☀", "sol"),
            ("🌙", "luna"),
            ("🥧", "pi"),
            ("🍎", "fm"),
        ] {
            let line = format!("{selector} inspect this");
            assert!(matches!(
                classify(&line),
                Input::Agent {
                    task: "inspect this",
                    access: AgentAccess::ReadOnly,
                    model: Some(actual)
                } if actual.name() == model
            ));
        }
        assert!(matches!(classify("☀️inspect"), Input::Shell(_)));
        assert!(matches!(classify("🌙inspect"), Input::Shell(_)));
        assert!(matches!(classify("🎵inspect"), Input::Shell(_)));
        assert!(matches!(classify("🎶inspect"), Input::Shell(_)));
        assert!(matches!(classify("🥧inspect"), Input::Shell(_)));
        assert!(matches!(classify("🍎inspect"), Input::Shell(_)));
    }

    #[test]
    fn slash_prefix_asks_multiple_agents_the_same_task() {
        assert!(matches!(
            classify("opus/luna? what replaces tree?"),
            Input::Agents { task: "what replaces tree?", models }
                if models == [AgentModel::Opus, AgentModel::Luna]
        ));
        assert!(matches!(
            classify("fm/sol/sonnet? inspect this"),
            Input::Agents { task: "inspect this", models }
                if models == [AgentModel::Fm, AgentModel::Sol, AgentModel::Sonnet]
        ));
        assert!(matches!(
            classify("opus? luna? inspect this"),
            Input::Agent {
                model: Some(AgentModel::Opus),
                task: "luna? inspect this",
                ..
            }
        ));
    }

    #[test]
    fn staffer_prefix_wraps_agent_requests_only() {
        assert!(matches!(
            classify("@staffer opus? Is there a LICENSE?"),
            Input::AsStaffer(inner) if matches!(*inner, Input::Agent {
                task: "Is there a LICENSE?",
                access: AgentAccess::ReadOnly,
                model: Some(AgentModel::Opus)
            })
        ));
        assert!(matches!(
            classify("@staffer codex! add LICENSE"),
            Input::AsStaffer(inner) if matches!(*inner, Input::Agent {
                task: "add LICENSE",
                access: AgentAccess::ReadWrite,
                model: Some(AgentModel::Codex)
            })
        ));
        assert!(matches!(
            classify("@staffer git status"),
            Input::Reserved(_)
        ));
    }

    #[test]
    fn live_shell_commands_are_classified() {
        assert!(matches!(classify("+"), Input::LiveDiff(DiffMode::Added)));
        assert!(matches!(
            classify("  +  "),
            Input::LiveDiff(DiffMode::Added)
        ));
        assert!(matches!(
            classify("+-"),
            Input::LiveDiff(DiffMode::AddedAndRemoved)
        ));
        assert!(matches!(
            classify("+ bash? git status"),
            Input::LiveHostShell {
                program: "bash",
                command: "git status"
            }
        ));
        assert!(matches!(
            classify("+bash? ls"),
            Input::LiveHostShell {
                program: "bash",
                command: "ls"
            }
        ));
        assert!(matches!(
            classify("+ zsh? pwd"),
            Input::LiveHostShell {
                program: "zsh",
                command: "pwd"
            }
        ));
        assert!(matches!(
            classify("+ git status"),
            Input::Reserved("+ git status")
        ));
        assert!(matches!(
            classify("+ git log"),
            Input::Reserved("+ git log")
        ));
    }

    #[test]
    fn live_head_is_classified_and_labeled() {
        assert!(matches!(
            classify("+ HEAD https://github.com/patrickgwsmith/euka"),
            Input::LiveHead("https://github.com/patrickgwsmith/euka")
        ));
        assert!(matches!(
            classify("+HEAD https://example.com/"),
            Input::LiveHead("https://example.com/")
        ));
        let live = super::LiveCommand {
            id: 1,
            cwd: std::path::PathBuf::from("/tmp"),
            target: super::LiveTarget::Head {
                url: "https://example.com/".into(),
            },
            output: String::new(),
            error: None,
            last_run: None,
            last_refresh: std::time::Instant::now(),
            revisions: Vec::new(),
        };
        assert_eq!(live.label(), "+ HEAD https://example.com/");
    }

    #[test]
    fn atom_urls_can_be_watched_explicitly_or_by_content_type() {
        assert!(matches!(
            classify("+ FEED https://example.com/main.atom"),
            Input::LiveFeed(
                "https://example.com/main.atom",
                super::FeedSelection::Explicit
            )
        ));
        assert!(matches!(
            classify("+ https://example.com/main.atom"),
            Input::LiveFeed(
                "https://example.com/main.atom",
                super::FeedSelection::ContentType
            )
        ));
    }

    #[test]
    fn bare_fm_question_enters_mode() {
        assert!(matches!(classify("fm?"), Input::EnterFm));
    }

    #[test]
    fn bare_question_shows_help() {
        assert!(matches!(classify("?"), Input::Help));
        assert!(matches!(classify("  ?  "), Input::Help));
        assert!(matches!(
            classify("? explain"),
            Input::Agent {
                model: None,
                access: AgentAccess::ReadOnly,
                task: "explain"
            }
        ));
    }

    #[test]
    fn named_shell_commands_keep_their_script() {
        assert!(matches!(
            classify("bash! echo a | tr a b"),
            Input::HostShell {
                program: "bash",
                command: "echo a | tr a b",
                execution: ShellExecution::CurrentUser
            }
        ));
        assert!(matches!(
            classify("zsh! printf '%s\\n' *.rs"),
            Input::HostShell {
                program: "zsh",
                command: "printf '%s\\n' *.rs",
                execution: ShellExecution::CurrentUser
            }
        ));
        assert!(matches!(
            classify("bash? git status"),
            Input::HostShell {
                program: "bash",
                command: "git status",
                execution: ShellExecution::Staffer
            }
        ));
        assert!(matches!(
            classify("zsh? pwd"),
            Input::HostShell {
                program: "zsh",
                command: "pwd",
                execution: ShellExecution::Staffer
            }
        ));
        assert!(matches!(classify("bash!file"), Input::Shell("bash!file")));
        assert!(matches!(classify("zsh!file"), Input::Shell("zsh!file")));
        assert!(matches!(classify("bash?file"), Input::Shell("bash?file")));
        assert!(matches!(classify("zsh?file"), Input::Shell("zsh?file")));
    }

    #[test]
    fn unknown_named_targets_do_not_become_shell_commands() {
        assert!(matches!(
            classify("unknownagent? explain the failure"),
            Input::UnavailableTarget {
                name: "unknownagent",
                access: AgentAccess::ReadOnly
            }
        ));
        assert!(matches!(
            classify("unknownagent! fix it"),
            Input::UnavailableTarget {
                name: "unknownagent",
                access: AgentAccess::ReadWrite
            }
        ));
        assert!(matches!(
            classify("foo?"),
            Input::UnavailableTarget {
                name: "foo",
                access: AgentAccess::ReadOnly
            }
        ));
        assert!(matches!(classify("foo?bar"), Input::Shell("foo?bar")));
        assert!(matches!(
            classify("./foo? task"),
            Input::Shell("./foo? task")
        ));
    }

    #[test]
    fn bare_agent_opens_interactively() {
        assert!(matches!(
            classify("opus?"),
            Input::InteractiveAgent {
                model: AgentModel::Opus,
                access: AgentAccess::ReadOnly
            }
        ));
        assert!(matches!(
            classify("  codex!  "),
            Input::InteractiveAgent {
                model: AgentModel::Codex,
                access: AgentAccess::ReadWrite
            }
        ));
        assert!(matches!(classify("fm?"), Input::EnterFm));
        assert!(matches!(classify("fm!"), Input::EnterFm));
    }

    #[test]
    fn bare_url_is_a_resource_request() {
        assert!(matches!(
            classify("https://example.com/spec"),
            Input::Url("https://example.com/spec")
        ));
    }

    #[test]
    fn head_url_is_a_diagnostic_request() {
        assert!(matches!(
            classify("HEAD https://example.com/"),
            Input::Head("https://example.com/")
        ));
        assert!(matches!(
            classify("head file.txt"),
            Input::Shell("head file.txt")
        ));
    }
}
