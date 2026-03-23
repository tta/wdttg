use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "wdttg")]
#[command(about = "Where Did The Time Go - A time tracker with EM quadrant support")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    #[arg(long, global = true)]
    pub no_prompt: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Start {
        query: Option<String>,

        #[arg(long)]
        em: Option<String>,
    },

    Stop,

    Status,

    Done,

    Reopen {
        query: Option<String>,
    },

    Archive {
        query: Option<String>,
    },

    Edit {
        query: Option<String>,

        #[arg(long)]
        name: Option<String>,

        #[arg(long)]
        notes: Option<String>,

        #[arg(long)]
        em: Option<String>,

        #[arg(long)]
        status: Option<String>,

        #[arg(long)]
        recurring: Option<bool>,
    },

    Log {
        #[arg(long)]
        task: Option<String>,

        #[arg(long)]
        start: Option<String>,

        #[arg(long)]
        end: Option<String>,
    },

    Report {
        #[command(subcommand)]
        report_type: ReportCommands,
    },

    Backup {
        #[arg(long, default_value = "10")]
        keep: usize,
    },
}

#[derive(Subcommand, Debug)]
pub enum ReportCommands {
    Quadrant {
        #[arg(default_value = "week")]
        period: String,
    },

    Task {
        #[arg(default_value = "week")]
        period: String,
    },

    Tag {
        #[arg(default_value = "week")]
        period: String,
    },

    Timeline {
        #[arg(default_value = "today")]
        date: String,
    },

    History {
        tag: Option<String>,

        #[arg(long, default_value = "week")]
        period: String,

        #[arg(long, default_value = "20")]
        limit: usize,
    },
}

pub fn parse() -> Cli {
    Cli::parse()
}
