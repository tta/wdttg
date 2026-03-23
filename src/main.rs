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

fn handle_status(db: &Arc<Database>) -> Result<(), String> {
    let session_service = SessionService::new(Arc::clone(db));

    let active = session_service.get_active().map_err(|e| e.to_string())?;

    match active {
        Some(session_with_task) => {
            let task = &session_with_task.task;
            let session = &session_with_task.session;

            let em_tag = task.em_quadrant().tag();
            let start_time = session.start_time.format("%H:%M");

            let now = chrono::Utc::now();
            let duration = now - session.start_time;
            let duration_str = format_duration(duration);

            println!("Active: {} {}", task.name, em_tag);
            println!("Started: {} ({} ago)", start_time, duration_str);
        }
        None => {
            println!("No active timer");
        }
    }

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

fn handle_done(db: &Arc<Database>) -> Result<(), String> {
    let session_service = SessionService::new(Arc::clone(db));
    let task_service = TaskService::new(Arc::clone(db));

    let last_session = session_service
        .get_last_session_task()
        .map_err(|e| e.to_string())?;

    match last_session {
        Some(session_with_task) => {
            let task = &session_with_task.task;

            task_service
                .set_status(task.id, wdttg::TaskStatus::Completed)
                .map_err(|e| e.to_string())?;

            println!("Completed: {}", task.name);
        }
        None => {
            return Err("No sessions found. Start tracking time first.".to_string());
        }
    }

    Ok(())
}

fn handle_reopen(query: Option<&str>, no_prompt: bool, db: &Arc<Database>) -> Result<(), String> {
    let task_service = TaskService::new(Arc::clone(db));

    let task = if let Some(query_str) = query {
        if !query_str.is_empty() {
            task_service
                .get_by_name(query_str)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Task not found: {}", query_str))?
        } else if is_interactive() && !no_prompt {
            let completed_tasks = task_service
                .search_completed("")
                .map_err(|e| e.to_string())?;

            if completed_tasks.is_empty() {
                return Err("No completed tasks to reopen".to_string());
            }

            let picker = TaskPicker::new(completed_tasks);
            match picker.run().map_err(|e| e.to_string())? {
                PickerResult::Selected(t) => t,
                PickerResult::CreateNew(_) => {
                    return Err("Cannot create new task in reopen. Use start command.".to_string());
                }
                PickerResult::Cancelled => {
                    println!("Cancelled.");
                    return Ok(());
                }
            }
        } else {
            return Err("Task name required in non-interactive mode".to_string());
        }
    } else if is_interactive() && !no_prompt {
        let completed_tasks = task_service
            .search_completed("")
            .map_err(|e| e.to_string())?;

        if completed_tasks.is_empty() {
            return Err("No completed tasks to reopen".to_string());
        }

        let picker = TaskPicker::new(completed_tasks);
        match picker.run().map_err(|e| e.to_string())? {
            PickerResult::Selected(t) => t,
            PickerResult::CreateNew(_) => {
                return Err("Cannot create new task in reopen. Use start command.".to_string());
            }
            PickerResult::Cancelled => {
                println!("Cancelled.");
                return Ok(());
            }
        }
    } else {
        return Err("Task name required in non-interactive mode".to_string());
    };

    task_service
        .set_status(task.id, wdttg::TaskStatus::Active)
        .map_err(|e| e.to_string())?;

    println!("Reopened: {}", task.name);

    Ok(())
}

fn handle_archive(query: Option<&str>, no_prompt: bool, db: &Arc<Database>) -> Result<(), String> {
    let task_service = TaskService::new(Arc::clone(db));

    let task = if let Some(query_str) = query {
        if !query_str.is_empty() {
            task_service
                .get_by_name(query_str)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Task not found: {}", query_str))?
        } else if is_interactive() && !no_prompt {
            let non_archived = task_service
                .get_non_archived_tasks()
                .map_err(|e| e.to_string())?;

            if non_archived.is_empty() {
                return Err("No tasks to archive".to_string());
            }

            let picker = TaskPicker::new(non_archived);
            match picker.run().map_err(|e| e.to_string())? {
                PickerResult::Selected(t) => t,
                PickerResult::CreateNew(_) => {
                    return Err("Cannot create new task in archive. Use start command.".to_string());
                }
                PickerResult::Cancelled => {
                    println!("Cancelled.");
                    return Ok(());
                }
            }
        } else {
            return Err("Task name required in non-interactive mode".to_string());
        }
    } else if is_interactive() && !no_prompt {
        let non_archived = task_service
            .get_non_archived_tasks()
            .map_err(|e| e.to_string())?;

        if non_archived.is_empty() {
            return Err("No tasks to archive".to_string());
        }

        let picker = TaskPicker::new(non_archived);
        match picker.run().map_err(|e| e.to_string())? {
            PickerResult::Selected(t) => t,
            PickerResult::CreateNew(_) => {
                return Err("Cannot create new task in archive. Use start command.".to_string());
            }
            PickerResult::Cancelled => {
                println!("Cancelled.");
                return Ok(());
            }
        }
    } else {
        return Err("Task name required in non-interactive mode".to_string());
    };

    task_service
        .set_status(task.id, wdttg::TaskStatus::Archived)
        .map_err(|e| e.to_string())?;

    println!("Archived: {}", task.name);

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
        Commands::Status => handle_status(&db),
        Commands::Done => handle_done(&db),
        Commands::Reopen { query } => handle_reopen(query.as_deref(), cli.no_prompt, &db),
        Commands::Archive { query } => handle_archive(query.as_deref(), cli.no_prompt, &db),
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
