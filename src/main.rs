use clap::CommandFactory;
use clap_complete::{generate, shells::Shell};
use std::io::IsTerminal;
use std::sync::Arc;
use textwrap::wrap;
use wdttg::ui::{
    edit_session, edit_task, print_timeline_static, select_em_quadrant, EditFormResult,
    EmSelectorResult, PickerResult, SessionEditResult, TaskPicker, TimelineView,
};
use wdttg::{
    cli, parse_date, parse_period, parse_relative_time, Commands, Database, EmQuadrant,
    SessionService, Task, TaskService,
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

fn handle_edit_session(session_id: i64, db: &Arc<Database>) -> Result<bool, String> {
    let session_service = SessionService::new(Arc::clone(db));
    let task_service = TaskService::new(Arc::clone(db));

    let session = session_service
        .get_by_id(session_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Session not found: {}", session_id))?;

    let task = task_service
        .get_by_id(session.task_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Task not found: {}", session.task_id))?;

    let result = edit_session(session, task.name.clone()).map_err(|e| e.to_string())?;

    match result {
        SessionEditResult::Saved {
            start,
            end,
            notes,
            link,
        } => {
            session_service
                .update_full(session_id, start, end, notes, link)
                .map_err(|e| e.to_string())?;
            println!("Session updated");
            Ok(true)
        }
        SessionEditResult::Deleted(id) => {
            session_service.delete(id).map_err(|e| e.to_string())?;
            println!("Session deleted");
            Ok(true)
        }
        SessionEditResult::Cancelled => Ok(false),
    }
}

fn parse_em_arg(em: &str) -> Result<EmQuadrant, String> {
    em.parse()
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
    link_arg: Option<&str>,
    bg: bool,
    no_prompt: bool,
    db: &Arc<Database>,
) -> Result<(), String> {
    let task_service = TaskService::new(Arc::clone(db));
    let session_service = SessionService::new(Arc::clone(db));

    if bg
        && session_service
            .get_active_background()
            .map_err(|e| e.to_string())?
            .is_some()
    {
        return Err(
            "A background task is already running. Stop it first with 'wdttg stop --bg'"
                .to_string(),
        );
    }

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

    let quadrant = if let Some(em_str) = em_arg {
        Some(parse_em_arg(em_str)?)
    } else if is_new_task && is_interactive() && !no_prompt {
        match prompt_em_quadrant() {
            Some(q) => Some(q),
            None => {
                println!("Cancelled.");
                return Ok(());
            }
        }
    } else if is_new_task {
        return Err("EM quadrant required for new tasks. Use --em 00|01|10|11".to_string());
    } else {
        None
    };

    if let Some(q) = quadrant {
        task_service
            .set_em_quadrant(task.id, q.is_urgent(), q.is_important())
            .map_err(|e| e.to_string())?;
    }

    let session = if bg {
        session_service
            .start_background(task.id, link_arg)
            .map_err(|e| e.to_string())?
    } else {
        session_service
            .start(task.id, link_arg)
            .map_err(|e| e.to_string())?
    };

    let updated_task = task_service.get_by_id(task.id).unwrap().unwrap();
    let em_tag = updated_task.em_quadrant().tag();
    let bg_label = if bg { " (background)" } else { "" };

    println!("Started{}: {} {}", bg_label, updated_task.name, em_tag);
    println!("  Session ID: {}", session.id);

    Ok(())
}

fn handle_status(db: &Arc<Database>) -> Result<(), String> {
    let session_service = SessionService::new(Arc::clone(db));

    let fg = session_service
        .get_active_foreground()
        .map_err(|e| e.to_string())?;
    let bg = session_service
        .get_active_background()
        .map_err(|e| e.to_string())?;

    match (fg, bg) {
        (None, None) => {
            println!("No active timer");
        }
        (Some(fg_only), None) => {
            let task = &fg_only.task;
            let session = &fg_only.session;
            let em_tag = task.em_quadrant().tag();
            let start_time = session.start_time.format("%H:%M");
            let now = chrono::Utc::now();
            let duration = now - session.start_time;
            let duration_str = format_duration(duration);

            println!("Foreground: {} {}", task.name, em_tag);
            println!("Started: {} ({} ago)", start_time, duration_str);
        }
        (None, Some(bg_only)) => {
            let task = &bg_only.task;
            let session = &bg_only.session;
            let em_tag = task.em_quadrant().tag();
            let start_time = session.start_time.format("%H:%M");
            let now = chrono::Utc::now();
            let duration = now - session.start_time;
            let duration_str = format_duration(duration);

            println!("Background: {} {}", task.name, em_tag);
            println!("Started: {} ({} ago)", start_time, duration_str);
        }
        (Some(fg_sess), Some(bg_sess)) => {
            let fg_task = &fg_sess.task;
            let fg_session = &fg_sess.session;
            let fg_em_tag = fg_task.em_quadrant().tag();
            let fg_start = fg_session.start_time.format("%H:%M");
            let now = chrono::Utc::now();
            let fg_duration = now - fg_session.start_time;
            let fg_duration_str = format_duration(fg_duration);

            println!("Foreground: {} {}", fg_task.name, fg_em_tag);
            println!("Started: {} ({} ago)", fg_start, fg_duration_str);

            let bg_task = &bg_sess.task;
            let bg_session = &bg_sess.session;
            let bg_em_tag = bg_task.em_quadrant().tag();
            let bg_start = bg_session.start_time.format("%H:%M");
            let bg_duration = now - bg_session.start_time;
            let bg_duration_str = format_duration(bg_duration);

            println!();
            println!("Background: {} {}", bg_task.name, bg_em_tag);
            println!("Started: {} ({} ago)", bg_start, bg_duration_str);

            let overlap_start = fg_session.start_time.max(bg_session.start_time);
            let overlap_secs = (now - overlap_start).num_seconds();

            if overlap_secs > 0 {
                let overlap_mins = overlap_secs / 60;
                let split_mins = overlap_mins / 2;
                println!();
                println!(
                    "Overlap: {}m (time split {}m each)",
                    overlap_mins, split_mins
                );
            }
        }
    }

    Ok(())
}

fn handle_stop(
    all: bool,
    task_name: Option<&str>,
    no_prompt: bool,
    db: &Arc<Database>,
) -> Result<(), String> {
    let session_service = SessionService::new(Arc::clone(db));

    if all {
        let fg_swt = session_service
            .get_active_foreground()
            .map_err(|e| e.to_string())?;
        let bg_swt = session_service
            .get_active_background()
            .map_err(|e| e.to_string())?;

        let (fg, bg_sess) = session_service.stop_all().map_err(|e| e.to_string())?;

        if fg.is_none() && bg_sess.is_none() {
            println!("No active timers");
        } else {
            if let Some(s) = fg {
                let duration = s.end_time.unwrap() - s.start_time;
                if let Some(swt) = &fg_swt {
                    println!("Stopped: {} ({})", swt.task.name, format_duration(duration));
                } else {
                    println!("Stopped: {}", format_duration(duration));
                }
            }
            if let Some(s) = bg_sess {
                let duration = s.end_time.unwrap() - s.start_time;
                if let Some(swt) = &bg_swt {
                    println!(
                        "Stopped background: {} ({})",
                        swt.task.name,
                        format_duration(duration)
                    );
                } else {
                    println!("Stopped background: {}", format_duration(duration));
                }
            }
        }
        return Ok(());
    }

    if let Some(name) = task_name {
        let fg = session_service
            .get_active_foreground()
            .map_err(|e| e.to_string())?;
        let bg_sess = session_service
            .get_active_background()
            .map_err(|e| e.to_string())?;

        if let Some(ref fg_swt) = fg {
            if task_name_matches(name, &fg_swt.task.name) {
                let stopped = session_service.stop().map_err(|e| e.to_string())?;
                if let Some(session) = stopped {
                    let duration = session.end_time.unwrap() - session.start_time;
                    println!(
                        "Stopped: {} ({})",
                        fg_swt.task.name,
                        format_duration(duration)
                    );
                }
                return Ok(());
            }
        }

        if let Some(ref bg_swt) = bg_sess {
            if task_name_matches(name, &bg_swt.task.name) {
                let stopped = session_service
                    .stop_background()
                    .map_err(|e| e.to_string())?;
                if let Some(session) = stopped {
                    let duration = session.end_time.unwrap() - session.start_time;
                    println!(
                        "Stopped background: {} ({})",
                        bg_swt.task.name,
                        format_duration(duration)
                    );
                }
                return Ok(());
            }
        }

        return Err(format!("No active task matching '{}'", name));
    }

    let fg = session_service
        .get_active_foreground()
        .map_err(|e| e.to_string())?;
    let bg_sess = session_service
        .get_active_background()
        .map_err(|e| e.to_string())?;

    match (fg, bg_sess) {
        (None, None) => {
            println!("No active timer");
        }
        (Some(fg_swt), None) => {
            let task_name = fg_swt.task.name.clone();
            let stopped = session_service.stop().map_err(|e| e.to_string())?;
            if let Some(session) = stopped {
                let duration = session.end_time.unwrap() - session.start_time;
                println!("Stopped: {} ({})", task_name, format_duration(duration));
            }
        }
        (None, Some(bg_swt)) => {
            let task_name = bg_swt.task.name.clone();
            let stopped = session_service
                .stop_background()
                .map_err(|e| e.to_string())?;
            if let Some(session) = stopped {
                let duration = session.end_time.unwrap() - session.start_time;
                println!(
                    "Stopped background: {} ({})",
                    task_name,
                    format_duration(duration)
                );
            }
        }
        (Some(fg_sess), Some(bg_s)) => {
            if no_prompt {
                return Err(
                    "Multiple tasks running. Use --task <name> to specify which task to stop, or --all to stop all.".into()
                );
            }

            let fg_task_name = fg_sess.task.name.clone();
            let bg_task_name = bg_s.task.name.clone();

            println!("Multiple tasks running:");
            println!("  [F] {} (foreground)", fg_task_name);
            println!("  [B] {} (background)", bg_task_name);
            println!("  [A] Stop all");
            println!("  [C] Cancel");

            use std::io::{self, BufRead, Write};
            io::stdout().flush().unwrap();
            let stdin = io::stdin();
            let input = stdin.lock().lines().next();

            match input {
                Some(Ok(ref s)) if s.to_lowercase() == "a" => {
                    let (fg, bg_stopped) = session_service.stop_all().map_err(|e| e.to_string())?;
                    if let Some(s) = fg {
                        let duration = s.end_time.unwrap() - s.start_time;
                        println!("Stopped: {} ({})", fg_task_name, format_duration(duration));
                    }
                    if let Some(s) = bg_stopped {
                        let duration = s.end_time.unwrap() - s.start_time;
                        println!(
                            "Stopped background: {} ({})",
                            bg_task_name,
                            format_duration(duration)
                        );
                    }
                }
                Some(Ok(ref s)) if s.to_lowercase() == "b" => {
                    let stopped = session_service
                        .stop_background()
                        .map_err(|e| e.to_string())?;
                    if let Some(session) = stopped {
                        let duration = session.end_time.unwrap() - session.start_time;
                        println!(
                            "Stopped background: {} ({})",
                            bg_task_name,
                            format_duration(duration)
                        );
                    }
                }
                Some(Ok(ref s)) if s.to_lowercase() == "c" => {
                    println!("Cancelled");
                }
                _ => {
                    let stopped = session_service.stop().map_err(|e| e.to_string())?;
                    if let Some(session) = stopped {
                        let duration = session.end_time.unwrap() - session.start_time;
                        println!("Stopped: {} ({})", fg_task_name, format_duration(duration));
                    }
                }
            }
        }
    }

    Ok(())
}

fn task_name_matches(query: &str, task_name: &str) -> bool {
    let query_lower = query.to_lowercase();
    let task_lower = task_name.to_lowercase();
    task_lower.contains(&query_lower) || query_lower.contains(&task_lower)
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

#[allow(clippy::too_many_arguments)]
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
                    updated_task.status = status_str
                        .parse::<wdttg::TaskStatus>()
                        .map_err(|e| e.to_string())?;
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
    em_arg: Option<&str>,
    link_arg: Option<&str>,
    no_prompt: bool,
    db: &Arc<Database>,
) -> Result<(), String> {
    let task_service = TaskService::new(Arc::clone(db));
    let session_service = SessionService::new(Arc::clone(db));

    enum TaskSelection {
        Existing(Task),
        NewTask { name: String },
    }

    let selection = if let Some(task_name) = task {
        if !task_name.is_empty() {
            match task_service
                .get_by_name(task_name)
                .map_err(|e| e.to_string())?
            {
                Some(existing) => TaskSelection::Existing(existing),
                None => TaskSelection::NewTask {
                    name: task_name.to_string(),
                },
            }
        } else if is_interactive() && !no_prompt {
            let tasks = task_service
                .get_pickable_tasks()
                .map_err(|e| e.to_string())?;
            let picker = TaskPicker::new(tasks);
            match picker.run().map_err(|e| e.to_string())? {
                PickerResult::Selected(t) => TaskSelection::Existing(t),
                PickerResult::CreateNew(name) => TaskSelection::NewTask { name },
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
            PickerResult::Selected(t) => TaskSelection::Existing(t),
            PickerResult::CreateNew(name) => TaskSelection::NewTask { name },
            PickerResult::Cancelled => {
                println!("Cancelled.");
                return Ok(());
            }
        }
    } else {
        return Err("Task name required in non-interactive mode".to_string());
    };

    let needs_em = matches!(selection, TaskSelection::NewTask { .. });

    let quadrant = if needs_em {
        Some(if let Some(em_str) = em_arg {
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
        })
    } else {
        None
    };

    let now = chrono::Utc::now();

    let start = if let Some(start_str) = start_arg {
        parse_relative_time(start_str).map_err(|e| e.to_string())?
    } else if is_interactive() && !no_prompt {
        print!("Start time (e.g., '2h ago', '14:30', '3/22 11:00'): ");
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

    let overlaps = session_service
        .get_overlapping_sessions(start, end)
        .map_err(|e| e.to_string())?;

    if !overlaps.is_empty() && is_interactive() && !no_prompt {
        println!("Warning: This session overlaps with:");
        for overlap in &overlaps {
            let end_display = overlap
                .session
                .end_time
                .map(|e| e.format("%H:%M").to_string())
                .unwrap_or_else(|| "now".to_string());
            let bg_indicator = if overlap.session.is_background {
                " [bg]"
            } else {
                ""
            };
            println!(
                "  - {}{} ({} to {})",
                overlap.task.name,
                bg_indicator,
                overlap.session.start_time.format("%H:%M"),
                end_display
            );
        }
        println!("Overlapping time will be split 50/50.");
        print!("Continue? [Y/n] ");
        use std::io::{self, Write};
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        if input.trim().to_lowercase() == "n" {
            println!("Cancelled.");
            return Ok(());
        }
    }

    let task = match selection {
        TaskSelection::Existing(t) => t,
        TaskSelection::NewTask { name } => {
            let created = task_service
                .get_or_create(&name)
                .map_err(|e| e.to_string())?;
            if let Some(q) = quadrant {
                task_service
                    .set_em_quadrant(created.id, q.is_urgent(), q.is_important())
                    .map_err(|e| e.to_string())?;
            }
            created
        }
    };

    let session = session_service
        .log_session(task.id, start, end, link_arg)
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

fn quadrant_ansi_color(quadrant: &EmQuadrant) -> &'static str {
    match quadrant {
        EmQuadrant::DoFirst => "\x1b[31m",   // Red
        EmQuadrant::Schedule => "\x1b[32m",  // Green
        EmQuadrant::Delegate => "\x1b[33m",  // Yellow
        EmQuadrant::Eliminate => "\x1b[90m", // Dark Gray
    }
}

const ANSI_RESET: &str = "\x1b[0m";

fn handle_report_quadrant(period: &str, db: &Arc<Database>) -> Result<(), String> {
    let session_service = SessionService::new(Arc::clone(db));
    let (start, end) = parse_period(period).map_err(|e| e.to_string())?;

    let quadrant_times = session_service
        .get_quadrant_time(start, end)
        .map_err(|e| e.to_string())?;

    if quadrant_times.is_empty() {
        println!("No sessions found for period: {}", period);
        return Ok(());
    }

    let total_seconds: i64 = quadrant_times.iter().map(|qt| qt.total_seconds).sum();

    println!("Time by EM Quadrant ({} to {})", start, end);
    println!("{}", "=".repeat(50));

    for qt in &quadrant_times {
        let percentage = if total_seconds > 0 {
            (qt.total_seconds as f64 / total_seconds as f64) * 100.0
        } else {
            0.0
        };
        let color = quadrant_ansi_color(&qt.quadrant);
        println!(
            "{}{}{}: {} ({:.1}%) - {}",
            color,
            qt.quadrant.tag(),
            ANSI_RESET,
            format_duration(chrono::Duration::seconds(qt.total_seconds)),
            percentage,
            qt.quadrant.description()
        );
    }

    println!("{}", "-".repeat(50));
    println!(
        "Total: {}",
        format_duration(chrono::Duration::seconds(total_seconds))
    );

    Ok(())
}

fn handle_report_task(period: &str, db: &Arc<Database>) -> Result<(), String> {
    let session_service = SessionService::new(Arc::clone(db));
    let (start, end) = parse_period(period).map_err(|e| e.to_string())?;

    let task_times = session_service
        .get_task_time(start, end)
        .map_err(|e| e.to_string())?;

    if task_times.is_empty() {
        println!("No sessions found for period: {}", period);
        return Ok(());
    }

    let total_seconds: i64 = task_times.iter().map(|tt| tt.total_seconds).sum();

    println!("Time by Task ({} to {})", start, end);
    println!("{}", "=".repeat(70));

    for tt in &task_times {
        let percentage = if total_seconds > 0 {
            (tt.total_seconds as f64 / total_seconds as f64) * 100.0
        } else {
            0.0
        };
        let color = quadrant_ansi_color(&tt.em_quadrant);
        let session_str = format!(
            "{:>3} {:<8}",
            tt.session_count,
            if tt.session_count == 1 {
                "session"
            } else {
                "sessions"
            }
        );
        let lines = wrap(&tt.task_name, 30);
        for (i, line) in lines.iter().enumerate() {
            if i == 0 {
                println!(
                    "{}{:<30}{} {:>10} ({:>5.1}%) {}  {}",
                    color,
                    line,
                    ANSI_RESET,
                    format_duration(chrono::Duration::seconds(tt.total_seconds)),
                    percentage,
                    session_str,
                    tt.em_quadrant.tag()
                );
            } else {
                println!("{}  {}{}", color, line, ANSI_RESET);
            }
        }
    }

    println!("{}", "-".repeat(70));
    println!(
        "Total: {} across {} task{}",
        format_duration(chrono::Duration::seconds(total_seconds)),
        task_times.len(),
        if task_times.len() == 1 { "" } else { "s" }
    );

    Ok(())
}

fn handle_report_tag(period: &str, db: &Arc<Database>) -> Result<(), String> {
    let session_service = SessionService::new(Arc::clone(db));
    let (start, end) = parse_period(period).map_err(|e| e.to_string())?;

    let tag_times = session_service
        .get_tag_time(start, end)
        .map_err(|e| e.to_string())?;

    if tag_times.is_empty() {
        println!("No tagged sessions found for period: {}", period);
        return Ok(());
    }

    let total_seconds: i64 = tag_times.iter().map(|tt| tt.total_seconds).sum();

    println!("Time by Tag ({} to {})", start, end);
    println!("{}", "=".repeat(50));

    for tt in &tag_times {
        let percentage = if total_seconds > 0 {
            (tt.total_seconds as f64 / total_seconds as f64) * 100.0
        } else {
            0.0
        };
        let lines = wrap(&tt.tag, 29);
        for (i, line) in lines.iter().enumerate() {
            if i == 0 {
                println!(
                    "#{:<29} {:>10} ({:>5.1}%)",
                    line,
                    format_duration(chrono::Duration::seconds(tt.total_seconds)),
                    percentage
                );
            } else {
                println!("  {}", line);
            }
        }
    }

    println!("{}", "-".repeat(50));
    println!(
        "Total: {} across {} tag{}",
        format_duration(chrono::Duration::seconds(total_seconds)),
        tag_times.len(),
        if tag_times.len() == 1 { "" } else { "s" }
    );

    Ok(())
}

fn handle_report_timeline(date: &str, no_prompt: bool, db: &Arc<Database>) -> Result<(), String> {
    let session_service = SessionService::new(Arc::clone(db));
    let date = parse_date(date).map_err(|e| e.to_string())?;

    let blocks = session_service
        .get_day_with_gaps(date)
        .map_err(|e| e.to_string())?;

    if blocks.is_empty() {
        println!("No sessions found for: {}", date);
        return Ok(());
    }

    if is_interactive() && !no_prompt {
        use wdttg::ui::TimelineResult;

        let mut view = TimelineView::new(blocks, date);
        loop {
            match view.run().map_err(|e| e.to_string())? {
                TimelineResult::Cancelled | TimelineResult::Closed => break,
                TimelineResult::EditSession(session_id) => {
                    handle_edit_session(session_id, db)?;
                    let blocks = session_service
                        .get_day_with_gaps(date)
                        .map_err(|e| e.to_string())?;
                    view = TimelineView::new(blocks, date);
                }
            }
        }
    } else {
        print_timeline_static(&blocks, date);
    }

    Ok(())
}

fn handle_report_history(
    tag: Option<&str>,
    period: &str,
    limit: usize,
    no_prompt: bool,
    db: &Arc<Database>,
) -> Result<(), String> {
    let session_service = SessionService::new(Arc::clone(db));
    let (start, end) = parse_period(period).map_err(|e| e.to_string())?;

    if let Some(tag_name) = tag {
        let sessions = session_service
            .get_sessions_by_tag(tag_name, start, end)
            .map_err(|e| e.to_string())?;

        if is_interactive() && !no_prompt {
            use wdttg::ui::{HistoryView, HistoryViewResult};

            loop {
                let (s, e) = parse_period(period).map_err(|err| err.to_string())?;
                let sessions = session_service
                    .get_sessions_by_tag(tag_name, s, e)
                    .map_err(|err| err.to_string())?;
                let view = HistoryView::new(sessions, tag_name.to_string());

                match view.run().map_err(|e| e.to_string())? {
                    HistoryViewResult::Done | HistoryViewResult::Back => break,
                    HistoryViewResult::PeriodChanged(_) => {}
                    HistoryViewResult::EditSession(session_id) => {
                        handle_edit_session(session_id, db)?;
                    }
                }
            }
        } else {
            use wdttg::ui::{print_history_static, HistoryViewPeriod};
            let ui_period = match period {
                "day" => HistoryViewPeriod::Day,
                "month" => HistoryViewPeriod::Month,
                _ => HistoryViewPeriod::Week,
            };
            print_history_static(&sessions, tag_name, ui_period);
        }
    } else if is_interactive() && !no_prompt {
        use wdttg::ui::{HistoryView, HistoryViewPeriod, HistoryViewResult};
        use wdttg::ui::{TagPicker, TagPickerPeriod, TagPickerResult};

        let mut current_period = match period {
            "day" => TagPickerPeriod::Day,
            "month" => TagPickerPeriod::Month,
            _ => TagPickerPeriod::Week,
        };

        loop {
            let (start, end) = current_period.date_range();
            let tags = session_service
                .get_tags_with_time(start, end)
                .map_err(|e| e.to_string())?;

            let picker = TagPicker::new(tags).with_period(current_period);
            match picker.run().map_err(|e| e.to_string())? {
                TagPickerResult::Selected(selected_tag) => {
                    let mut history_period = match current_period {
                        TagPickerPeriod::Day => HistoryViewPeriod::Day,
                        TagPickerPeriod::Week => HistoryViewPeriod::Week,
                        TagPickerPeriod::Month => HistoryViewPeriod::Month,
                    };

                    loop {
                        let (h_start, h_end) = history_period.date_range();
                        let sessions = session_service
                            .get_sessions_by_tag(&selected_tag, h_start, h_end)
                            .map_err(|e| e.to_string())?;

                        let view = HistoryView::new(sessions, selected_tag.clone())
                            .with_period(history_period);
                        match view.run().map_err(|e| e.to_string())? {
                            HistoryViewResult::Done => {
                                return Ok(());
                            }
                            HistoryViewResult::Back => {
                                break;
                            }
                            HistoryViewResult::PeriodChanged(new_period) => {
                                history_period = new_period;
                            }
                            HistoryViewResult::EditSession(session_id) => {
                                handle_edit_session(session_id, db)?;
                            }
                        }
                    }
                }
                TagPickerResult::PeriodChanged(new_period) => {
                    current_period = new_period;
                }
                TagPickerResult::Cancelled => {
                    return Ok(());
                }
            }
        }
    } else {
        let sessions = session_service
            .get_recent(limit as i64)
            .map_err(|e| e.to_string())?;

        if sessions.is_empty() {
            println!("No sessions found");
            return Ok(());
        }

        println!("Recent Sessions (last {})", limit);
        println!("{}", "=".repeat(60));

        for swt in &sessions {
            let date = swt.session.start_time.format("%Y-%m-%d");
            let start_time = swt.session.start_time.format("%H:%M");
            let end_time = swt
                .session
                .end_time
                .map(|t| t.format("%H:%M").to_string())
                .unwrap_or_else(|| "active".to_string());
            let em_tag = swt.task.em_quadrant().tag();

            if let Some(seconds) = swt.duration_seconds() {
                println!(
                    "{} {}-{} [{}] {} {}",
                    date,
                    start_time,
                    end_time,
                    format_duration(chrono::Duration::seconds(seconds)),
                    swt.task.name,
                    em_tag
                );
            } else {
                println!(
                    "{} {}-{} [active] {} {}",
                    date, start_time, end_time, swt.task.name, em_tag
                );
            }
        }
    }

    Ok(())
}

fn handle_backup(keep: usize, db: &Arc<Database>) -> Result<(), String> {
    let (backup_path, kept, removed) = db.backup(keep).map_err(|e| e.to_string())?;

    let backup_path_str = backup_path
        .to_str()
        .unwrap_or("unknown path")
        .replace("/Users/", "~/")
        .replace(&std::env::var("HOME").unwrap_or_default(), "~");

    println!("Backup created: {}", backup_path_str);
    println!("Kept {} backups, removed {} old backups.", kept, removed);

    Ok(())
}

fn handle_completions(shell: &str) -> Result<(), String> {
    let shell = shell.parse::<Shell>().map_err(|_| {
        format!(
            "Unknown shell: {}. Options: bash, zsh, fish, elvish, powershell",
            shell
        )
    })?;

    let mut cmd = cli::Cli::command();
    let name = cmd.get_name().to_string();
    generate(shell, &mut cmd, name, &mut std::io::stdout());

    Ok(())
}

fn main() {
    if let Err(e) = wdttg::config::init() {
        eprintln!("Error loading config: {}", e);
        std::process::exit(1);
    }

    let cli = cli::parse();

    let db = match Database::open() {
        Ok(d) => Arc::new(d),
        Err(e) => {
            eprintln!("Error opening database: {}", e);
            std::process::exit(1);
        }
    };

    let result = match cli.command {
        Commands::Start {
            query,
            em,
            link,
            bg,
        } => handle_start(
            query.as_deref(),
            em.as_deref(),
            link.as_deref(),
            bg,
            cli.no_prompt,
            &db,
        ),
        Commands::Stop { all, task } => handle_stop(all, task.as_deref(), cli.no_prompt, &db),
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
        Commands::Log {
            task,
            start,
            end,
            em,
            link,
        } => handle_log(
            task.as_deref(),
            start.as_deref(),
            end.as_deref(),
            em.as_deref(),
            link.as_deref(),
            cli.no_prompt,
            &db,
        ),
        Commands::Report { report_type } => match report_type {
            wdttg::ReportCommands::Quadrant { period } => handle_report_quadrant(&period, &db),
            wdttg::ReportCommands::Task { period } => handle_report_task(&period, &db),
            wdttg::ReportCommands::Tag { period } => handle_report_tag(&period, &db),
            wdttg::ReportCommands::Timeline { date } => {
                handle_report_timeline(&date, cli.no_prompt, &db)
            }
            wdttg::ReportCommands::History { tag, period, limit } => {
                handle_report_history(tag.as_deref(), &period, limit, cli.no_prompt, &db)
            }
        },
        Commands::Backup { keep } => handle_backup(keep, &db),
        Commands::Completions { shell } => handle_completions(&shell),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
