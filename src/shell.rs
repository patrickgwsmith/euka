use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellExecution {
    CurrentUser,
    Staffer,
}

impl ShellExecution {
    pub fn suffix(self) -> char {
        match self {
            Self::CurrentUser => '!',
            Self::Staffer => '?',
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Word(String),
    Pipe,
    In,
    Out,
    Append,
    ErrOut,
    ErrAppend,
}

#[derive(Default, Debug)]
struct Stage {
    args: Vec<String>,
    stdin: Option<String>,
    stdout: Option<(String, bool)>,
    stderr: Option<(String, bool)>,
}

fn expand(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, last_status: i32) -> String {
    if chars.peek() == Some(&'?') {
        chars.next();
        return last_status.to_string();
    }
    let mut name = String::new();
    if chars.peek() == Some(&'{') {
        chars.next();
        while let Some(&c) = chars.peek() {
            chars.next();
            if c == '}' {
                break;
            }
            name.push(c);
        }
    } else {
        while let Some(&c) = chars.peek() {
            if !c.is_ascii_alphanumeric() && c != '_' {
                break;
            }
            name.push(c);
            chars.next();
        }
    }
    if name.is_empty() {
        "$".into()
    } else {
        std::env::var(&name).unwrap_or_default()
    }
}

fn lex(input: &str, last_status: i32) -> Result<Vec<Token>, String> {
    let mut chars = input.chars().peekable();
    let mut tokens = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote = None;
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else if c == '$' && q == '"' {
                word.push_str(&expand(&mut chars, last_status));
            } else if c == '\\' && q == '"' {
                if let Some(next) = chars.next() {
                    if !matches!(next, '$' | '"' | '\\' | '\n') {
                        word.push('\\');
                    }
                    if next != '\n' {
                        word.push(next);
                    }
                } else {
                    return Err("trailing escape".into());
                }
            } else {
                word.push(c);
            }
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                started = true;
            }
            '\\' => {
                word.push(chars.next().ok_or("trailing escape")?);
                started = true;
            }
            '$' => {
                word.push_str(&expand(&mut chars, last_status));
                started = true;
            }
            '~' if !started && matches!(chars.peek(), None | Some(' ' | '\t' | '/')) => {
                word.push_str(&std::env::var("HOME").unwrap_or_else(|_| "~".into()));
                started = true;
            }
            ' ' | '\t' => {
                if started {
                    tokens.push(Token::Word(std::mem::take(&mut word)));
                    started = false;
                }
            }
            '|' | '<' | '>' => {
                if started {
                    tokens.push(Token::Word(std::mem::take(&mut word)));
                    started = false;
                }
                tokens.push(match c {
                    '|' => Token::Pipe,
                    '<' => Token::In,
                    _ if chars.peek() == Some(&'>') => {
                        chars.next();
                        Token::Append
                    }
                    _ => Token::Out,
                });
            }
            '2' if !started && chars.peek() == Some(&'>') => {
                chars.next();
                let append = chars.peek() == Some(&'>');
                if append {
                    chars.next();
                }
                tokens.push(if append {
                    Token::ErrAppend
                } else {
                    Token::ErrOut
                });
            }
            _ => {
                word.push(c);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err("unclosed quote".into());
    }
    if started {
        tokens.push(Token::Word(word));
    }
    Ok(tokens)
}

fn parse(input: &str, last_status: i32) -> Result<Vec<Stage>, String> {
    let tokens = lex(input, last_status)?;
    if tokens.is_empty() {
        return Ok(Vec::new());
    }
    let mut stages = vec![Stage::default()];
    let mut iter = tokens.into_iter();
    while let Some(token) = iter.next() {
        let stage = stages.last_mut().unwrap();
        match token {
            Token::Word(s) => stage.args.push(s),
            Token::Pipe => {
                if stage.args.is_empty() {
                    return Err("empty pipeline stage".into());
                }
                stages.push(Stage::default());
            }
            op => {
                let path = match iter.next() {
                    Some(Token::Word(s)) => s,
                    _ => return Err("redirection needs a filename".into()),
                };
                match op {
                    Token::In => stage.stdin = Some(path),
                    Token::Out => stage.stdout = Some((path, false)),
                    Token::Append => stage.stdout = Some((path, true)),
                    Token::ErrOut => stage.stderr = Some((path, false)),
                    Token::ErrAppend => stage.stderr = Some((path, true)),
                    _ => unreachable!(),
                }
            }
        }
    }
    if stages.last().unwrap().args.is_empty() {
        return Err("empty pipeline stage".into());
    }
    Ok(stages)
}

fn output_file(path: &str, append: bool) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(path)
}

pub enum Outcome {
    Status(i32),
    Exit(i32),
}

