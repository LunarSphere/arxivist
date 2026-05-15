use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

//setup the database w/ a couple of SQL queries
pub async fn setup_db() -> anyhow::Result<SqlitePool> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1) //one writer for simplicity
        .connect("sqlite://crawler.db?mode=rwc")
        .await?;

    sqlx::query("PRAGMA journal_mode = WAL;")
        .execute(&pool)
        .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            url TEXT NOT NULL UNIQUE,
            status TEXT NOT NULL DEFAULT 'queued',
            http_status INTEGER,
            content_type TEXT,
            title TEXT,
            fetched_at TEXT,
            error TEXT
        );
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS links (
            from_url TEXT NOT NULL,
            to_url TEXT NOT NULL,
            UNIQUE(from_url, to_url)
        );
        "#,
    )
    .execute(&pool)
    .await?;

    Ok(pool)
}
// define messages types for database events
pub enum DbEvent {
    PageQueued {
        url: String,
    },
    PageFetched {
        url: String,
        http_status: u16,
        content_type: String,
    },
    PageFailed {
        url: String,
        error: String,
    },
    LinkFound {
        from_url: String,
        to_url: String,
    },
}

pub async fn db_writer(
    pool: SqlitePool,
    rx: async_channel::Receiver<DbEvent>,
) -> anyhow::Result<()> {
    while let Ok(event) = rx.recv().await {
        match event {
            DbEvent::PageQueued { url } => {
                sqlx::query(
                    r#"
                    INSERT OR IGNORE INTO pages (url, status)
                    VALUES (?, 'queued');
                    "#,
                )
                .bind(url)
                .execute(&pool)
                .await?;
            }
            DbEvent::PageFetched {
                url,
                http_status,
                content_type,
            } => {
                sqlx::query(
                    r#"
                    UPDATE pages
                    SET status = 'fetched',
                        http_status = ?,
                        content_type = ?,
                        fetched_at = datetime('now'),
                        error = NULL
                    WHERE url = ?
                    "#,
                )
                .bind(http_status)
                .bind(content_type)
                .bind(url)
                .execute(&pool)
                .await?;
            }
            DbEvent::PageFailed { url, error } => {
                sqlx::query(
                    r#"
                    UPDATE pages
                    SET status = "failed",
                        error = ?,
                        fetched_at = datetime('now')
                    WHERE url = ?
                    "#,
                )
                .bind(error)
                .bind(url)
                .execute(&pool)
                .await?;
            }
            DbEvent::LinkFound { from_url, to_url } => {
                sqlx::query(
                    r#"
                    INSERT OR IGNORE INTO links (from_url, to_url)
                    Values(?, ?)
                    "#,
                )
                .bind(from_url)
                .bind(to_url)
                .execute(&pool)
                .await?;
            }
        }
    }
    Ok(())
}
