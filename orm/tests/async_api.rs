#![cfg(feature = "derive")]

use std::future::Future;

use rusqlite_orm::{
    dao::Repository,
    database::{DatabasePool, DdlVersion, builder::DatabaseConnectionBuilder},
    errors::DatabaseError,
    types::{expression::Expression, order_by::OrderBy, where_clause::Where},
};

mod author {
    use rusqlite_orm::Entity;

    use super::book::Book;

    #[derive(Entity, Debug, Clone, Default)]
    #[entity(table = "authors")]
    #[primary_key(id)]
    #[unique("name", (name))]
    pub struct Author {
        #[autoincrement]
        pub id: i64,
        pub name: String,

        #[relationship((id, super::book::entity::columns::AUTHOR_ID))]
        pub books: Vec<Book>,
    }
}

mod book {
    use rusqlite_orm::Entity;

    use super::author::Author;

    #[derive(Entity, Debug, Clone, Default)]
    #[entity(table = "books")]
    #[primary_key(id)]
    #[index("author", (author_id))]
    pub struct Book {
        pub id: i64,
        pub author_id: i64,
        pub title: String,

        #[relationship((author_id, super::author::entity::columns::ID))]
        pub author: Option<Author>,
    }
}

use author::{Author, AuthorRepository, entity as author_entity};
use book::{Book, BookRepository, entity as book_entity};

const DDLS: [DdlVersion; 1] = [DdlVersion {
    version: 1,
    description: "authors and books",
    sql: "CREATE TABLE authors (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE);
          CREATE TABLE books (id INTEGER PRIMARY KEY, author_id INTEGER NOT NULL, title TEXT NOT NULL);",
    update_fn: None,
}];

/// Fails to compile if the future can't be moved to another thread, as Tauri commands need.
fn assert_send<F: Future + Send>(future: F) -> F {
    future
}

async fn open() -> DatabasePool {
    let db = DatabaseConnectionBuilder::default().build("test").unwrap();
    assert_send(db.create_schema(&DDLS)).await.unwrap();
    db
}

#[test]
fn full_async_round_trip() {
    pollster::block_on(async {
        let db = open().await;

        let mut ann = Author {
            name: "Ann".into(),
            ..Default::default()
        };
        let mut bob = Author {
            name: "Bob".into(),
            ..Default::default()
        };
        let inserted = assert_send(
            AuthorRepository::insert()
                .item(&mut ann)
                .item(&mut bob)
                .execute(&db),
        )
        .await
        .unwrap();
        assert_eq!(inserted, 2);
        assert!(
            ann.id > 0 && bob.id > ann.id,
            "autoincrement ids are written back"
        );

        let mut books: Vec<Book> = (1..=3)
            .map(|id| Book {
                id,
                author_id: if id == 3 { bob.id } else { ann.id },
                title: format!("Book {id}"),
                ..Default::default()
            })
            .collect();
        let mut insert = BookRepository::insert();
        for book in &mut books {
            insert = insert.item(book);
        }
        assert_eq!(insert.execute(&db).await.unwrap(), 3);

        // Builders
        let titles: Vec<String> = assert_send(
            BookRepository::select()
                .where_(Where::Eq(book_entity::columns::AUTHOR_ID, ann.id.into()))
                .order_by(OrderBy::Desc(book_entity::columns::ID))
                .fetch(&db),
        )
        .await
        .unwrap()
        .into_iter()
        .map(|b| b.title)
        .collect();
        assert_eq!(titles, ["Book 2", "Book 1"]);
        assert_eq!(BookRepository::select().count(&db).await.unwrap(), 3);
        let rows = BookRepository::select()
            .columns(&[book_entity::columns::TITLE])
            .fetch_one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rows.get_text("title"), "Book 1");

        // Generated helpers
        assert!(
            assert_send(AuthorRepository::exists(&db, ann.id))
                .await
                .unwrap()
        );
        assert!(!AuthorRepository::exists(&db, 999).await.unwrap());
        let found = assert_send(AuthorRepository::select_by_name(&db, "Bob"))
            .await
            .unwrap();
        assert_eq!(found.map(|a| a.id), Some(bob.id));
        assert!(AuthorRepository::exists_by_name(&db, "Ann").await.unwrap());
        let by_author = assert_send(BookRepository::select_by_author(
            &db,
            ann.id,
            Some(&[OrderBy::Asc(book_entity::columns::TITLE)]),
        ))
        .await
        .unwrap();
        assert_eq!(by_author.len(), 2);
        assert_eq!(
            BookRepository::count_by_author(&db, bob.id).await.unwrap(),
            1
        );

        // Relationships
        let mut ann_loaded = AuthorRepository::select_by_id(&db, ann.id)
            .await
            .unwrap()
            .unwrap();
        assert_send(ann_loaded.fetch_books_relationship(&db))
            .await
            .unwrap();
        assert_eq!(ann_loaded.books.len(), 2);
        let mut book = BookRepository::select_by_id(&db, 3).await.unwrap().unwrap();
        book.fetch_author_relationship(&db).await.unwrap();
        assert_eq!(book.author.as_ref().map(|a| a.name.as_str()), Some("Bob"));

        // update_by_id / delete_by_id / update / delete builders
        book.title = "Renamed".into();
        assert_send(book.update_by_id(&db)).await.unwrap();
        assert_eq!(
            BookRepository::select_by_id(&db, 3)
                .await
                .unwrap()
                .unwrap()
                .title,
            "Renamed"
        );
        let updated = BookRepository::update()
            .set(book_entity::columns::TITLE, "Same".into())
            .where_(Where::Eq(book_entity::columns::AUTHOR_ID, ann.id.into()))
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(updated, 2);
        assert_send(book.delete_by_id(&db)).await.unwrap();
        let deleted = BookRepository::delete().execute(&db).await.unwrap();
        assert_eq!(deleted, 2);
        assert_eq!(BookRepository::select().count(&db).await.unwrap(), 0);
    });
}