pub fn reader_shell_command(program: &str, cwd: &Path, noninteractive: bool) -> Command {
    let mut command = Command::new("sudo");
    if noninteractive {
        command.arg("-n");
    }
    command.args(["-H", "-u", "staffer", "--"]);
    if let Some(root) = owned_git_root(cwd) {
        command.arg("/usr/bin/env");
        command.args(["GIT_CONFIG_COUNT=1", "GIT_CONFIG_KEY_0=safe.directory"]);
        let mut value = OsString::from("GIT_CONFIG_VALUE_0=");
        value.push(root.as_os_str());
        command.arg(value);
    }
    command.arg(format!("/bin/{program}"));
    command
}

fn owned_git_root(cwd: &Path) -> Option<PathBuf> {
    let owner = unsafe { libc::geteuid() };
    for directory in cwd.ancestors() {
        let git_path = directory.join(".git");
        let git_metadata = match fs::metadata(&git_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => return None,
        };
        let root = fs::canonicalize(directory).ok()?;
        if fs::metadata(&root).ok()?.uid() != owner || git_metadata.uid() != owner {
            return None;
        }
        if git_metadata.is_file() {
            let git_file = fs::read_to_string(&git_path).ok()?;
            let path = git_file.lines().next()?.strip_prefix("gitdir: ")?.trim();
            let git_dir = if Path::new(path).is_absolute() {
                PathBuf::from(path)
            } else {
                directory.join(path)
            };
            if fs::metadata(git_dir).ok()?.uid() != owner {
                return None;
            }
        } else if !git_metadata.is_dir() {
            return None;
        }
        return Some(root);
    }
    None
}

pub fn execute_host_shell(
    program: &str,
    script: &str,
    execution: ShellExecution,
) -> Result<i32, String> {
    if script.trim().is_empty() {
        return Err(format!(
            "{program}{}: expected a command",
            execution.suffix()
        ));
    }
    let mut child = match execution {
        ShellExecution::Staffer => {
            let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
            reader_shell_command(program, &cwd, unsafe { libc::isatty(0) } != 1)
        }
        ShellExecution::CurrentUser => Command::new(program),
    };
    child.arg("-c").arg(script);
    unsafe {
        child.pre_exec(|| {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGQUIT, libc::SIG_DFL);
            Ok(())
        });
    }
    let status = match child.status() {
        Ok(status) => status,
        Err(error) => {
            eprintln!("euka: {program}: {error}");
            return Ok(if error.kind() == io::ErrorKind::NotFound {
                127
            } else {
                126
            });
        }
    };
    use std::os::unix::process::ExitStatusExt;
    Ok(status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0)))
}

pub fn execute_substitution_shell(script: &str) -> Result<i32, String> {
    let shell = std::env::var("SHELL").ok();
    let program = shell.as_deref().filter(|shell| {
        matches!(
            Path::new(shell).file_name().and_then(|name| name.to_str()),
            Some("bash" | "zsh")
        )
    });
    let fallback = if cfg!(target_os = "macos") {
        "zsh"
    } else {
        "bash"
    };
    execute_host_shell(
        program.unwrap_or(fallback),
        script,
        ShellExecution::CurrentUser,
    )
}

