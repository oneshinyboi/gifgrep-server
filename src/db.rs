use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::str::FromStr;

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS favorites (
  id         TEXT PRIMARY KEY,
  url        TEXT NOT NULL,
  preview    TEXT,
  provider   TEXT,
  title      TEXT,
  added_at   INTEGER NOT NULL,
  last_used  INTEGER,
  use_count  INTEGER NOT NULL DEFAULT 0
);
"#;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Favorite {
    pub id: String,
    pub url: String,
    pub preview: String,
    pub provider: String,
    pub title: String,
    pub use_count: i64,
    pub added_at: String,
    pub last_used: Option<String>,
}

pub fn iso(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub async fn open_db(path: &str) -> Result<SqlitePool, sqlx::Error> {
    let opts = SqliteConnectOptions::from_str(path)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await?;
    sqlx::query(SCHEMA).execute(&pool).await?;
    Ok(pool)
}

pub async fn fetch(pool: &SqlitePool, id: &str) -> Result<Option<Favorite>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT id, url, preview, provider, title, added_at, last_used, use_count \
         FROM favorites WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    row.map(favorite_from_row).transpose()
}

fn favorite_from_row(row: sqlx::sqlite::SqliteRow) -> Result<Favorite, sqlx::Error> {
    let preview: Option<String> = row.try_get("preview")?;
    let provider: Option<String> = row.try_get("provider")?;
    let title: Option<String> = row.try_get("title")?;
    let added_at: i64 = row.try_get("added_at")?;
    let last_used: Option<i64> = row.try_get("last_used")?;
    Ok(Favorite {
        id: row.try_get("id")?,
        url: row.try_get("url")?,
        preview: preview.unwrap_or_default(),
        provider: provider.unwrap_or_default(),
        title: title.unwrap_or_default(),
        added_at: iso(added_at),
        last_used: last_used.map(iso),
        use_count: row.try_get("use_count")?,
    })
}

pub async fn count(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    let row = sqlx::query("SELECT COUNT(*) AS n FROM favorites")
        .fetch_one(pool)
        .await?;
    row.try_get("n")
}

pub async fn list_ids(
    pool: &SqlitePool,
    limit: i64,
    offset: i64,
) -> Result<Vec<String>, sqlx::Error> {
    // A negative LIMIT means "no limit" in SQLite, so -1 keeps the unbounded
    // default while still applying OFFSET.
    let rows = sqlx::query(
        "SELECT id FROM favorites \
         ORDER BY use_count DESC, last_used DESC, id ASC LIMIT ? OFFSET ?",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    rows
        .into_iter()
        .map(|r| r.try_get::<String, _>(0))
        .collect::<Result<Vec<_>, _>>()
}

pub struct NewFavorite<'a> {
    pub id: &'a str,
    pub url: &'a str,
    pub preview: &'a str,
    pub provider: &'a str,
    pub title: &'a str,
}

pub async fn upsert(pool: &SqlitePool, f: &NewFavorite<'_>, now: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO favorites (id, url, preview, provider, title, added_at, last_used, use_count) \
         VALUES (?, ?, ?, ?, ?, ?, NULL, 0) \
         ON CONFLICT(id) DO UPDATE SET \
             url = excluded.url, \
             preview = excluded.preview, \
             provider = excluded.provider, \
             title = excluded.title",
    )
    .bind(f.id)
    .bind(f.url)
    .bind(f.preview)
    .bind(f.provider)
    .bind(f.title)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn record_use(pool: &SqlitePool, id: &str, now: i64) -> Result<bool, sqlx::Error> {
    let res =
        sqlx::query("UPDATE favorites SET use_count = use_count + 1, last_used = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(pool)
            .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
    let res = sqlx::query("DELETE FROM favorites WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}
