use std::path::{Path, PathBuf};

use rusqlite_orm::{
    database::{
        DatabasePool,
        builder::{DatabaseConnectionBuilder, JournalMode, Synchronous},
    },
    errors::DatabaseError,
};

/// Path of a database file unique to this process and `name`, with no leftovers from earlier runs.
fn db_path(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusqlite-orm-maintenance-{}-{name}.db",
        std::process::id()
    ));
    remove_db(&path);
    path
}

/// Removes the database file at `path` along with its WAL and shared-memory files.
fn remove_db(path: &Path) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut file = path.as_os_str().to_owned();
        file.push(suffix);
        let _ = std::fs::remove_file(file);
    }
}

fn open(path: &Path, journal_mode: JournalMode) -> DatabasePool {
    DatabaseConnectionBuilder::default()
        .location(path)
        .pool_size(2)
        .min_idle(1)
        .journal_mode(journal_mode)
        .synchronous(Synchronous::Normal)
        .build("maintenance")
        .unwrap()
}

async fn pragma(db: &DatabasePool, name: &'static str) -> String {
    db.run_in_connection(async move |conn| {
        Ok(conn.query_row(&format!("PRAGMA {name}"), [], |r| {
            r.get::<_, rusqlite_orm::rusqlite::types::Value>(0)
        })?)
    })
    .await
    .map(|v| match v {
        rusqlite_orm::rusqlite::types::Value::Integer(i) => i.to_string(),
        rusqlite_orm::rusqlite::types::Value::Text(t) => t,
        other => format!("{other:?}"),
    })
    .unwrap()
}

async fn insert_rows(db: &DatabasePool, rows: i64) {
    db.run_in_transaction(async move |tx| {
        tx.execute_batch("CREATE TABLE IF NOT EXISTS items (id INTEGER PRIMARY KEY, name TEXT);")?;
        for id in 0..rows {
            tx.execute(
                "INSERT INTO items (id, name) VALUES (?1, ?2)",
                (id, format!("item {id}")),
            )?;
        }
        Ok(())
    })
    .await
    .unwrap();
}

async fn count_rows(db: &DatabasePool) -> i64 {
    db.run_in_connection(async |conn| {
        Ok(conn.query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))?)
    })
    .await
    .unwrap()
}

#[test]
fn synchronous_is_applied_to_every_connection() {
    futures_lite::future::block_on(async {
        let path = db_path("synchronous");
        let db = open(&path, JournalMode::Wal);

        assert_eq!(pragma(&db, "journal_mode").await, "wal");
        // NORMAL is 1
        assert_eq!(pragma(&db, "synchronous").await, "1");

        drop(db);
        remove_db(&path);
    });
}

#[test]
fn synchronous_defaults_to_sqlite_default() {
    futures_lite::future::block_on(async {
        let path = db_path("synchronous-default");
        let db = DatabaseConnectionBuilder::default()
            .location(&path)
            .pool_size(1)
            .min_idle(1)
            .build("maintenance")
            .unwrap();

        // FULL is 2
        assert_eq!(pragma(&db, "synchronous").await, "2");

        drop(db);
        remove_db(&path);
    });
}

#[test]
fn vacuum_into_copies_the_committed_data_still_in_the_wal() {
    futures_lite::future::block_on(async {
        let path = db_path("source");
        let copy = db_path("copy");
        let db = open(&path, JournalMode::Wal);
        insert_rows(&db, 100).await;

        db.vacuum_into(&copy).await.unwrap();

        let copied = open(&copy, JournalMode::Delete);
        assert_eq!(count_rows(&copied).await, 100);
        // The source is left as it was
        assert_eq!(count_rows(&db).await, 100);

        drop((db, copied));
        remove_db(&path);
        remove_db(&copy);
    });
}

#[test]
fn vacuum_into_fails_on_an_existing_file() {
    futures_lite::future::block_on(async {
        let path = db_path("source-existing");
        let copy = db_path("copy-existing");
        let db = open(&path, JournalMode::Wal);
        insert_rows(&db, 1).await;
        db.vacuum_into(&copy).await.unwrap();

        let res = db.vacuum_into(&copy).await;
        assert!(matches!(res, Err(DatabaseError::Vacuum(_))));

        drop(db);
        remove_db(&path);
        remove_db(&copy);
    });
}

#[test]
fn vacuum_reclaims_the_space_of_deleted_rows() {
    futures_lite::future::block_on(async {
        let path = db_path("vacuum");
        let db = open(&path, JournalMode::Delete);
        insert_rows(&db, 5000).await;
        db.run_in_connection(async |conn| Ok(conn.execute_batch("DELETE FROM items;")?))
            .await
            .unwrap();
        let pages_before: i64 = pragma(&db, "page_count").await.parse().unwrap();

        db.vacuum().await.unwrap();

        let pages_after: i64 = pragma(&db, "page_count").await.parse().unwrap();
        assert!(pages_after < pages_before);
        assert_eq!(pragma(&db, "freelist_count").await, "0");

        drop(db);
        remove_db(&path);
    });
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut file = path.as_os_str().to_owned();
    file.push(suffix);
    PathBuf::from(file)
}

#[test]
fn close_removes_the_wal_files_even_with_live_clones() {
    futures_lite::future::block_on(async {
        let path = db_path("close");
        let db = open(&path, JournalMode::Wal);
        insert_rows(&db, 10).await;
        assert!(sidecar(&path, "-wal").exists());
        assert!(sidecar(&path, "-shm").exists());

        // A clone kept alive, as an app's state that is never dropped
        let clone = db.clone();
        db.close().await;

        assert!(clone.is_closed());
        assert!(!sidecar(&path, "-wal").exists());
        assert!(!sidecar(&path, "-shm").exists());

        // The data was checkpointed into the database file
        let reopened = open(&path, JournalMode::Wal);
        assert_eq!(count_rows(&reopened).await, 10);

        drop(reopened);
        remove_db(&path);
    });
}

#[test]
fn closed_pool_rejects_every_call() {
    futures_lite::future::block_on(async {
        let path = db_path("closed");
        let db = open(&path, JournalMode::Wal);
        let clone = db.clone();
        db.close().await;
        // Closing twice does nothing
        db.close().await;

        let res = clone
            .run_in_connection(async |conn| Ok(conn.execute_batch("SELECT 1;")?))
            .await;
        assert!(matches!(res, Err(DatabaseError::ClosedConnection())));
        assert!(matches!(
            clone.vacuum().await,
            Err(DatabaseError::ClosedConnection())
        ));

        remove_db(&path);
    });
}
