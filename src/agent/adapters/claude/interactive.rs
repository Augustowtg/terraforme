//! Comandos do modo interativo do Claude Code que o `-p` não informa no
//! `initialize` (ex.: `/help`, `/status`, `/resume`, `/login`).
//!
//! A lista foi tirada do Claude Code 2.1.308 (os comandos visíveis do
//! modo interativo menos os que o `initialize` já traz). Ela completa o
//! catálogo para a lista da `/` mostrar todos os comandos do Claude; a
//! função de cada um continua sendo uma interface do terra; os que o terra
//! não tem abrem a interface original do Claude, na mesma sessão, com o
//! comando já digitado ([`opens_original`]).
//!
//! Os de pagamento e plano ([`BILLING`]) ficam de fora.

/// Os comandos de pagamento e plano, que o terra não mostra nem roda.
pub const BILLING: &[&str] = &[
    "extra-usage",
    "passes",
    "rate-limit-options",
    "stickers",
    "upgrade",
    "usage-credits",
];

/// O comando só existe no modo interativo: roda na interface original.
pub fn opens_original(name: &str) -> bool {
    COMMANDS.iter().any(|(command, _)| *command == name)
}

/// Nome (sem a `/`) e descrição, em ordem alfabética.
pub const COMMANDS: &[(&str, &str)] = &[
    ("add-dir", "Add a new working directory"),
    ("artifacts", "Browse your published and shared artifacts"),
    (
        "autofix-pr",
        "Monitor and autofix any issues with the current PR",
    ),
    (
        "background",
        "Send this session to the background and free the terminal",
    ),
    (
        "branch",
        "Create a branch of the current conversation at this point",
    ),
    ("brief", "Toggle brief-only mode"),
    (
        "btw",
        "Ask a quick side question without interrupting the main conversation",
    ),
    ("cd", "Move this session to a new working directory"),
    (
        "cloud-plugins",
        "Choose whether cloud sessions use the plugins enabled on this machine",
    ),
    (
        "copy",
        "Copy Claude's last response to clipboard (or /copy N for the Nth-latest)",
    ),
    ("daemon", "Manage background services and routines"),
    (
        "design-login",
        "Authorize design-system access for /design-sync with your claude.ai account",
    ),
    ("desktop", "Continue the current session in Claude Desktop"),
    ("diff", "Show the changes made in this session"),
    ("exit", "Exit the REPL"),
    (
        "export",
        "Export the current conversation to a file or clipboard",
    ),
    ("feedback", "Send feedback to Anthropic or report a bug"),
    (
        "fork",
        "Spawn a background agent that inherits the full conversation",
    ),
    ("help", "Show help and available commands"),
    ("hooks", "View hook configurations for tool events"),
    ("ide", "Manage IDE integrations and show status"),
    ("install", "Install Claude Code native build"),
    (
        "install-github-app",
        "Set up Claude GitHub Actions for a repository",
    ),
    ("install-slack-app", "Install the Claude Slack app"),
    ("login", "Sign in with your Anthropic account"),
    ("logout", "Sign out from your Anthropic account"),
    ("loops", "List, create, and delete loops"),
    ("memory", "Edit CLAUDE.md files and memory settings"),
    ("mobile", "Show QR code to download the Claude mobile app"),
    (
        "pause-memory",
        "Pause or resume auto memory for this session",
    ),
    ("permissions", "Manage allow and deny tool permission rules"),
    ("plan", "Enable plan mode or view the current session plan"),
    ("plugin", "Manage Claude Code plugins"),
    (
        "powerup",
        "Discover Claude Code features through quick interactive lessons",
    ),
    ("privacy-settings", "View and update your privacy settings"),
    ("radio", "Listen to Claude FM lo-fi radio"),
    (
        "remote-control",
        "Continue this session from claude.ai or the Claude app",
    ),
    (
        "remote-env",
        "Choose the default environment for cloud agents",
    ),
    ("restart", "Restart Claude Code, keeping the conversation"),
    ("resume", "Resume a previous conversation"),
    ("scroll-speed", "Adjust mouse wheel scroll speed"),
    ("session", "Show cloud session URL and QR code"),
    (
        "setup-bedrock",
        "Reconfigure Amazon Bedrock authentication, region, or model pins",
    ),
    (
        "setup-vertex",
        "Reconfigure Google Vertex AI authentication, project, region, or model pins",
    ),
    ("skills", "List available skills"),
    (
        "status",
        "Show Claude Code status including version, model, account, API connectivity, and tool statuses",
    ),
    ("statusline", "Set up Claude Code's status line UI"),
    (
        "stop",
        "Stop this background session; transcript and worktree are kept",
    ),
    (
        "subtask",
        "Send a subagent off with your full context; its result comes back here",
    ),
    (
        "tasks",
        "View and manage everything running in the background",
    ),
    (
        "teleport",
        "Send this session to the cloud, or resume one from claude.ai",
    ),
    (
        "terminal-setup",
        "Install the Shift+Enter key binding for newlines",
    ),
    ("theme", "Change the theme"),
    ("tui", "Set the terminal UI renderer (default | fullscreen)"),
    (
        "ultraplan",
        "Draft a plan in the cloud and review it in the browser",
    ),
    (
        "version",
        "Show this session's version (autoupdate may have a newer one)",
    ),
    ("voice", "Toggle voice mode"),
    (
        "web-setup",
        "Set up cloud sessions with your GitHub account",
    ),
    (
        "wellbeing",
        "Configure optional break reminders and quiet-hours nudges",
    ),
    ("workflows", "Browse running and completed workflows"),
];

#[cfg(test)]
#[test]
fn billing_is_not_listed() {
    assert!(BILLING.iter().all(|name| !opens_original(name)));
    assert!(opens_original("hooks"));
    assert!(!opens_original("compact"), "o `-p` roda o /compact");
}
