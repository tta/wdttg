use std::io::IsTerminal;
use std::sync::Arc;
use wdttg::ui::{select_em_quadrant, EmSelectorResult, PickerResult, TaskPicker};
use wdttg::{cli, Commands, Database, EmQuadrant, SessionService, TaskService};

fn format_duration(duration: chrono::Duration) -> String {
    let total_minutes = duration.num_minutes();
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;

    if hours > 0 {
        format!("{}h {}m", hours, minutes)
    } else {
        format!("{}m", minutes)
    }
}

fn is_interactive() -> bool {
    std::io::stdout().is_terminal()
}

fn parse_em_arg(em: &str) -> Result<EmQuadrant, String> {
    EmQuadrant::from_str(em)
}

fn prompt_em_quadrant() -> Option<EmQuadrant> {
    match select_em_quadrant() {
        Ok(EmSelectorResult::Selected(quadrant)) => Some(quadrant),
        _ => None,
    }
}

fn handle_start(
    query: Option<&str>,
    em_arg: Option<&str>,
    no_prompt: bool,
    db: &Arc<Database>,
) -> Result<(), String> {
    let task_service = TaskService::new(Arc::clone(db));
    let session_service = SessionService::new(Arc::clone(db));

    let task = if let Some(query_str) = query {
        if !query_str.is_empty() {
            task_service
                .get_or_create(query_str)
                .map_err(|e| e.to_string())?
        } else if is_interactive() && !no_prompt {
            let tasks = task_service
                .get_pickable_tasks()
                .map_err(|e| e.to_string())?;
            let picker = TaskPicker::new(tasks);
            match picker.run().map_err(|e| e.to_string())? {
                PickerResult::Selected(t) => t,
                PickerResult::CreateNew(name) => task_service
                    .get_or_create(&name)
                    .map_err(|e| e.to_string())?,
                PickerResult::Cancelled => {
                    println!("Cancelled.");
                    return Ok(());
                }
            }
        } else {
            return Err("Task name required in non-interactive mode".to_string());
        }
    } else if is_interactive() && !no_prompt {
        let tasks = task_service
            .get_pickable_tasks()
            .map_err(|e| e.to_string())?;
        let picker = TaskPicker::new(tasks);
        match picker.run().map_err(|e| e.to_string())? {
            PickerResult::Selected(t) => t,
            PickerResult::CreateNew(name) => task_service
                .get_or_create(&name)
                .map_err(|e| e.to_string())?,
            PickerResult::Cancelled => {
                println!("Cancelled.");
                return Ok(());
            }
        }
    } else {
        return Err("Task name required in non-interactive mode".to_string());
    };

    let is_new_task = !task.em_urgent && !task.em_important;

    if is_new_task {
        let quadrant = if let Some(em_str) = em_arg {
            parse_em_arg(em_str)?
        } else if is_interactive() && !no_prompt {
            match prompt_em_quadrant() {
                Some(q) => q,
                None => {
                    println!("Cancelled.");
                    return Ok(());
                }
            }
        } else {
            return Err("EM quadrant required for new tasks. Use --em 00|01|10|11".to_string());
        };

        task_service
            .set_em_quadrant(task.id, quadrant.is_urgent(), quadrant.is_important())
            .map_err(|e| e.to_string())?;
    }

    let session = session_service.start(task.id).map_err(|e| e.to_string())?;

    let updated_task = task_service.get_by_id(task.id).unwrap().unwrap();
    let em_tag = updated_task.em_quadrant().tag();

    println!("Started: {} {}", updated_task.name, em_tag);
    println!("  Session ID: {}", session.id);

    Ok(())
}

fn handle_stop(db: &Arc<Database>) -> Result<(), String> {
    let session_service = SessionService::new(Arc::clone(db));

    let active = session_service.get_active().map_err(|e| e.to_string())?;

    if active.is_none() {
        println!("No active timer");
        return Ok(());
    }

    let active = active.unwrap();
    let task_name = active.task.name.clone();
    let session_id = active.session.id;

    let stopped = session_service.stop().map_err(|e| e.to_string())?;

    if let Some(session) = stopped {
        let duration = session.end_time.unwrap() - session.start_time;
        let duration_str = format_duration(duration);

        println!("Stopped: {} ({})", task_name, duration_str);
        println!("  Session ID: {}", session_id);
    } else {
        println!("No active timer");
    }

    Ok(())
}

fn main() {
    let cli = cli::parse();

    let db = match Database::open() {
        Ok(d) => Arc::new(d),
        Err(e) => {
            eprintln!("Error opening database: {}", e);
            std::process::exit(1);
        }
    };

    let result = match cli.command {
        Commands::Start { query, em } => {
            handle_start(query.as_deref(), em.as_deref(), cli.no_prompt, &db)
        }
        Commands::Stop => handle_stop(&db),
        Commands::Status => {
            println!("Status: not implemented");
            Ok(())
        }
        Commands::Done => {
            println!("Done: not implemented");
            Ok(())
        }
        Commands::Reopen { query } => {
            let query_str = query.as_deref().unwrap_or("");
            println!("Reopen: {} (not implemented)", query_str);
            Ok(())
        }
        Commands::Archive { query } => {
            let query_str = query.as_deref().unwrap_or("");
            println!("Archive: {} (not implemented)", query_str);
            Ok(())
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
            Ok(())
        }
        Commands::Log { task, start, end } => {
            println!("Log: not implemented");
            if task.is_some() || start.is_some() || end.is_some() {
                println!("  task={:?}, start={:?}, end={:?}", task, start, end);
            }
            Ok(())
        }
        Commands::Report { report_type } => match report_type {
            wdttg::ReportCommands::Quadrant { period } => {
                println!("Report quadrant: {} (not implemented)", period);
                Ok(())
            }
            wdttg::ReportCommands::Task { period } => {
                println!("Report task: {} (not implemented)", period);
                Ok(())
            }
            wdttg::ReportCommands::Tag { period } => {
                println!("Report tag: {} (not implemented)", period);
                Ok(())
            }
            wdttg::ReportCommands::Timeline { date } => {
                println!("Report timeline: {} (not implemented)", date);
                Ok(())
            }
            wdttg::ReportCommands::History { limit } => {
                println!("Report history: limit={} (not implemented)", limit);
                Ok(())
            }
        },
        Commands::Backup { keep } => {
            println!("Backup: keep={} (not implemented)", keep);
            Ok(())
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
