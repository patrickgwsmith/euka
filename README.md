# Euka

Euka is an experimental shell for humans and coding agents to share. It implements a direct Unix shell and Apple Foundation Models, Claude Code, Codex, and Pi targets. Euka is short for eukaryotes.

Build and run with `cargo run`. Use `cargo run -- --help` to print help and exit. Euka supports macOS and Linux.

The interactive prompt shows the current directory in a compact form, like fish: parent directory names are shortened to one character, the current directory name stays in full, and the home directory is shown as `~`. For example, `/Users/pgwsmith/Collected/euka` appears as `~/C/euka>`. The prompt is bold cyan in a color terminal; `fm?` mode adds a bold magenta label. As you type, agent selectors such as `opus/luna?` and `codex!` are magenta, `bash?`, `zsh?`, and `@staffer` are blue, and `bash!` and `zsh!` are red. The same colors apply to `+ bash?` and `+ zsh?` watches. Set `NO_COLOR=1` for a plain prompt. Press Down at an empty prompt to show the full absolute path for three seconds.

Enter `?` at the shell prompt for a cheatsheet of current prefixes and built-ins. On macOS, it also shows the command for creating a `staffer` account for `bash?` and `zsh?`.

Current shell features:

- Interactive input with editing, in-memory history, history suggestions, and Tab completion for commands and file or directory arguments. Paths can be relative, absolute, or under `~/`; spaces in names are escaped. Ctrl-A/E/B/F move the cursor; Ctrl-W/U/K delete text; Ctrl-Y yanks it back; Ctrl-P/N browse history; Option-B/F/D move or delete by word.
- PATH command lookup, arguments, quotes, backslash escapes, `$NAME` and `$?` expansion.
- Bare commands containing shell command substitution (`$(...)`) run through Bash or Zsh. Euka uses `$SHELL` when it names Bash or Zsh, and otherwise uses Zsh on macOS or Bash on Linux. Shell-run built-ins such as `cd` do not change Euka's own directory.
- Use `name?(task)` as one argument of an ordinary command to insert a read-only agent reply, for example `git commit -m luna?(Suggest a commit message)`. Euka waits for the agent, records its numbered reply, then passes its text as one literal argument. The task cannot contain `(`; the first `)` closes it. A missing `)` or text attached after it is an error. `$()` remains shell command substitution.
- `cd`, `export NAME=value`, `unset NAME`, `exit`, and standalone `NAME=value`. `reset` clears the in-memory session, input history, live watches, and request counters, so the next agent request is `#1`. It keeps the current directory and environment. Requests already running may finish externally, but their results are discarded. Use `bash! reset` to run the system terminal reset utility.
- Foreground commands, pipelines, `<`, `>`, `>>`, `2>`, and `2>>`.
- `bash! command` and `zsh! command` run a command through that shell as a foreground child process. Euka passes the script through unchanged, records its exit status, and does not invoke an LLM. Changes to the child shell's directory or environment do not carry back to Euka.
- `bash? command` and `zsh? command` run through `sudo` as the `staffer` account, in the foreground. `sudo` may ask for your password. In noninteractive input, Euka uses `sudo -n` so a missing authorization fails immediately. For a Git repository owned by the Euka user, Euka passes a command-scoped `safe.directory` setting to Git so `staffer` can inspect it. These commands have whatever access `staffer` has; Euka does not yet prevent writes to group-writable paths or elsewhere. Do not treat this as an enforced read-only sandbox.
- Other `name? task` and `name! task` forms are recognized as Euka targets. An unavailable target reports an Euka error instead of being run as a shell command. Target names can contain letters, digits, underscores, hyphens, and periods; the `?` or `!` must end the first word.
- Ctrl-C at the prompt, Ctrl-D to exit, and normal terminal behavior for foreground programs.
- `#` comments and `- [ ]` todos are kept in the in-memory session. A bare HTTP or HTTPS URL loads a text resource in the background, keeps up to 4 MiB in memory, and makes an excerpt available to later agent requests. `HEAD https://...` displays response headers and the elapsed time to receive them; individual DNS, TCP, and TLS timings are not available through the current HTTP client. Euka uses one worker thread per origin and reuses its HTTP client between requests. Redirects are reported for now; enter the destination URL to load it. Other future syntax is reserved.
- `+ bash? command` and `+ zsh? command` run a shell command as `staffer`, keep its output in shared context, and refresh it every two seconds. For example, `+ bash? ls` and `+ bash? git status` both work. They use noninteractive `sudo`, so configure passwordless access for `staffer` first; errors appear at the prompt. Register different commands or directories to watch them together. Each command is run again on every refresh.
- `+` refreshes every registered live command and shows a line diff against the previous `+` run (or the first completed result).

On macOS with Apple's `fm` CLI available and its license accepted:

```text
fm? explain how src/session.rs classifies input
fm! create a file named note.txt containing hello
fm?
explain the input parser
.
```

`fm?` can list directories and read files under the current working directory. `fm!` can also write files and run commands. Euka checks tool access for every step; a read-only request cannot use the write or run tools. Repeated tool calls are skipped, and Euka requires a final answer when the step budget is exhausted. Each request runs in the background, and its answer appears at the prompt when ready. Requests share the in-memory comments, todos, command names and exit statuses, and earlier agent answers. Command output is not yet captured in the session. The bare `?` and `!` forms remain reserved until a default model is defined.

With an installed and authenticated agent CLI, `claude?`, `codex?`, and `pi?` run in the background with reading tools. Their `!` forms, including `codex! task`, can edit files and run commands. The `opus`, `sonnet`, `sol`, `luna`, and `terra` aliases support the same suffixes. Agents normally run as the current Unix user. Prefix a CLI agent request with `@staffer` to run it as the `staffer` Unix user, for example `@staffer opus? Is there a LICENSE?`. This uses noninteractive `sudo`; `staffer` needs access to the CLI, its own authentication, and permission to read the project. The prefix does not support `fm` because its tools run inside Euka. Codex uses its `read-only` or `workspace-write` sandbox according to the suffix. For `codex?` and its aliases, Euka ignores Codex's user config so an automatic approval setting cannot override read-only mode. Claude uses restricted mode and an explicit tool list; Pi uses an explicit tool list. On macOS, Euka also runs CLI `?` requests under a system sandbox that blocks writes to the current Git working tree, or the current directory outside a Git repository. This protects project files and the Git index even if an agent's own tool restrictions fail. It does not block writes outside that tree. On Linux, CLI `?` requests still rely on each CLI's restrictions. Euka passes its in-memory session context to each agent.

Agent requests display a stable label such as `codex#3: …`, followed by `codex#3: answer` when complete. Since request numbers are unique across agents, enter `#3` or `codex#3` to show the full stored answer. Mention either form in another agent request, such as `claude? do you agree with #3?`, to include that full answer in its context. A reference to an unfinished request reports that it is still working. Labels last for the current in-memory session.

Use a slash-separated prefix to ask several agents the same read-only question at once, for example `opus/luna? what is a modern alternative to tree?`. Each agent runs in the background and gets its own reply label.

Entering bare `fm?` switches the prompt to `fm?>`; subsequent lines are read-only fm requests. Enter `.`, type `exit`, or press Ctrl-C to return to the shell prompt. Running agent requests keep working across the switch.

Euka intentionally does not implement Bash control syntax, globbing, or persistence. Shell commands run directly without an LLM.
