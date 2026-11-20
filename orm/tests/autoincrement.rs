#![cfg(feature = "derive")]

use rusqlite_orm::{
    Entity,
    dao::Repository,
    database::{DdlVersion, builder::DatabaseConnectionBuilder},
};

/// Ids narrower than SQLite's rowid
#[derive(Entity, Debug, Clone, Default)]
#[entity(table = "tags")]
#[primary_key(id)]
pub struct Tag {
    #[autoincrement]
    pub id: u8,
    pub name: String,
}

const DDLS: [DdlVersion; 1] = [DdlVersion {
    version: 1,
    description: "tags",
    sql: "CREATE TABLE tags (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL);",
    update_fn: None,
}];

#[test]
fn narrow_ids_are_cast_and_overflow_fails() {
    pollster::block_on(async {
        let db = DatabaseConnectionBuilder::default().build("test").unwrap();
        db.create_schema(&DDLS).await.unwrap();

        let mut first = Tag {
            name: "first".into(),
            ..Default::default()
        };
        TagRepository::insert()
            .item(&mut first)
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(first.id, 1);

        db.run_in_connection(async |conn| {
            conn.execute(
                "UPDATE sqlite_sequence SET seq = 254 WHERE name = 'tags'",
                [],
            )
            .unwrap();
            Ok(())
        })
        .await
        .unwrap();
        let mut last = Tag {
            name: "last".into(),
            ..Default::default()
        };
        TagRepository::insert()
            .item(&mut last)
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(last.id, 255);

        // 256 doesn't fit a u8: the insert fails rather than truncating the id
        let mut over = Tag {
            name: "over".into(),
            ..Default::default()
        };
        assert!(
            TagRepository::insert()
                .item(&mut over)
                .execute(&db)
                .await
                .is_err()
        );
        assert_eq!(over.id, 0);
    });
}
