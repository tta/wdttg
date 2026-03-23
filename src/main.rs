use wdttg::{Commands, ReportCommands};

fn main() {
    let cli = wdttg::cli::parse();

    match cli.command {
        Commands::Start { query, em } => {
            let query_str = query.as_deref().unwrap_or("");
            let em_str = em.as_deref().unwrap_or("");
            if em_str.is_empty() {
                println!("Start: {} (not implemented)", query_str);
            } else {
                println!("Start: {} --em {} (not implemented)", query_str, em_str);
            }
        }
        Commands::Stop => {
            println!("Stop: not implemented");
        }
        Commands::Status => {
            println!("Status: not implemented");
        }
        Commands::Done => {
            println!("Done: not implemented");
        }
        Commands::Reopen { query } => {
            let query_str = query.as_deref().unwrap_or("");
            println!("Reopen: {} (not implemented)", query_str);
        }
        Commands::Archive { query } => {
            let query_str = query.as_deref().unwrap_or("");
            println!("Archive: {} (not implemented)", query_str);
        }
        Commands::Edit {
            query,
            name,
            notes,
            em,
            status,
            recurring,
        } => {
            let query_str = query.as_deref().unwrap_or("");
            println!("Edit: {} (not implemented)", query_str);
            if name.is_some()
                || notes.is_some()
                || em.is_some()
                || status.is_some()
                || recurring.is_some()
            {
                println!(
                    "  Fields: name={:?}, notes={:?}, em={:?}, status={:?}, recurring={:?}",
                    name, notes, em, status, recurring
                );
            }
        }
        Commands::Log { task, start, end } => {
            println!("Log: not implemented");
            if task.is_some() || start.is_some() || end.is_some() {
                println!("  task={:?}, start={:?}, end={:?}", task, start, end);
            }
        }
        Commands::Report { report_type } => match report_type {
            ReportCommands::Quadrant { period } => {
                println!("Report quadrant: {} (not implemented)", period);
            }
            ReportCommands::Task { period } => {
                println!("Report task: {} (not implemented)", period);
            }
            ReportCommands::Tag { period } => {
                println!("Report tag: {} (not implemented)", period);
            }
            ReportCommands::Timeline { date } => {
                println!("Report timeline: {} (not implemented)", date);
            }
            ReportCommands::History { limit } => {
                println!("Report history: limit={} (not implemented)", limit);
            }
        },
        Commands::Backup { keep } => {
            println!("Backup: keep={} (not implemented)", keep);
        }
    }
}
