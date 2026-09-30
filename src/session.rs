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

pub struct LiveCommand {
    pub id: usize,
    pub cwd: PathBuf,
    pub program: &'static str,
    pub command: String,
    pub output: String,
    pub error: Option<String>,
    pub last_run: Option<String>,
    pub last_refresh: Instant,
}

impl LiveCommand {
    pub fn label(&self) -> String {
        format!("+ {}? {}", self.program, self.command)
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
    Resource(Resource),
    HttpHead {
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
        reader: bool,
    },
    Comment(&'a str),
    Todo(&'a str),
    LiveHostShell {
        program: &'static str,
        command: &'a str,
    },
    LiveDiff,
    Url(&'a str),
    Head(&'a str),
    EnterFm,
    AgentReference {
        model: &'a str,
        id: usize,
    },
    AgentNumberReference {
        id: usize,
    },
    Agent {
        task: &'a str,
        access: AgentAccess,
        model: Option<&'a str>,
    },
    Agents {
        task: &'a str,
        models: Vec<&'a str>,
    },
    AsStaffer(Box<Input<'a>>),
    UnavailableTarget {
        name: &'a str,
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
    } else if line.trim_end() == "+" {
        Input::LiveDiff
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
    } else if let Some((model, id)) = agent_reference(line.trim_end()) {
        Input::AgentReference { model, id }
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
            reader: false,
        }
    } else if let Some(command) = shell_target(line, "zsh!") {
        Input::HostShell {
            program: "zsh",
            command,
            reader: false,
        }
    } else if let Some(command) = shell_target(line, "bash?") {
        Input::HostShell {
            program: "bash",
            command,
            reader: true,
        }
    } else if let Some(command) = shell_target(line, "zsh?") {
        Input::HostShell {
            program: "zsh",
            command,
            reader: true,
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
        Input::UnavailableTarget { name, access }
    } else if line.starts_with('+') || line.starts_with("-?") || line.starts_with('@') {
        Input::Reserved(line)
    } else {
        Input::Shell(line)
    }
}

fn agent_request(line: &str) -> Option<(&str, AgentAccess, &str)> {
    let (selector, task) = line.split_once(char::is_whitespace)?;
    let (model, access) = if let Some(model) = selector.strip_suffix('?') {
        (model, AgentAccess::ReadOnly)
    } else {
        (selector.strip_suffix('!')?, AgentAccess::ReadWrite)
    };
    (is_agent_model(model) && !task.trim().is_empty()).then_some((model, access, task.trim()))
}

fn multi_agent_request(line: &str) -> Option<(Vec<&str>, &str)> {
    let space = line.find(char::is_whitespace)?;
    let names = line[..space].strip_suffix('?')?;
    let models: Vec<&str> = names.split('/').collect();
    let task = line[space..].trim();
    (models.len() > 1 && models.iter().all(|model| is_agent_model(model)) && !task.is_empty())
        .then_some((models, task))
}

pub fn is_agent_model(model: &str) -> bool {
    matches!(
        model,
        "fm" | "claude" | "opus" | "sonnet" | "codex" | "sol" | "luna" | "terra" | "pi"
    )
}

pub fn agent_reference(token: &str) -> Option<(&str, usize)> {
    let (model, number) = token.split_once('#')?;
    if !is_agent_model(model) {
        return None;
    }
    Some((model, parse_agent_id(number)?))
}

pub fn agent_number_reference(token: &str) -> Option<usize> {
    parse_agent_id(token.strip_prefix('#')?)
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
    use super::{agent_number_reference, agent_reference, classify, AgentAccess, Input};

    #[test]
    fn agent_reply_reference_is_explicit_syntax() {
        assert!(matches!(
            classify("codex#3"),
            Input::AgentReference {
                model: "codex",
                id: 3
            }
        ));
        assert_eq!(agent_reference("claude#12"), Some(("claude", 12)));
        assert_eq!(agent_reference("codex#0"), None);
        assert_eq!(agent_reference("codex#3x"), None);
        assert_eq!(agent_reference("issue#3"), None);
        assert!(matches!(
            classify("#3"),
            Input::AgentNumberReference { id: 3 }
        ));
        assert_eq!(agent_number_reference("#12"), Some(12));
        assert_eq!(agent_number_reference("#0"), None);
        assert_eq!(agent_number_reference("#3x"), None);
        assert!(matches!(classify("# note"), Input::Comment("note")));
    }

    #[test]
    fn fm_prefixes_are_explicit() {
        assert!(matches!(
            classify("fm? explain"),
            Input::Agent {
                model: Some("fm"),
                access: AgentAccess::ReadOnly,
                task: "explain"
            }
        ));
        assert!(matches!(
            classify("fm! fix"),
            Input::Agent {
                model: Some("fm"),
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
                model: Some("claude")
            }
        ));
        assert!(matches!(
            classify("claude?file"),
            Input::Shell("claude?file")
        ));
        assert!(matches!(
            classify("claude! fix the parser"),
            Input::Agent {
                model: Some("claude"),
                access: AgentAccess::ReadWrite,
                task: "fix the parser"
            }
        ));
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
                } if model == name
            ));
            let writable = format!("{name}! fix this");
            assert!(matches!(
                classify(&writable),
                Input::Agent { task: "fix this", access: AgentAccess::ReadWrite, model: Some(model) } if model == name
            ));
        }
        assert!(matches!(
            classify("codex! Please add LICENSE with Apache-2.0 with my full name"),
            Input::Agent {
                task: "Please add LICENSE with Apache-2.0 with my full name",
                access: AgentAccess::ReadWrite,
                model: Some("codex")
            }
        ));
    }

    #[test]
    fn claude_model_aliases_are_agent_requests() {
        for name in ["opus", "sonnet"] {
            let line = format!("{name}? inspect this");
            assert!(matches!(
                classify(&line),
                Input::Agent {
                    task: "inspect this",
                    access: AgentAccess::ReadOnly,
                    model: Some(model)
                } if model == name
            ));
            let writable = format!("{name}! fix this");
            assert!(matches!(
                classify(&writable),
                Input::Agent { task: "fix this", access: AgentAccess::ReadWrite, model: Some(model) } if model == name
            ));
        }
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
                } if model == name
            ));
            let writable = format!("{name}! fix this");
            assert!(matches!(
                classify(&writable),
                Input::Agent { task: "fix this", access: AgentAccess::ReadWrite, model: Some(model) } if model == name
            ));
        }
    }

    #[test]
    fn slash_prefix_asks_multiple_agents_the_same_task() {
        assert!(matches!(
            classify("opus/luna? what replaces tree?"),
            Input::Agents { task: "what replaces tree?", models }
                if models == ["opus", "luna"]
        ));
        assert!(matches!(
            classify("fm/sol/sonnet? inspect this"),
            Input::Agents { task: "inspect this", models }
                if models == ["fm", "sol", "sonnet"]
        ));
        assert!(matches!(
            classify("opus? luna? inspect this"),
            Input::Agent {
                model: Some("opus"),
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
                model: Some("opus")
            })
        ));
        assert!(matches!(
            classify("@staffer codex! add LICENSE"),
            Input::AsStaffer(inner) if matches!(*inner, Input::Agent {
                task: "add LICENSE",
                access: AgentAccess::ReadWrite,
                model: Some("codex")
            })
        ));
        assert!(matches!(
            classify("@staffer git status"),
            Input::Reserved(_)
        ));
    }

    #[test]
    fn live_shell_commands_are_classified() {
        assert!(matches!(classify("+"), Input::LiveDiff));
        assert!(matches!(classify("  +  "), Input::LiveDiff));
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
                reader: false
            }
        ));
        assert!(matches!(
            classify("zsh! printf '%s\\n' *.rs"),
            Input::HostShell {
                program: "zsh",
                command: "printf '%s\\n' *.rs",
                reader: false
            }
        ));
        assert!(matches!(
            classify("bash? git status"),
            Input::HostShell {
                program: "bash",
                command: "git status",
                reader: true
            }
        ));
        assert!(matches!(
            classify("zsh? pwd"),
            Input::HostShell {
                program: "zsh",
                command: "pwd",
                reader: true
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
