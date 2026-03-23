use std::io::IsTerminal;
use std::sync::Arc;
use wdttg::ui::{
    edit_task, select_em_quadrant, EditFormResult, EmSelectorResult, PickerResult, TaskPicker,
};
use wdttg::{
    cli, parse_relative_time, Commands, Database, EmQuadrant, SessionService, TaskService,
    TaskStatus,
};

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

    let (task, is_new_task) = if let Some(query_str) = query {
        if !query_str.is_empty() {
            match task_service
                .get_by_name(query_str)
                .map_err(|e| e.to_string())?
            {
                Some(existing) => (existing, false),
                None => {
                    let created = task_service
                        .get_or_create(query_str)
                        .map_err(|e| e.to_string())?;
                    (created, true)
                }
            }
        } else if is_interactive() && !no_prompt {
            let tasks = task_service
                .get_pickable_tasks()
                .map_err(|e| e.to_string())?;
            let picker = TaskPicker::new(tasks);
            match picker.run().map_err(|e| e.to_string())? {
                PickerResult::Selected(t) => (t, false),
                PickerResult::CreateNew(name) => {
                    let created = task_service
                        .get_or_create(&name)
                        .map_err(|e| e.to_string())?;
                    (created, true)
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
        let tasks = task_service
            .get_pickable_tasks()
            .map_err(|e| e.to_string())?;
        let picker = TaskPicker::new(tasks);
        match picker.run().map_err(|e| e.to_string())? {
            PickerResult::Selected(t) => (t, false),
            PickerResult::CreateNew(name) => {
                let created = task_service
                    .get_or_create(&name)
                    .map_err(|e| e.to_string())?;
                (created, true)
            }
            PickerResult::Cancelled => {
                println!("Cancelled.");
                return Ok(());
            }
        }
    } else {
        return Err("Task name required in non-interactive mode".to_string());
    };

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

fn handle_edit(
    query: Option<&str>,
    name: Option<&str>,
    notes: Option<&str>,
    em: Option<&str>,
    status: Option<&str>,
    recurring: Option<bool>,
    no_prompt: bool,
    db: &Arc<Database>,
) -> Result<(), String> {
    let task_service = TaskService::new(Arc::clone(db));

    if no_prompt || !is_interactive() {
        if let Some(query_str) = query {
            if !query_str.is_empty() {
                let task = task_service
                    .get_by_name(query_str)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("Task not found: {}", query_str))?;

                let mut updated_task = task.clone();

                if let Some(new_name) = name {
                    updated_task.name = new_name.to_string();
                }

                if let Some(new_notes) = notes {
                    updated_task.notes = if new_notes.is_empty() {
                        None
                    } else {
                        Some(new_notes.to_string())
                    };
                }

                if let Some(em_str) = em {
                    let quadrant = parse_em_arg(em_str)?;
                    updated_task.em_urgent = quadrant.is_urgent();
                    updated_task.em_important = quadrant.is_important();
                }

                if let Some(status_str) = status {
                    updated_task.status =
                        TaskStatus::from_str(status_str).map_err(|e| e.to_string())?;
                }

                if let Some(recur) = recurring {
                    updated_task.recurring = recur;
                }

                task_service
                    .update(&updated_task)
                    .map_err(|e| e.to_string())?;

                println!("Updated: {}", updated_task.name);
                return Ok(());
            }
        }
        return Err("Task name required in non-interactive mode".to_string());
    }

    let task = if let Some(query_str) = query {
        if !query_str.is_empty() {
            task_service
                .get_by_name(query_str)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Task not found: {}", query_str))?
        } else {
            let non_archived = task_service
                .get_non_archived_tasks()
                .map_err(|e| e.to_string())?;

            if non_archived.is_empty() {
                return Err("No tasks to edit".to_string());
            }

            let picker = TaskPicker::new(non_archived);
            match picker.run().map_err(|e| e.to_string())? {
                PickerResult::Selected(t) => t,
                PickerResult::CreateNew(_) => {
                    return Err("Cannot create new task in edit. Use start command.".to_string());
                }
                PickerResult::Cancelled => {
                    println!("Cancelled.");
                    return Ok(());
                }
            }
        }
    } else {
        let non_archived = task_service
            .get_non_archived_tasks()
            .map_err(|e| e.to_string())?;

        if non_archived.is_empty() {
            return Err("No tasks to edit".to_string());
        }

        let picker = TaskPicker::new(non_archived);
        match picker.run().map_err(|e| e.to_string())? {
            PickerResult::Selected(t) => t,
            PickerResult::CreateNew(_) => {
                return Err("Cannot create new task in edit. Use start command.".to_string());
            }
            PickerResult::Cancelled => {
                println!("Cancelled.");
                return Ok(());
            }
        }
    };

    let result = edit_task(task.clone()).map_err(|e| e.to_string())?;

    match result {
        EditFormResult::Saved(updated_task) => {
            let mut final_task = task.clone();
            final_task.name = updated_task.name;
            final_task.notes = updated_task.notes;
            final_task.em_urgent = updated_task.em_urgent;
            final_task.em_important = updated_task.em_important;
            final_task.status = updated_task.status;
            final_task.recurring = updated_task.recurring;

            task_service
                .update(&final_task)
                .map_err(|e| e.to_string())?;

            println!("Updated: {}", final_task.name);
        }
        EditFormResult::Cancelled => {
            println!("Cancelled.");
        }
    }

    Ok(())
}

fn handle_log(
    task: Option<&str>,
    start_arg: Option<&str>,
    end_arg: Option<&str>,
    no_prompt: bool,
    db: &Arc<Database>,
) -> Result<(), String> {
    let task_service = TaskService::new(Arc::clone(db));
    let session_service = SessionService::new(Arc::clone(db));

    let task = if let Some(task_name) = task {
        if !task_name.is_empty() {
            match task_service
                .get_by_name(task_name)
                .map_err(|e| e.to_string())?
            {
                Some(existing) => existing,
                None => task_service
                    .get_or_create(task_name)
                    .map_err(|e| e.to_string())?,
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

    let now = chrono::Utc::now();

    let start = if let Some(start_str) = start_arg {
        parse_relative_time(start_str).map_err(|e| e.to_string())?
    } else if is_interactive() && !no_prompt {
        print!("Start time (e.g., '2h ago', '14:30', 'yesterday 09:00'): ");
        use std::io::{self, Write};
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        parse_relative_time(input.trim()).map_err(|e| e.to_string())?
    } else {
        return Err("Start time required in non-interactive mode".to_string());
    };

    let end = if let Some(end_str) = end_arg {
        parse_relative_time(end_str).map_err(|e| e.to_string())?
    } else if is_interactive() && !no_prompt {
        print!("End time (e.g., '1h ago', '15:00', or press Enter for now'): ");
        use std::io::{self, Write};
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        let input = input.trim();
        if input.is_empty() {
            now
        } else {
            parse_relative_time(input).map_err(|e| e.to_string())?
        }
    } else {
        return Err("End time required in non-interactive mode".to_string());
    };

    if end <= start {
        return Err("End time must be after start time".to_string());
    }

    if end > now {
        return Err("End time cannot be in the future".to_string());
    }

    let session = session_service
        .log_session(task.id, start, end)
        .map_err(|e| e.to_string())?;

    let duration = end - start;
    let duration_str = format_duration(duration);
    let start_time = start.format("%H:%M");
    let end_time = end.format("%H:%M");

    println!(
        "Logged: {} ({}) from {} to {}",
        task.name, duration_str, start_time, end_time
    );
    println!("  Session ID: {}", session.id);

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
        } => handle_edit(
            query.as_deref(),
            name.as_deref(),
            notes.as_deref(),
            em.as_deref(),
            status.as_deref(),
            recurring,
            cli.no_prompt,
            &db,
        ),
        Commands::Log { task, start, end } => handle_log(
            task.as_deref(),
            start.as_deref(),
            end.as_deref(),
            cli.no_prompt,
            &db,
        ),
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
