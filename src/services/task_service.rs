use crate::db::Database;
use crate::models::{Task, TaskStatus};
use chrono::{DateTime, Utc};
use rusqlite::params;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TaskServiceError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Task not found: {0}")]
    NotFound(i64),
}

pub type Result<T> = std::result::Result<T, TaskServiceError>;

pub struct TaskService {
    db: Arc<Database>,
}

impl TaskService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub fn get_or_create(&self, name: &str) -> Result<Task> {
        if let Some(task) = self.get_by_name(name)? {
            return Ok(task);
        }

        let tags = Self::extract_tags(name);
        let conn = self.db.connection();

        conn.execute(
            "INSERT INTO tasks (name, em_urgent, em_important) VALUES (?1, 0, 0)",
            params![name],
        )?;

        let id = conn.last_insert_rowid();
        self.set_tags_internal(&conn, id, &tags)?;

        drop(conn);

        self.get_by_id(id)?.ok_or(TaskServiceError::NotFound(id))
    }

    pub fn get_by_id(&self, id: i64) -> Result<Option<Task>> {
        let conn = self.db.connection();

        let result = conn.query_row(
            "SELECT id, name, notes, em_urgent, em_important, status, recurring, created_at, updated_at
             FROM tasks WHERE id = ?1",
            params![id],
            |row| self.row_to_task(&conn, row),
        );

        match result {
            Ok(task) => Ok(Some(task)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_by_name(&self, name: &str) -> Result<Option<Task>> {
        let conn = self.db.connection();

        let result = conn.query_row(
            "SELECT id, name, notes, em_urgent, em_important, status, recurring, created_at, updated_at
             FROM tasks WHERE name = ?1",
            params![name],
            |row| self.row_to_task(&conn, row),
        );

        match result {
            Ok(task) => Ok(Some(task)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_pickable_tasks(&self) -> Result<Vec<Task>> {
        let conn = self.db.connection();

        let mut stmt = conn.prepare(
            "SELECT id, name, notes, em_urgent, em_important, status, recurring, created_at, updated_at
             FROM tasks 
             WHERE status = 'active' OR recurring = 1
             ORDER BY updated_at DESC",
        )?;

        let tasks = stmt
            .query_map([], |row| self.row_to_task(&conn, row))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(tasks)
    }

    pub fn search_completed(&self, query: &str) -> Result<Vec<Task>> {
        let conn = self.db.connection();

        let pattern = format!("%{}%", query);
        let mut stmt = conn.prepare(
            "SELECT id, name, notes, em_urgent, em_important, status, recurring, created_at, updated_at
             FROM tasks 
             WHERE status = 'completed' AND name LIKE ?1
             ORDER BY updated_at DESC",
        )?;

        let tasks = stmt
            .query_map(params![pattern], |row| self.row_to_task(&conn, row))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(tasks)
    }

    pub fn get_non_archived_tasks(&self) -> Result<Vec<Task>> {
        let conn = self.db.connection();

        let mut stmt = conn.prepare(
            "SELECT id, name, notes, em_urgent, em_important, status, recurring, created_at, updated_at
             FROM tasks 
             WHERE status != 'archived'
             ORDER BY updated_at DESC",
        )?;

        let tasks = stmt
            .query_map([], |row| self.row_to_task(&conn, row))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(tasks)
    }

    pub fn update(&self, task: &Task) -> Result<()> {
        let conn = self.db.connection();

        conn.execute(
            "UPDATE tasks SET 
             name = ?1, notes = ?2, em_urgent = ?3, em_important = ?4, 
             status = ?5, recurring = ?6
             WHERE id = ?7",
            params![
                task.name,
                task.notes,
                task.em_urgent as i32,
                task.em_important as i32,
                task.status.as_str(),
                task.recurring as i32,
                task.id,
            ],
        )?;

        self.set_tags_internal(&conn, task.id, &task.tags)?;

        Ok(())
    }

    pub fn set_em_quadrant(&self, id: i64, urgent: bool, important: bool) -> Result<()> {
        let conn = self.db.connection();

        let rows_affected = conn.execute(
            "UPDATE tasks SET em_urgent = ?1, em_important = ?2 WHERE id = ?3",
            params![urgent as i32, important as i32, id],
        )?;

        if rows_affected == 0 {
            return Err(TaskServiceError::NotFound(id));
        }

        Ok(())
    }

    pub fn set_status(&self, id: i64, status: TaskStatus) -> Result<()> {
        let conn = self.db.connection();

        let rows_affected = conn.execute(
            "UPDATE tasks SET status = ?1 WHERE id = ?2",
            params![status.as_str(), id],
        )?;

        if rows_affected == 0 {
            return Err(TaskServiceError::NotFound(id));
        }

        Ok(())
    }

    pub fn extract_tags(name: &str) -> Vec<String> {
        name.split('/')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    pub fn set_tags(&self, task_id: i64, tags: &[String]) -> Result<()> {
        let conn = self.db.connection();
        self.set_tags_internal(&conn, task_id, tags)?;
        Ok(())
    }

    fn set_tags_internal(
        &self,
        conn: &rusqlite::Connection,
        task_id: i64,
        tags: &[String],
    ) -> Result<()> {
        conn.execute("DELETE FROM task_tags WHERE task_id = ?1", params![task_id])?;

        for tag in tags {
            conn.execute(
                "INSERT OR IGNORE INTO tags (name) VALUES (?1)",
                params![tag],
            )?;

            conn.execute(
                "INSERT INTO task_tags (task_id, tag_id)
                 SELECT ?1, id FROM tags WHERE name = ?2",
                params![task_id, tag],
            )?;
        }

        Ok(())
    }

    fn row_to_task(
        &self,
        conn: &rusqlite::Connection,
        row: &rusqlite::Row,
    ) -> std::result::Result<Task, rusqlite::Error> {
        let id: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        let notes: Option<String> = row.get(2)?;
        let em_urgent: i32 = row.get(3)?;
        let em_important: i32 = row.get(4)?;
        let status_str: String = row.get(5)?;
        let recurring: i32 = row.get(6)?;
        let created_at_str: String = row.get(7)?;
        let updated_at_str: String = row.get(8)?;

        let status = TaskStatus::from_str(&status_str).map_err(|_| {
            rusqlite::Error::InvalidColumnType(5, "status".to_string(), rusqlite::types::Type::Text)
        })?;

        let created_at: DateTime<Utc> = Self::parse_sqlite_datetime(&created_at_str)?;
        let updated_at: DateTime<Utc> = Self::parse_sqlite_datetime(&updated_at_str)?;

        let tags = self.get_tags_for_task(conn, id)?;

        Ok(Task {
            id,
            name,
            notes,
            em_urgent: em_urgent != 0,
            em_important: em_important != 0,
            status,
            recurring: recurring != 0,
            tags,
            created_at,
            updated_at,
        })
    }

    fn get_tags_for_task(
        &self,
        conn: &rusqlite::Connection,
        task_id: i64,
    ) -> std::result::Result<Vec<String>, rusqlite::Error> {
        let mut stmt = conn.prepare(
            "SELECT t.name FROM tags t
             INNER JOIN task_tags tt ON t.id = tt.tag_id
             WHERE tt.task_id = ?1
             ORDER BY t.name",
        )?;

        let tags = stmt
            .query_map(params![task_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(tags)
    }

    fn parse_sqlite_datetime(s: &str) -> std::result::Result<DateTime<Utc>, rusqlite::Error> {
        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
            .map(|dt| dt.and_utc())
            .map_err(|_| {
                rusqlite::Error::InvalidColumnType(
                    0,
                    "datetime".to_string(),
                    rusqlite::types::Type::Text,
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::EmQuadrant;

    fn create_test_service() -> TaskService {
        let db = Database::open_memory().expect("Failed to create in-memory database");
        TaskService::new(Arc::new(db))
    }

    #[test]
    fn test_extract_tags() {
        assert_eq!(
            TaskService::extract_tags("meeting/standup"),
            vec!["meeting", "standup"]
        );
        assert_eq!(
            TaskService::extract_tags("coding/jira/feature"),
            vec!["coding", "jira", "feature"]
        );
        assert_eq!(TaskService::extract_tags("single"), vec!["single"]);
        assert_eq!(
            TaskService::extract_tags("  spaced  /  tags  "),
            vec!["spaced", "tags"]
        );
        assert_eq!(TaskService::extract_tags(""), Vec::<String>::new());
        assert_eq!(TaskService::extract_tags("/"), Vec::<String>::new());
    }

    #[test]
    fn test_get_or_create_creates_new_task() {
        let service = create_test_service();

        let task = service.get_or_create("meeting/standup").unwrap();

        assert_eq!(task.name, "meeting/standup");
        assert_eq!(task.tags, vec!["meeting", "standup"]);
        assert!(!task.em_urgent);
        assert!(!task.em_important);
        assert_eq!(task.status, TaskStatus::Active);
        assert!(!task.recurring);
    }

    #[test]
    fn test_get_or_create_returns_existing_task() {
        let service = create_test_service();

        let task1 = service.get_or_create("coding/jira-123").unwrap();
        let task2 = service.get_or_create("coding/jira-123").unwrap();

        assert_eq!(task1.id, task2.id);
        assert_eq!(task1.name, task2.name);
    }

    #[test]
    fn test_get_by_id() {
        let service = create_test_service();

        let created = service.get_or_create("test-task").unwrap();
        let found = service.get_by_id(created.id).unwrap();

        assert!(found.is_some());
        let task = found.unwrap();
        assert_eq!(task.id, created.id);
        assert_eq!(task.name, "test-task");
    }

    #[test]
    fn test_get_by_id_not_found() {
        let service = create_test_service();

        let result = service.get_by_id(99999).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_get_pickable_tasks() {
        let service = create_test_service();

        let active = service.get_or_create("active-task").unwrap();
        let completed = service.get_or_create("completed-task").unwrap();
        service
            .set_status(completed.id, TaskStatus::Completed)
            .unwrap();

        let recurring = service.get_or_create("recurring-task").unwrap();
        service
            .update(&Task {
                id: recurring.id,
                name: recurring.name.clone(),
                notes: None,
                em_urgent: false,
                em_important: true,
                status: TaskStatus::Completed,
                recurring: true,
                tags: recurring.tags.clone(),
                created_at: recurring.created_at,
                updated_at: recurring.updated_at,
            })
            .unwrap();

        let pickable = service.get_pickable_tasks().unwrap();

        assert!(pickable.iter().any(|t| t.id == active.id));
        assert!(!pickable.iter().any(|t| t.id == completed.id));
        assert!(pickable.iter().any(|t| t.id == recurring.id));
    }

    #[test]
    fn test_set_em_quadrant() {
        let service = create_test_service();

        let task = service.get_or_create("test-task").unwrap();
        service.set_em_quadrant(task.id, true, true).unwrap();

        let updated = service.get_by_id(task.id).unwrap().unwrap();
        assert!(updated.em_urgent);
        assert!(updated.em_important);
        assert_eq!(updated.em_quadrant(), EmQuadrant::DoFirst);
    }

    #[test]
    fn test_set_em_quadrant_not_found() {
        let service = create_test_service();

        let result = service.set_em_quadrant(99999, true, true);
        assert!(result.is_err());
    }

    #[test]
    fn test_set_status() {
        let service = create_test_service();

        let task = service.get_or_create("test-task").unwrap();
        service.set_status(task.id, TaskStatus::Completed).unwrap();

        let updated = service.get_by_id(task.id).unwrap().unwrap();
        assert_eq!(updated.status, TaskStatus::Completed);
    }

    #[test]
    fn test_search_completed() {
        let service = create_test_service();

        let task1 = service.get_or_create("coding/jira-123").unwrap();
        let task2 = service.get_or_create("coding/jira-456").unwrap();
        let task3 = service.get_or_create("meeting/standup").unwrap();

        service.set_status(task1.id, TaskStatus::Completed).unwrap();
        service.set_status(task2.id, TaskStatus::Completed).unwrap();
        service.set_status(task3.id, TaskStatus::Completed).unwrap();

        let results = service.search_completed("jira").unwrap();

        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|t| t.id == task1.id));
        assert!(results.iter().any(|t| t.id == task2.id));
        assert!(!results.iter().any(|t| t.id == task3.id));
    }

    #[test]
    fn test_set_tags() {
        let service = create_test_service();

        let task = service.get_or_create("test-task").unwrap();

        service
            .set_tags(task.id, &["new-tag".to_string(), "another-tag".to_string()])
            .unwrap();

        let updated = service.get_by_id(task.id).unwrap().unwrap();
        assert_eq!(updated.tags, vec!["another-tag", "new-tag"]);
    }

    #[test]
    fn test_update_task() {
        let service = create_test_service();

        let task = service.get_or_create("old-name").unwrap();

        let mut updated_task = task.clone();
        updated_task.name = "new-name".to_string();
        updated_task.notes = Some("Updated notes".to_string());
        updated_task.em_urgent = true;
        updated_task.em_important = true;
        updated_task.recurring = true;
        updated_task.tags = vec!["new-tag".to_string()];

        service.update(&updated_task).unwrap();

        let found = service.get_by_id(task.id).unwrap().unwrap();
        assert_eq!(found.name, "new-name");
        assert_eq!(found.notes, Some("Updated notes".to_string()));
        assert!(found.em_urgent);
        assert!(found.em_important);
        assert!(found.recurring);
        assert_eq!(found.tags, vec!["new-tag"]);
    }

    #[test]
    fn test_get_non_archived_tasks() {
        let service = create_test_service();

        let active = service.get_or_create("active-task").unwrap();
        let completed = service.get_or_create("completed-task").unwrap();
        service
            .set_status(completed.id, TaskStatus::Completed)
            .unwrap();
        let archived = service.get_or_create("archived-task").unwrap();
        service
            .set_status(archived.id, TaskStatus::Archived)
            .unwrap();

        let non_archived = service.get_non_archived_tasks().unwrap();

        assert!(non_archived.iter().any(|t| t.id == active.id));
        assert!(non_archived.iter().any(|t| t.id == completed.id));
        assert!(!non_archived.iter().any(|t| t.id == archived.id));
    }
}
