use anyhow::Result;
use serde::Serialize;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

pub type DbPool = SqlitePool;

use sqlx::Row;
use sqlx::{Pool, Sqlite};

#[derive(Debug, serde::Serialize)]
pub struct MessageRow {
    pub role: String,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, serde::Serialize)]
pub struct SessionWithMessages {
    pub session_id: String,
    pub created_at: String,
    pub messages: Vec<MessageRow>,
}

pub async fn init_db() -> Result<DbPool> {
    let mut db_path = std::env::current_dir()?;
    db_path.push("chat.db");

    if let Some(parent) = db_path.parent() {
        if !parent.exists() {
            println!("[DB] Creating directory: {:?}", parent);
            std::fs::create_dir_all(parent)?;
        }
    }

    if !db_path.exists() {
        println!("[DB] Creating empty DB file at {:?}", db_path);
        std::fs::File::create(&db_path)?;
    }

    let db_url = format!("sqlite:{}", db_path.to_string_lossy());
    println!("[DB] Using SQLite URL: {}", db_url);

    if std::fs::OpenOptions::new()
        .write(true)
        .open(&db_path)
        .is_err()
    {
        return Err(anyhow::anyhow!(
            "SQLite file is not writable: {:?}",
            db_path
        ));
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS sessions (
            id          TEXT PRIMARY KEY,
            created_at  DATETIME DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS messages (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id  TEXT NOT NULL,
            role        TEXT NOT NULL,
            content     TEXT NOT NULL,
            created_at  DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (session_id) REFERENCES sessions(id)
        );
        "#,
    )
    .execute(&pool)
    .await?;

    println!("[DB] Database initialized successfully.");
    Ok(pool)
}

pub async fn save_chat_turn(
    pool: &DbPool,
    session_id: &str,
    user_prompt: &str,
    assistant_reply: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO sessions (id)
        VALUES (?1);
        "#,
    )
    .bind(session_id)
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO messages (session_id, role, content)
        VALUES (?1, 'user', ?2);
        "#,
    )
    .bind(session_id)
    .bind(user_prompt)
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO messages (session_id, role, content)
        VALUES (?1, 'assistant', ?2);
        "#,
    )
    .bind(session_id)
    .bind(assistant_reply)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn load_all_history(pool: &DbPool) -> Result<Vec<SessionWithMessages>> {
    let sessions = sqlx::query(
        r#"
        SELECT id, created_at
        FROM sessions
        ORDER BY created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut result = Vec::new();

    for session in sessions {
        let session_id: String = session.get("id");
        let created_at: String = session.get("created_at");

        let messages = sqlx::query(
            r#"
            SELECT role, content, created_at
            FROM messages
            WHERE session_id = ?
            ORDER BY created_at ASC
            "#,
        )
        .bind(&session_id)
        .fetch_all(pool)
        .await?;

        let msg_list = messages
            .into_iter()
            .map(|row| MessageRow {
                role: row.get("role"),
                content: row.get("content"),
                created_at: row.get("created_at"),
            })
            .collect::<Vec<_>>();

        result.push(SessionWithMessages {
            session_id,
            created_at,
            messages: msg_list,
        });
    }

    Ok(result)
}
