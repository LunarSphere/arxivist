use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

async fn get_or_create_page_id(pool: &SqlitePool, url: &str) -> anyhow::Result<i64> {
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO pages (url, status)
        VALUES (?, 'queued');
        "#,
    )
    .bind(url)
    .execute(pool)
    .await?;

    let page_id: i64 = sqlx::query_scalar(
        r#"
        SELECT id FROM pages
        WHERE url = ?;
        "#,
    )
    .bind(url)
    .fetch_one(pool)
    .await?;

    Ok(page_id)
}

//setup the database w/ a couple of SQL queries
pub async fn setup_db() -> anyhow::Result<SqlitePool> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1) //one writer for simplicity
        .connect("sqlite://crawler.db?mode=rwc")
        .await?;

    sqlx::query("PRAGMA journal_mode = WAL;")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA foreign_keys = ON;")
        .execute(&pool)
        .await?;

    // TODO: maybe make table align more with db schema
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
            CREATE TABLE IF NOT EXISTS page_contents (
                page_id INTEGER PRIMARY KEY,
                html TEXT,
                extracted_text TEXT,
                content_hash TEXT,
                stored_at TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY(page_id) REFERENCES pages(id)
            );
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS links (
            from_page_id INTEGER NOT NULL,
            to_page_id INTEGER NOT NULL,
            anchor_text TEXT
            discovered_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(from_page_id, to_page_id),
            FOREIGN KEY(from_page_id) REFERENCES pages(id),
            FOREIGN KEY(to_page_id) REFERENCES pages(id)
        );
        "#,
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS page_assets (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            page_id INTEGER NOT NULL,
            asset_url TEXT NOT NULL,
            asset_type TEXT NOT NULL,
            alt_text TEXT,
            discovered_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(page_id, asset_url),
            FOREIGN KEY(page_id) REFERENCES pages(id)
        );
        "#,
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS terms (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            term TEXT NOT NULL UNIQUE
        );
        "#,
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS postings (
            term_id INTEGER NOT NULL,
            page_id INTEGER NOT NULL,
            term_frequency INTEGER NOT NULL,
            positions TEXT,
            PRIMARY KEY (term_id, page_id),
            FOREIGN KEY(term_id) REFERENCES terms(id),
            FOREIGN KEY(page_id) REFERENCES pages(id)
        );
        "#,
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS document_stats (
            page_id INTEGER PRIMARY KEY,
            word_count INTEGER NOT NULL,
            unique_terms INTEGER NOT NULL,
            indexed_at TEXT NOT NULL DEFAULT (datetime('now')),
            FOREIGN KEY(page_id) REFERENCES pages(id)
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
        title: Option<String>,
    },
    PageFailed {
        url: String,
        error: String,
    },
    LinkFound {
        from_url: String,
        to_url: String,
        anchor_text: Option<String>,
    },
    PageContentFound {
        url: String,
        html: String,
        extracted_text: Option<String>,
        content_hash: Option<String>,
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
                title,
            } => {
                sqlx::query(
                    r#"
                    UPDATE pages
                    SET status = 'fetched',
                        http_status = ?,
                        content_type = ?,
                        title = ?,
                        fetched_at = datetime('now'),
                        error = NULL
                    WHERE url = ?
                    "#,
                )
                .bind(http_status)
                .bind(content_type)
                .bind(title)
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
            DbEvent::LinkFound {
                from_url,
                to_url,
                anchor_text,
            } => {
                let from_page_id = get_or_create_page_id(&pool, &from_url).await?;
                let to_page_id = get_or_create_page_id(&pool, &to_url).await?;
                sqlx::query(
                    r#"
                    INSERT OR IGNORE INTO links (from_page_id, to_page_id, anchor_text)
                    Values(?, ?, ?)
                    "#,
                )
                .bind(from_page_id)
                .bind(to_page_id)
                .bind(anchor_text)
                .execute(&pool)
                .await?;
            }
            DbEvent::PageContentFound {
                url,
                html,
                extracted_text,
                content_hash,
            } => {
                let page_id = get_or_create_page_id(&pool, &url).await?;
                sqlx::query(
                    r#"
                   INSERT INTO page_contents (
                       page_id,
                       html,
                       extracted_text,
                       content_hash
                   )
                   VALUES (?, ?, ?, ?)
                   ON CONFLICT(page_id) DO UPDATE SET
                       html = excluded.html,
                       extracted_text = excluded.extracted_text,
                       content_hash = excluded.content_hash,
                       stored_at = datetime('now');
                   "#,
                )
                .bind(page_id)
                .bind(html)
                .bind(extracted_text)
                .bind(content_hash)
                .execute(&pool)
                .await?;
            }
        }
    }
    Ok(())
}