pub fn execute(input: &str, last_status: i32) -> Result<Outcome, String> {
    let mut stages = parse(input, last_status)?;
    if stages.is_empty() {
        return Ok(Outcome::Status(0));
    }
    if stages.len() == 1 {
        let args = &stages[0].args;
        match args[0].as_str() {
            "cd" => {
                if args.len() > 2 {
                    return Err("cd: too many arguments".into());
                }
                let target = if args.len() == 1 {
                    std::env::var("HOME").map_err(|_| "cd: HOME is unset")?
                } else if args[1] == "-" {
                    std::env::var("OLDPWD").map_err(|_| "cd: OLDPWD is unset")?
                } else {
                    args[1].clone()
                };
                let old = std::env::current_dir().map_err(|e| e.to_string())?;
                std::env::set_current_dir(&target).map_err(|e| format!("cd: {target}: {e}"))?;
                std::env::set_var("OLDPWD", old);
                std::env::set_var("PWD", std::env::current_dir().unwrap());
                return Ok(Outcome::Status(0));
            }
            "exit" => {
                let status = if args.len() > 1 {
                    args[1]
                        .parse()
                        .map_err(|_| "exit: expected numeric status")?
                } else {
                    last_status
                };
                return Ok(Outcome::Exit(status));
            }
            "export" => {
                for arg in &args[1..] {
                    let (key, value) = arg.split_once('=').ok_or("export: expected NAME=value")?;
                    if !valid_name(key) {
                        return Err(format!("export: invalid name: {key}"));
                    }
                    std::env::set_var(key, value);
                }
                return Ok(Outcome::Status(0));
            }
            "unset" => {
                for key in &args[1..] {
                    std::env::remove_var(key);
                }
                return Ok(Outcome::Status(0));
            }
            _ => {}
        }
        if args.len() == 1 {
            if let Some((key, value)) = args[0].split_once('=') {
                if valid_name(key) {
                    std::env::set_var(key, value);
                    return Ok(Outcome::Status(0));
                }
            }
        }
    }
    let count = stages.len();
    let mut children: Vec<Child> = Vec::new();
    let mut previous = None;
    for (index, stage) in stages.iter_mut().enumerate() {
        let mut envs = Vec::new();
        while let Some((key, value)) = stage.args.first().and_then(|s| s.split_once('=')) {
            if !valid_name(key) {
                break;
            }
            envs.push((key.to_owned(), value.to_owned()));
            stage.args.remove(0);
        }
        if stage.args.is_empty() {
            return Err("missing command after environment assignment".into());
        }
        let mut command = Command::new(&stage.args[0]);
        command.args(&stage.args[1..]).envs(envs);
        if let Some(path) = &stage.stdin {
            command.stdin(File::open(path).map_err(|e| format!("{path}: {e}"))?);
        } else if let Some(stdout) = previous.take() {
            command.stdin(Stdio::from(stdout));
        }
        if let Some((path, append)) = &stage.stdout {
            command.stdout(output_file(path, *append).map_err(|e| format!("{path}: {e}"))?);
        } else if index + 1 < count {
            command.stdout(Stdio::piped());
        }
        if let Some((path, append)) = &stage.stderr {
            command.stderr(output_file(path, *append).map_err(|e| format!("{path}: {e}"))?);
        }
        unsafe {
            command.pre_exec(|| {
                libc::signal(libc::SIGINT, libc::SIG_DFL);
                libc::signal(libc::SIGQUIT, libc::SIG_DFL);
                Ok(())
            });
        }
        match command.spawn() {
            Ok(mut child) => {
                previous = child.stdout.take();
                children.push(child);
            }
            Err(e) => {
                drop(previous);
                for child in &mut children {
                    let _ = child.kill();
                }
                for child in &mut children {
                    let _ = child.wait();
                }
                eprintln!("euka: {}: {e}", stage.args[0]);
                return Ok(Outcome::Status(if e.kind() == io::ErrorKind::NotFound {
                    127
                } else {
                    126
                }));
            }
        }
    }
    let mut status = 0;
    for (index, child) in children.iter_mut().enumerate() {
        let result = child.wait().map_err(|e| e.to_string())?;
        if index + 1 == count {
            use std::os::unix::process::ExitStatusExt;
            status = result
                .code()
                .unwrap_or_else(|| 128 + result.signal().unwrap_or(0));
        }
    }
    Ok(Outcome::Status(status))
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn reader_shell_trusts_only_the_current_users_repository() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("euka-reader-{}-{unique}", std::process::id()));
        let nested = root.join("nested");
        fs::create_dir_all(&nested).unwrap();

        let plain = reader_shell_command("bash", &nested, true);
        let plain_args: Vec<_> = plain
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(!plain_args.iter().any(|arg| arg.starts_with("GIT_CONFIG_")));

        fs::create_dir(root.join(".git")).unwrap();
        let trusted = reader_shell_command("zsh", &nested, false);
        let trusted_args: Vec<_> = trusted
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(trusted_args.contains(&"GIT_CONFIG_COUNT=1".to_owned()));
        assert!(trusted_args.contains(&"GIT_CONFIG_KEY_0=safe.directory".to_owned()));
        assert!(trusted_args.contains(&format!(
            "GIT_CONFIG_VALUE_0={}",
            root.canonicalize().unwrap().display()
        )));
        assert_eq!(trusted_args.last().unwrap(), "/bin/zsh");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn quotes_and_operators() {
        assert_eq!(
            lex("echo 'a b' \"c d\"|cat 2>>err", 0).unwrap(),
            vec![
                Token::Word("echo".into()),
                Token::Word("a b".into()),
                Token::Word("c d".into()),
                Token::Pipe,
                Token::Word("cat".into()),
                Token::ErrAppend,
                Token::Word("err".into())
            ]
        );
    }
    #[test]
    fn empty_quotes_and_bad_syntax() {
        assert_eq!(lex("echo ''", 0).unwrap()[1], Token::Word(String::new()));
        assert!(parse("echo |", 0).is_err());
        assert!(parse("cat >", 0).is_err());
        assert!(parse("echo 'unfinished", 0).is_err());
    }
}
