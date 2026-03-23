use tempfile::TempDir;
use wdttg::Database;

#[test]
fn test_database_file_created() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("wdttg").join("test.db");

    let db = Database::open_path(&db_path).expect("Failed to open database");

    assert!(db_path.exists(), "Database file should be created");
    assert!(db.path().is_some(), "Path should be Some for file database");
    assert_eq!(db.path().unwrap(), &db_path);
}

#[test]
fn test_database_creates_parent_dirs() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("nested").join("dirs").join("test.db");

    let _db = Database::open_path(&db_path).expect("Failed to open database");

    assert!(
        db_path.exists(),
        "Database file should be created with parent dirs"
    );
}

#[test]
fn test_memory_database_no_path() {
    let db = Database::open_memory().expect("Failed to create in-memory database");
    assert!(
        db.path().is_none(),
        "Path should be None for in-memory database"
    );
}

#[test]
fn test_tables_created_after_migration() {
    let db = Database::open_memory().expect("Failed to create in-memory database");
    let conn = db.connection();

    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    assert!(tables.contains(&"tasks".to_string()));
    assert!(tables.contains(&"sessions".to_string()));
    assert!(tables.contains(&"tags".to_string()));
    assert!(tables.contains(&"task_tags".to_string()));
}

#[test]
fn test_backup_path_for_file_database() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("wdttg").join("test.db");

    let db = Database::open_path(&db_path).expect("Failed to open database");
    let backup_path = db.backup_path().expect("Failed to get backup path");

    assert!(backup_path.ends_with("backups"));
}

#[test]
fn test_backup_path_for_memory_database() {
    let db = Database::open_memory().expect("Failed to create in-memory database");
    let result = db.backup_path();

    assert!(
        result.is_err(),
        "In-memory database should not support backup path"
    );
}

#[test]
fn test_wdttg_db_env_override() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let custom_path = temp_dir.path().join("custom").join("custom.db");

    std::env::set_var("WDTTG_DB", &custom_path);

    let db = Database::open().expect("Failed to open database");

    assert_eq!(db.path().unwrap(), &custom_path);
    assert!(
        custom_path.exists(),
        "Custom path database should be created"
    );

    std::env::remove_var("WDTTG_DB");
}

#[test]
fn test_reopen_existing_database() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("wdttg").join("test.db");

    {
        let db = Database::open_path(&db_path).expect("Failed to open database");
        let conn = db.connection();
        conn.execute(
            "INSERT INTO tasks (name, em_urgent, em_important) VALUES ('test', 1, 1)",
            [],
        )
        .expect("Failed to insert task");
    }

    {
        let db = Database::open_path(&db_path).expect("Failed to reopen database");
        let conn = db.connection();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "Data should persist after reopening");
    }
}

mod cli_tests {
    use assert_cmd::Command;
    use predicates::prelude::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static DB_LOCK: Mutex<()> = Mutex::new(());

    fn with_temp_db<F>(test: F)
    where
        F: FnOnce(&TempDir, &std::path::Path),
    {
        let _lock = DB_LOCK.lock().unwrap();
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db_path = temp_dir.path().join("test.db");
        std::env::set_var("WDTTG_DB", &db_path);
        test(&temp_dir, &db_path);
        std::env::remove_var("WDTTG_DB");
    }

    #[test]
    fn test_help_shows_all_commands() {
        let mut cmd = Command::cargo_bin("wdttg").unwrap();
        cmd.arg("--help")
            .assert()
            .success()
            .stdout(predicate::str::contains("start"))
            .stdout(predicate::str::contains("stop"))
            .stdout(predicate::str::contains("status"))
            .stdout(predicate::str::contains("done"))
            .stdout(predicate::str::contains("reopen"))
            .stdout(predicate::str::contains("archive"))
            .stdout(predicate::str::contains("edit"))
            .stdout(predicate::str::contains("log"))
            .stdout(predicate::str::contains("report"))
            .stdout(predicate::str::contains("backup"));
    }

