//! Clap shapes for the stencilwright command tree.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "stencilwright",
    version,
    about = "privacy-preserving site mapping harness",
    after_help = "Site resource commands use: stencilwright <site> config|place|element|page|value ..."
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// Scaffold ~/.stencilwright/<site>/ with default TOML artifacts.
    Init { site: String },

    /// Import ./stencils/<site>/, or an explicit source path, into ~/.stencilwright/<site>/.
    Load {
        site: String,
        source: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },

    /// Daemon lifecycle (rarely invoked directly).
    Session {
        #[command(subcommand)]
        op: SessionOp,
    },

    /// Hidden: detached daemon entry point invoked by `session start`.
    #[command(hide = true)]
    Daemon { site_dir: PathBuf },

    /// Site-first resource commands: `<site> config|place|element|page|value ...`.
    #[command(external_subcommand)]
    Site(Vec<String>),
}

#[derive(Subcommand, Debug)]
pub(crate) enum SessionOp {
    Start { site: String },
    Stop { site: String },
    Status { site: String },
}

#[derive(Parser, Debug)]
pub(crate) struct SiteCli {
    pub(crate) site: String,
    #[command(subcommand)]
    pub(crate) command: SiteCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum SiteCommand {
    Config {
        #[command(subcommand)]
        op: ConfigCommand,
    },
    Place {
        #[command(subcommand)]
        op: PlaceCommand,
    },
    Element {
        #[command(subcommand)]
        op: SiteElementCommand,
    },
    Page {
        #[command(subcommand)]
        op: PageCommand,
    },
    Value {
        #[command(subcommand)]
        op: ValueCommand,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ConfigCommand {
    Show,
    Set(ConfigSetArgs),
}

#[derive(Args, Debug)]
pub(crate) struct ConfigSetArgs {
    #[arg(
        long = "onepassword-account",
        conflicts_with = "clear_onepassword_account"
    )]
    pub(crate) onepassword_account: Option<String>,
    #[arg(long = "clear-onepassword-account")]
    pub(crate) clear_onepassword_account: bool,
}

#[derive(Subcommand, Debug)]
pub(crate) enum PlaceCommand {
    Add {
        name: String,
        selector: Option<String>,
    },
    List,
    #[command(external_subcommand)]
    Target(Vec<String>),
}

#[derive(Parser, Debug)]
pub(crate) struct PlaceTargetCli {
    pub(crate) name: String,
    #[command(subcommand)]
    pub(crate) command: PlaceTargetCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum PlaceTargetCommand {
    Goto,
    Set(PlaceSetArgs),
    Signature {
        #[command(subcommand)]
        op: SignatureCommand,
    },
    Completion {
        #[command(subcommand)]
        op: CompletionCommand,
    },
    Element {
        #[command(subcommand)]
        op: PlaceElementCommand,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum SignatureCommand {
    #[command(override_usage = "stencilwright <site> place <NAME> signature set [OPTIONS]")]
    Set(SignatureSetArgs),
}

#[derive(Subcommand, Debug)]
pub(crate) enum CompletionCommand {
    #[command(override_usage = "stencilwright <site> place <NAME> completion set [OPTIONS]")]
    Set(SignatureSetArgs),
    #[command(override_usage = "stencilwright <site> place <NAME> completion clear")]
    Clear,
}

#[derive(Args, Debug)]
pub(crate) struct PlaceSetArgs {
    #[arg(long)]
    pub(crate) interactive: Option<bool>,
    #[arg(long)]
    pub(crate) url: Option<String>,
    #[arg(long)]
    pub(crate) clear_url: bool,
    #[arg(long)]
    pub(crate) redirect: Option<String>,
    #[arg(long)]
    pub(crate) clear_redirect: bool,
    #[arg(long)]
    pub(crate) submit_click: Option<String>,
    #[arg(long)]
    pub(crate) clear_submit: bool,
}

#[derive(Args, Debug)]
pub(crate) struct SignatureSetArgs {
    #[arg(long)]
    pub(crate) url: Option<String>,
    #[arg(long)]
    pub(crate) selector: Option<String>,
    #[arg(long)]
    pub(crate) visible_selector: Option<String>,
    #[arg(long)]
    pub(crate) absent_selector: Option<String>,
    #[arg(long)]
    pub(crate) text: Option<String>,
    #[arg(long)]
    pub(crate) clear_url: bool,
    #[arg(long)]
    pub(crate) clear_selector: bool,
    #[arg(long)]
    pub(crate) clear_visible_selector: bool,
    #[arg(long)]
    pub(crate) clear_absent_selector: bool,
    #[arg(long)]
    pub(crate) clear_text: bool,
}

#[derive(Subcommand, Debug)]
pub(crate) enum PlaceElementCommand {
    Add(ElementAddArgs),
    Unmask {
        selector: String,
        #[arg(long)]
        reason: Option<String>,
    },
    List,
}

#[derive(Subcommand, Debug)]
pub(crate) enum SiteElementCommand {
    Add(ElementAddArgs),
    List,
}

#[derive(Args, Debug)]
pub(crate) struct ElementAddArgs {
    pub(crate) selector: String,
    #[arg(long = "as")]
    pub(crate) name: Option<String>,
    #[arg(long = "auto-fill")]
    pub(crate) auto_fill: Option<String>,
    #[arg(long)]
    pub(crate) unmasked: bool,
    #[arg(long)]
    pub(crate) reason: Option<String>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum PageCommand {
    Goto {
        url: String,
    },
    Click {
        selector: String,
        /// Bypass actionability checks (for present-but-unclickable controls).
        #[arg(long)]
        force: bool,
    },
    Fill {
        selector: String,
        value: String,
    },
    /// Press a single key (e.g. Enter) on a selector.
    Press {
        selector: String,
        key: String,
    },
    /// Type text with real per-character key events (rich editors).
    Type {
        selector: String,
        text: String,
    },
    /// Press a key on the page's focused element (e.g. Enter), no selector.
    Key {
        key: String,
    },
    Dump,
}

#[derive(Subcommand, Debug)]
pub(crate) enum ValueCommand {
    Add { name: String, reference: String },
    Search(ValueSearchArgs),
    List,
    Remove { name: String },
}

#[derive(Args, Debug)]
pub(crate) struct ValueSearchArgs {
    /// Keyword or URL fragment to match against item title, vault, category, and URLs.
    pub(crate) query: String,
    #[arg(long, default_value_t = 10)]
    pub(crate) limit: usize,
    #[arg(long = "category")]
    pub(crate) categories: Vec<String>,
    #[arg(long)]
    pub(crate) vault: Option<String>,
    #[arg(long)]
    pub(crate) all_categories: bool,
}