#[test]
fn transactions_commit_or_roll_back() {
    pollster::block_on(async {
        let db = open().await;

        let author = assert_send(db.run_in_transaction(async |tx| {
            let mut author = Author {
                name: "Ann".into(),
                ..Default::default()
            };
            AuthorRepository::insert()
                .item(&mut author)
                .execute_in(tx)
                .await?;
            Ok(author)
        }))
        .await
        .unwrap();
        assert!(AuthorRepository::exists(&db, author.id).await.unwrap());

        let err = db
            .run_in_transaction(async |tx| {
                let mut other = Author {
                    name: "Bob".into(),
                    ..Default::default()
                };
                AuthorRepository::insert()
                    .item(&mut other)
                    .execute_in(tx)
                    .await?;
                // Same unique name: fails, and the whole transaction rolls back.
                let mut dup = Author {
                    name: "Bob".into(),
                    ..Default::default()
                };
                AuthorRepository::insert()
                    .item(&mut dup)
                    .execute_in(tx)
                    .await?;
                Ok(())
            })
            .await
            .unwrap_err();
        assert!(matches!(err, DatabaseError::Transaction(_)));
        assert!(!AuthorRepository::exists_by_name(&db, "Bob").await.unwrap());

        let count =
            assert_send(db.run_in_connection(async |conn| {
                Ok(AuthorRepository::select().count_in(conn).await?)
            }))
            .await
            .unwrap();
        assert_eq!(count, 1);
    });
}

#[test]
fn or_ignore_skips_ids_of_rows_not_inserted() {
    pollster::block_on(async {
        let db = open().await;

        let mut ann = Author {
            name: "Ann".into(),
            ..Default::default()
        };
        AuthorRepository::insert()
            .item(&mut ann)
            .execute(&db)
            .await
            .unwrap();

        let mut dup = Author {
            name: "Ann".into(),
            ..Default::default()
        };
        let mut new = Author {
            name: "Cid".into(),
            ..Default::default()
        };
        let inserted = AuthorRepository::insert()
            .item(&mut dup)
            .item(&mut new)
            .or_ignore()
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(inserted, 1);
        assert_eq!(dup.id, 0, "ignored row keeps its id untouched");
        assert!(new.id > ann.id);
    });
}

#[test]
fn generated_in_helpers_are_async() {
    pollster::block_on(async {
        let db = open().await;

        let (author, books) = db
            .run_in_transaction(async |tx| {
                let mut ann = Author {
                    name: "Ann".into(),
                    ..Default::default()
                };
                AuthorRepository::insert()
                    .item(&mut ann)
                    .execute_in(tx)
                    .await?;
                let mut book = Book {
                    id: 1,
                    author_id: ann.id,
                    title: "Book 1".into(),
                    ..Default::default()
                };
                BookRepository::insert()
                    .item(&mut book)
                    .execute_in(tx)
                    .await?;

                assert!(AuthorRepository::exists_in(tx, ann.id).await?);
                assert!(AuthorRepository::exists_by_name_in(tx, "Ann").await?);
                assert_eq!(BookRepository::count_by_author_in(tx, ann.id).await?, 1);
                let by_author = BookRepository::select_by_author_in(tx, ann.id, None).await?;

                book.title = "Renamed".into();
                book.update_by_id_in(tx).await?;
                let mut loaded = BookRepository::select_by_id_in(tx, 1).await?.unwrap();
                loaded.fetch_author_relationship_in(tx).await?;
                assert_eq!(loaded.title, "Renamed");
                assert_eq!(loaded.author.as_ref().map(|a| a.id), Some(ann.id));

                let mut ann_loaded = AuthorRepository::select_by_name_in(tx, "Ann")
                    .await?
                    .unwrap();
                ann_loaded.fetch_books_relationship_in(tx).await?;
                assert_eq!(ann_loaded.books.len(), 1);

                loaded.delete_by_id_in(tx).await?;
                Ok((ann, by_author))
            })
            .await
            .unwrap();

        assert_eq!(books.len(), 1);
        assert_eq!(books[0].author_id, author.id);
        assert_eq!(BookRepository::select().count(&db).await.unwrap(), 0);
    });
}

