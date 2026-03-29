use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "wdttg")]
#[command(about = "Where Did The Time Go - A time tracker with EM quadrant support")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    #[arg(long, global = true, help = "Disable interactive prompts")]
    pub no_prompt: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    #[clap(about = "Start tracking time on a task")]
    Start {
        #[arg(help = "Task name or search query")]
        query: Option<String>,

        #[arg(long, help = "Eisenhower Matrix quadrant (00, 01, 10, 11)")]
        em: Option<String>,

        #[arg(long, help = "Link to external resource (URL or file path)")]
        link: Option<String>,

        #[arg(long, help = "Run as background task")]
        bg: bool,
    },

    #[clap(about = "Stop current timer")]
    Stop {
        #[arg(long, help = "Stop all active timers (foreground and background)")]
        all: bool,

        #[arg(short, long, help = "Stop timer for specific task")]
        task: Option<String>,
    },

    #[clap(about = "Show current timer status")]
    Status,

    #[clap(about = "Mark last task as completed")]
    Done,

    #[clap(about = "Reopen a completed task")]
    Reopen {
        #[arg(help = "Task name or search query")]
        query: Option<String>,
    },

    #[clap(about = "Archive a task")]
    Archive {
        #[arg(help = "Task name or search query")]
        query: Option<String>,
    },

    #[clap(about = "Edit task metadata")]
    Edit {
        #[arg(help = "Task name or search query")]
        query: Option<String>,

        #[arg(long, help = "New task name")]
        name: Option<String>,

        #[arg(long, help = "Task notes")]
        notes: Option<String>,

        #[arg(long, help = "Eisenhower Matrix quadrant (00, 01, 10, 11)")]
        em: Option<String>,

        #[arg(long, help = "Task status (active, completed, archived)")]
        status: Option<String>,

        #[arg(long, help = "Mark as recurring task")]
        recurring: Option<bool>,
    },

    #[clap(about = "Add retrospective time entry")]
    Log {
        #[arg(long, help = "Task name")]
        task: Option<String>,

        #[arg(long, help = "Start time (YYYY-MM-DD HH:MM or relative like '2h ago')")]
        start: Option<String>,

        #[arg(
            long,
            help = "End time (YYYY-MM-DD HH:MM or relative like '1h ago'). Omit for ongoing session"
        )]
        end: Option<String>,

        #[arg(long, help = "Eisenhower Matrix quadrant (00, 01, 10, 11)")]
        em: Option<String>,

        #[arg(long, help = "Link to external resource (URL or file path)")]
        link: Option<String>,
    },

    #[clap(about = "Show timeline for a specific date (today, yesterday, YYYY-MM-DD)")]
    Timeline {
        #[arg(
            default_value = "today",
            help = "Date to show (today, yesterday, YYYY-MM-DD)"
        )]
        date: String,
    },

    #[clap(about = "Generate time reports")]
    Report {
        #[command(subcommand)]
        report_type: ReportCommands,
    },

    #[clap(about = "Create database backup")]
    Backup {
        #[arg(long, default_value = "10", help = "Number of backups to keep")]
        keep: usize,
    },

    #[clap(about = "Generate shell completions")]
    Completions {
        #[arg(help = "Shell type (bash, zsh, fish, elvish)")]
        shell: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum ReportCommands {
    #[clap(about = "Time by Eisenhower Matrix quadrant")]
    Quadrant {
        #[arg(default_value = "week", help = "Time period (day, week, month)")]
        period: String,
    },

    #[clap(about = "Time by task")]
    Task {
        #[arg(default_value = "week", help = "Time period (day, week, month)")]
        period: String,
    },

    #[clap(about = "Time by tag")]
    Tag {
        #[arg(default_value = "week", help = "Time period (day, week, month)")]
        period: String,
    },

    #[clap(about = "Browse sessions by tag")]
    History {
        #[arg(help = "Filter by tag (optional)")]
        tag: Option<String>,

        #[arg(long, default_value = "week", help = "Time period (day, week, month)")]
        period: String,

        #[arg(
            long,
            default_value = "20",
            help = "Maximum number of sessions to show"
        )]
        limit: usize,
    },
}

pub fn parse() -> Cli {
    Cli::parse()
}
