#![cfg(feature = "serialize")]

use rusqlite_orm::database::{DatabasePool, builder::DatabaseConnectionBuilder};

async fn names(db: &DatabasePool) -> Vec<String> {
    db.run_in_connection(async |conn| {
        let mut stmt = conn.prepare("SELECT name FROM items ORDER BY id")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<Result<Vec<String>, _>>()?)
    })
    .await
    .unwrap()
}

async fn insert(db: &DatabasePool, name: &'static str) {
    db.run_in_connection(async move |conn| {
        conn.execute("INSERT INTO items (name) VALUES (?1)", [name])?;
        Ok(())
    })
    .await
    .unwrap();
}

async fn user_version(db: &DatabasePool) -> i64 {
    db.run_in_connection(async |conn| Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?))
        .await
        .unwrap()
}

#[test]
fn in_memory_databases_start_as_independent_copies() {
    futures_lite::future::block_on(async {
        let original = DatabaseConnectionBuilder::default()
            .build("original")
            .unwrap();
        original
            .run_in_connection(async |conn| {
                Ok(conn.execute_batch(
                    "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
                     INSERT INTO items (name) VALUES ('a'), ('b');
                     PRAGMA user_version = 7;",
                )?)
            })
            .await
            .unwrap();

        let bytes = original.serialize().await.unwrap();
        let copy = |name: &str| {
            DatabaseConnectionBuilder::default()
                .enable_foreign_keys()
                .contents(bytes.clone())
                .build(name)
                .unwrap()
        };
        let first = copy("first");
        let second = copy("second");

        // Same data and schema version as the original
        assert_eq!(names(&first).await, ["a", "b"]);
        assert_eq!(user_version(&first).await, 7);

        // Writable, and each copy is on its own
        insert(&first, "c").await;
        assert_eq!(names(&first).await, ["a", "b", "c"]);
        assert_eq!(names(&second).await, ["a", "b"]);
        assert_eq!(names(&original).await, ["a", "b"]);

        // It grows past the size of the bytes it started from
        let count: i64 = second
            .run_in_connection(async |conn| {
                conn.execute_batch(
                    "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 5000)
                     INSERT INTO items (name) SELECT printf('item %d', i) FROM n;",
                )?;
                Ok(conn.query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))?)
            })
            .await
            .unwrap();
        assert_eq!(count, 5002);

        // The pragmas apply to the copied database
        let foreign_keys: i64 = first
            .run_in_connection(async |conn| {
                Ok(conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?)
            })
            .await
            .unwrap();
        assert_eq!(foreign_keys, 1);
    });
}