async fn rename_authors(
    tx: &mut rusqlite_orm::rusqlite::Transaction<'_>,
) -> rusqlite_orm::errors::Result<()> {
    AuthorRepository::update()
        .set(author_entity::columns::NAME, "Migrated".into())
        .execute_in(tx)
        .await?;
    Ok(())
}

#[test]
fn async_update_fn_runs_in_migration() {
    pollster::block_on(async {
        let db = open().await;
        AuthorRepository::insert()
            .item(&mut Author {
                name: "Ann".into(),
                ..Default::default()
            })
            .execute(&db)
            .await
            .unwrap();

        let next = [DdlVersion {
            version: DDLS.iter().map(|d| d.version).max().unwrap() + 1,
            description: "rename authors",
            sql: "SELECT 1;",
            update_fn: Some(|tx| Box::pin(rename_authors(tx))),
        }];
        db.create_schema(&next).await.unwrap();

        assert!(
            AuthorRepository::exists_by_name(&db, "Migrated")
                .await
                .unwrap()
        );
    });
}

#[test]
fn aggregates_and_group_by() {
    pollster::block_on(async {
        let db = open().await;

        let mut books: Vec<Book> = [(1, 10), (2, 10), (3, 20), (4, 10), (5, 30)]
            .into_iter()
            .map(|(id, author_id)| Book {
                id,
                author_id,
                title: format!("Book {id}"),
                ..Default::default()
            })
            .collect();
        let mut insert = BookRepository::insert();
        for book in &mut books {
            insert = insert.item(book);
        }
        insert.execute(&db).await.unwrap();

        // Unaliased functions are read by their SQL text, aliased ones by their alias
        let groups: Vec<(i64, i64, i64)> = BookRepository::select()
            .columns(&[
                book_entity::columns::AUTHOR_ID.into(),
                book_entity::columns::ID.max(),
                Expression::count_all().alias("total"),
            ])
            .where_(Where::NotEq(book_entity::columns::AUTHOR_ID, 30.into()))
            .group_by(&[book_entity::columns::AUTHOR_ID])
            .order_by(OrderBy::Asc(book_entity::columns::AUTHOR_ID))
            .fetch(&db)
            .await
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r.get_int("author_id"),
                    r.get_int("MAX(id)"),
                    r.get_int("total"),
                )
            })
            .collect();
        assert_eq!(groups, [(10, 4, 3), (20, 3, 1)]);

        // Counting a grouped select counts its groups
        let authors = BookRepository::select()
            .columns(&[book_entity::columns::AUTHOR_ID])
            .group_by(&[book_entity::columns::AUTHOR_ID])
            .count(&db)
            .await
            .unwrap();
        assert_eq!(authors, 3);

        // Without GROUP BY a function aggregates every matching row
        let row = BookRepository::select()
            .columns(&[
                book_entity::columns::ID.min().alias("first"),
                book_entity::columns::ID.sum().alias("sum"),
                book_entity::columns::ID.avg().alias("avg"),
                book_entity::columns::TITLE.count().alias("titles"),
            ])
            .fetch_one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.get_int("first"), 1);
        assert_eq!(row.get_int("sum"), 15);
        assert_eq!(row.get_f64("avg"), 3.0);
        assert_eq!(row.get_int("titles"), 5);

        // As a subquery: the latest book of each author
        let latest = BookRepository::select()
            .columns(&[
                book_entity::columns::AUTHOR_ID.into(),
                book_entity::columns::ID.max(),
            ])
            .group_by(&[book_entity::columns::AUTHOR_ID])
            .to_subquery();
        let titles: Vec<String> = BookRepository::select()
            .where_(Where::InMultipleSub(
                vec![book_entity::columns::AUTHOR_ID, book_entity::columns::ID],
                latest,
            ))
            .order_by(OrderBy::Asc(book_entity::columns::ID))
            .fetch(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|b| b.title)
            .collect();
        assert_eq!(titles, ["Book 3", "Book 4", "Book 5"]);
    });
}
