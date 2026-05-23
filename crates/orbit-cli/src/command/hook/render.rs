use clap::ValueEnum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum HookOutputFormat {
    Claude,
    Codex,
    Gemini,
    Grok,
}

impl From<HookOutputFormat> for orbit_core::command::learning_hook::HookOutputFormat {
    fn from(format: HookOutputFormat) -> Self {
        match format {
            HookOutputFormat::Claude => Self::Claude,
            HookOutputFormat::Codex => Self::Codex,
            HookOutputFormat::Gemini => Self::Gemini,
            HookOutputFormat::Grok => Self::Grok,
        }
    }
}