    #[test]
    fn test_start_help_shows_flags() {
        let mut cmd = Command::cargo_bin("wdttg").unwrap();
        cmd.args(["start", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--em"))
            .stdout(predicate::str::contains("--no-prompt"))
            .stdout(predicate::str::contains("QUERY"));
    }

    #[test]
    fn test_no_prompt_flag_parsed() {
        with_temp_db(|_temp_dir, _db_path| {
            let mut cmd = Command::cargo_bin("wdttg").unwrap();
            cmd.args(["--no-prompt", "start", "test", "--em", "11"])
                .assert()
                .success()
                .stdout(predicate::str::contains("Started: test"))
                .stdout(predicate::str::contains("#em11"));
        });
    }

    #[test]
    fn test_invalid_command_shows_error() {
        let mut cmd = Command::cargo_bin("wdttg").unwrap();
        cmd.arg("invalid")
            .assert()
            .failure()
            .stderr(predicate::str::contains("error"));
    }

    #[test]
    fn test_report_subcommands() {
        let mut cmd = Command::cargo_bin("wdttg").unwrap();
        cmd.args(["report", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("quadrant"))
            .stdout(predicate::str::contains("task"))
            .stdout(predicate::str::contains("tag"))
            .stdout(predicate::str::contains("timeline"))
            .stdout(predicate::str::contains("history"));
    }

    #[test]
    fn test_start_creates_task_and_session() {
        with_temp_db(|_temp_dir, _db_path| {
            let mut cmd = Command::cargo_bin("wdttg").unwrap();
            cmd.args(["--no-prompt", "start", "new-task", "--em", "01"])
                .assert()
                .success()
                .stdout(predicate::str::contains("Started: new-task"))
                .stdout(predicate::str::contains("#em01"))
                .stdout(predicate::str::contains("Session ID:"));
        });
    }

    #[test]
    fn test_start_with_existing_task_reuses_it() {
        with_temp_db(|_temp_dir, _db_path| {
            let mut cmd1 = Command::cargo_bin("wdttg").unwrap();
            cmd1.args(["--no-prompt", "start", "existing-task", "--em", "11"])
                .assert()
                .success();

            let mut cmd2 = Command::cargo_bin("wdttg").unwrap();
            cmd2.args(["--no-prompt", "stop"]).assert().success();

            let mut cmd3 = Command::cargo_bin("wdttg").unwrap();
            cmd3.args(["--no-prompt", "start", "existing-task"])
                .assert()
                .success()
                .stdout(predicate::str::contains("Started: existing-task"))
                .stdout(predicate::str::contains("#em11"));
        });
    }

    #[test]
    fn test_start_stops_previous_session() {
        with_temp_db(|_temp_dir, _db_path| {
            let mut cmd1 = Command::cargo_bin("wdttg").unwrap();
            cmd1.args(["--no-prompt", "start", "task-one", "--em", "11"])
                .assert()
                .success();

            let mut cmd2 = Command::cargo_bin("wdttg").unwrap();
            cmd2.args(["--no-prompt", "start", "task-two", "--em", "01"])
                .assert()
                .success();

            let mut cmd3 = Command::cargo_bin("wdttg").unwrap();
            cmd3.args(["--no-prompt", "stop"])
                .assert()
                .success()
                .stdout(predicate::str::contains("Stopped: task-two"));
        });
    }

    #[test]
    fn test_stop_ends_session_and_shows_duration() {
        with_temp_db(|_temp_dir, _db_path| {
            let mut cmd1 = Command::cargo_bin("wdttg").unwrap();
            cmd1.args(["--no-prompt", "start", "test-task", "--em", "11"])
                .assert()
                .success();

            let mut cmd2 = Command::cargo_bin("wdttg").unwrap();
            cmd2.args(["--no-prompt", "stop"])
                .assert()
                .success()
                .stdout(predicate::str::contains("Stopped: test-task"))
                .stdout(predicate::str::contains("("))
                .stdout(predicate::str::contains("Session ID:"));
        });
    }

    #[test]
    fn test_stop_no_active_timer() {
        with_temp_db(|_temp_dir, _db_path| {
            let mut cmd = Command::cargo_bin("wdttg").unwrap();
            cmd.args(["--no-prompt", "stop"])
                .assert()
                .success()
                .stdout(predicate::str::contains("No active timer"));
        });
    }

    #[test]
    fn test_start_requires_em_for_new_task_in_non_interactive() {
        with_temp_db(|_temp_dir, _db_path| {
            let mut cmd = Command::cargo_bin("wdttg").unwrap();
            cmd.args(["--no-prompt", "start", "new-task-without-em"])
                .assert()
                .failure()
                .stderr(predicate::str::contains("EM quadrant required"));
        });
    }

    #[test]
    fn test_start_reuses_em_for_existing_task() {
        with_temp_db(|_temp_dir, _db_path| {
            let mut cmd1 = Command::cargo_bin("wdttg").unwrap();
            cmd1.args(["--no-prompt", "start", "task-with-em", "--em", "10"])
                .assert()
                .success();

            let mut cmd2 = Command::cargo_bin("wdttg").unwrap();
            cmd2.args(["--no-prompt", "stop"]).assert().success();

            let mut cmd3 = Command::cargo_bin("wdttg").unwrap();
            cmd3.args(["--no-prompt", "start", "task-with-em"])
                .assert()
                .success()
                .stdout(predicate::str::contains("#em10"));
        });
    }
}
