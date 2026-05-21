use std::path::Path;

use clap::{Args, ValueEnum};
use orbit_core::OrbitError;

use super::dispatch::{print_action_summary, run_action};
use super::workspace::{env_home_dir, resolve_workspace_layout};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum ScopeArg {
    /// Write to user-level config (~/.claude, ~/.codex, ~/.gemini, ~/.grok).
    Home,
    /// Write to repo-local config (.mcp.json, .codex/, .gemini/, .grok/). Default.
    #[default]
    Workspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub(super) enum McpProvider {
    Claude,
    Codex,
    Gemini,
    Grok,
    Cursor,
    Vscode,
    Windsurf,
}

impl McpProvider {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Grok => "grok",
            Self::Cursor => "cursor",
            Self::Vscode => "vscode",
            Self::Windsurf => "windsurf",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum McpAction {
    Init,
    Remove,
}

impl McpAction {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Init => "init",
            Self::Remove => "remove",
        }
    }
}

#[derive(Args, Debug, Clone, Default)]
pub struct ProviderSelectionArgs {
    /// Use auto-detected provider targets for the current workspace.
    #[arg(long)]
    pub auto: bool,
    /// Target a supported MCP client integration. Can be repeated.
    #[arg(long = "client", value_enum, value_name = "CLIENT")]
    clients: Vec<McpProvider>,
    /// Target Claude Code integration only.
    #[arg(long)]
    pub claude: bool,
    /// Target Codex CLI integration only.
    #[arg(long)]
    pub codex: bool,
    /// Target Gemini CLI integration only.
    #[arg(long)]
    pub gemini: bool,
    /// Target Grok Build integration only.
    #[arg(long)]
    pub grok: bool,
    /// Target Cursor integration only.
    #[arg(long)]
    pub cursor: bool,
    /// Target VS Code integration only.
    #[arg(long)]
    pub vscode: bool,
    /// Target Windsurf integration only.
    #[arg(long)]
    pub windsurf: bool,
    /// Target all supported MCP client integrations.
    #[arg(long)]
    pub all: bool,
}

impl ProviderSelectionArgs {
    fn any_explicit_provider(&self) -> bool {
        !self.clients.is_empty()
            || self.claude
            || self.codex
            || self.gemini
            || self.grok
            || self.cursor
            || self.vscode
            || self.windsurf
    }

    fn resolve_mode(&self) -> Result<ProviderSelectionMode, OrbitError> {
        if self.auto && (self.any_explicit_provider() || self.all) {
            return Err(OrbitError::InvalidInput(
                "--auto cannot be combined with --client, --claude, --codex, --gemini, --grok, --cursor, --vscode, --windsurf, or --all".to_string(),
            ));
        }
        if self.all && self.any_explicit_provider() {
            return Err(OrbitError::InvalidInput(
                "--all cannot be combined with --client, --claude, --codex, --gemini, --grok, --cursor, --vscode, or --windsurf".to_string(),
            ));
        }
        if self.auto || (!self.any_explicit_provider() && !self.all) {
            return Ok(ProviderSelectionMode::Auto);
        }
        if self.all {
            return Ok(ProviderSelectionMode::Explicit(vec![
                McpProvider::Claude,
                McpProvider::Codex,
                McpProvider::Gemini,
                McpProvider::Grok,
                McpProvider::Cursor,
                McpProvider::Vscode,
                McpProvider::Windsurf,
            ]));
        }

        let mut providers = Vec::new();
        for provider in [
            McpProvider::Claude,
            McpProvider::Codex,
            McpProvider::Gemini,
            McpProvider::Grok,
            McpProvider::Cursor,
            McpProvider::Vscode,
            McpProvider::Windsurf,
        ] {
            if self.explicit_provider_requested(provider) {
                providers.push(provider);
            }
        }
        Ok(ProviderSelectionMode::Explicit(providers))
    }

    fn explicit_provider_requested(&self, provider: McpProvider) -> bool {
        self.clients.contains(&provider)
            || match provider {
                McpProvider::Claude => self.claude,
                McpProvider::Codex => self.codex,
                McpProvider::Gemini => self.gemini,
                McpProvider::Grok => self.grok,
                McpProvider::Cursor => self.cursor,
                McpProvider::Vscode => self.vscode,
                McpProvider::Windsurf => self.windsurf,
            }
    }
}

pub(super) enum ProviderSelectionMode {
    Auto,
    Explicit(Vec<McpProvider>),
}

#[derive(Args, Debug, Clone, Default)]
#[command(about = "Initialize MCP client integration for the current workspace")]
pub struct InitArgs {
    #[command(flatten)]
    pub providers: ProviderSelectionArgs,
    /// Scope for written config files (workspace: repo-local, home: user-level).
    #[arg(long, value_enum, default_value_t = ScopeArg::Workspace)]
    pub scope: ScopeArg,
}

impl InitArgs {
    pub fn execute_without_runtime(self, root_override: Option<&Path>) -> Result<(), OrbitError> {
        let layout = resolve_workspace_layout(root_override)?;
        let providers = run_action(
            McpAction::Init,
            &layout.repo_root,
            &layout.orbit_root,
            self.providers.resolve_mode()?,
            env_home_dir(),
            self.scope,
        )?;
        print_action_summary(McpAction::Init, &providers);
        Ok(())
    }
}

#[derive(Args, Debug, Clone, Default)]
#[command(about = "Remove MCP client integration for the current workspace")]
pub struct RemoveArgs {
    #[command(flatten)]
    pub providers: ProviderSelectionArgs,
    /// Scope for config files to remove (workspace: repo-local, home: user-level).
    #[arg(long, value_enum, default_value_t = ScopeArg::Workspace)]
    pub scope: ScopeArg,
}

impl RemoveArgs {
    pub fn execute_without_runtime(self, root_override: Option<&Path>) -> Result<(), OrbitError> {
        let layout = resolve_workspace_layout(root_override)?;
        let providers = run_action(
            McpAction::Remove,
            &layout.repo_root,
            &layout.orbit_root,
            self.providers.resolve_mode()?,
            env_home_dir(),
            self.scope,
        )?;
        print_action_summary(McpAction::Remove, &providers);
        Ok(())
    }
}

pub(crate) fn init_auto_for_workspace(
    repo_root: &Path,
    orbit_root: &Path,
) -> Result<Vec<String>, OrbitError> {
    // `orbit workspace init` is a per-workspace setup, so its auto-MCP path
    // writes repo-local files. `orbit mcp init` defaults to workspace scope
    // as well; pass `--scope home` for a user-level registration.
    run_action(
        McpAction::Init,
        repo_root,
        orbit_root,
        ProviderSelectionMode::Auto,
        env_home_dir(),
        ScopeArg::Workspace,
    )
    .map(|providers| {
        providers
            .into_iter()
            .map(|provider| provider.label().to_string())
            .collect()
    })
}

