# rusqlite_orm

The runtime crate of the [`rusqlite_orm`](../README.md) workspace: a lightweight, compile-time-checked ORM layer for [`rusqlite`](https://crates.io/crates/rusqlite), built around the `#[derive(Entity)]` procedural macro from [`rusqlite_orm_macros`](../macros). It provides the `Entity`/`Repository` traits, query builders, `Where`/`OrderBy` types, a pooled connection with transaction management, and schema migrations.

> `rusqlite_orm` and `rusqlite_orm_macros` are versioned and published together and are intended to be used as a pair — `rusqlite_orm` re-exports `rusqlite` itself, so you don't need to depend on `rusqlite` directly.

## Features

- **Derive-based entities** — annotate a struct with `#[derive(Entity)]` and get table metadata, row-to-struct mapping, and column constants for free.
- **A generated `Repository`** — every `#[derive(Entity)]` struct gets a companion `<Struct>Repository` unit struct implementing `rusqlite_orm::dao::Repository<Struct>`, which is where the query builders and generated lookups (`select_by_id`, `exists`, index helpers, ...) live.
- **Typed query builders** — `select()`, `insert()`, `update()`, `delete()` builders with a fluent API, called on the generated `<Struct>Repository`.
- **Rich `WHERE` clauses** — `Eq`, `NotEq`, `Gt`, `Lt`, `In`, `InMultiple` (tuple `IN`), `Null`, `NotNull`, combinable with `And` / `Or`.
- **Ordering, limits & pagination** — `OrderBy::Asc` / `OrderBy::Desc`, `.limit(n)` and `.offset(n)`.
- **Generated convenience methods** for entities with a struct-level `#[primary_key(field_a, field_b, ...)]` attribute:
  - on the **repository**: `exists`, `select_by_id` (and `_in_conn` variants);
  - on the **entity instance** itself: `update_by_id`, `delete_by_id` (and `_in_conn` variants).
- **Generated index lookups** — declare `#[index("name", (col_a, col_b))]` on the struct (repeatable) to get, on the repository, `select_by_name(...)` plus its `count_by_name` and `_in_conn` counterparts.
- **Generated unique-index lookups** — `#[unique("name", (col_d, col_e))]` uses the same syntax as `#[index(...)]`, but the generated `select_by_name` returns `Option<Self>` (at most one row) instead of `Vec<Self>`, has no `order_by` parameter, and its count counterpart is `exists_by_name` returning `bool`.
- **Relationships between entities** — annotate an `Option<T>` or `Vec<T>` field with `#[relationship((local_field, remote_column), ...)]` to get `fetch_<field>_relationship` / `fetch_<field>_relationship_in_conn` instance methods that lazily load the related row(s).
- **Optional derived `PartialEq` / `Eq` / `Hash`** based on the entity's id column(s), via `comparable` / `hashable` attribute flags.
- **Fields excluded from the schema** with `#[transient]`, populated via `Default::default()` when mapping rows back (requires the struct to implement `Default`). Relationship fields are excluded automatically the same way.
- **Multiple SQLite schemas** — `#[entity(schema = "...")]` attaches an entity to a schema other than `"main"` (e.g. an `ATTACH`ed database); every generated statement is qualified as `<schema>.<table>`.
- **Wide column type support** — the `Value` enum (`rusqlite_orm::dao::helpers::types::value::Value`) covers every signed/unsigned integer width (`i8`…`i64`, `isize`, `u8`…`u64`, `usize`), `f32`/`f64`, `bool`, `String`, `Vec<u8>` (BLOB, hex-encoded when logged) and `Option<T>` (mapped to `NULL`/the inner value).
- **Pooled connections** — `Database::initialize` opens an [`r2d2`](https://crates.io/crates/r2d2)-backed pool (via `r2d2_sqlite`, up to 8 connections, 5s connect/busy timeout) with `PRAGMA foreign_keys = ON` and `PRAGMA journal_mode = DELETE`, stored in a process-wide singleton.
- **Two ways to run statements** — `Database::run_in_connection(...)` borrows a pooled connection for one or more statements (not atomic across calls unless you wrap them yourself), and `Database::run_in_transaction(...)` runs the closure inside a single `rusqlite::Transaction`. Every query builder and generated helper exposes both a "managed" method (`execute`, `fetch_one`, ...) that opens its own pooled connection, and an `_in_conn` counterpart taking an explicit `&rusqlite::Connection` (a `&Transaction` works too, since it derefs to `Connection`) for composing multiple statements.
- **Cached prepared statements** — `SELECT` statements are prepared via `Connection::prepare_cached`, so repeated queries with the same shape reuse the cached statement.
- **SQL-file schema migrations** — the `dlls!("path")` macro embeds every `<version>_<description>.sql` file found in a directory (relative to the crate manifest) into a static array of `DdlVersion`s, applied in order and tracked via SQLite's `PRAGMA user_version`; when any migration is applied, `create_schema` runs `VACUUM` afterwards to reclaim space.
- **Query logging** — every generated statement is logged (via the `log` crate) with parameters interpolated, plus the number of affected/fetched rows.

## Installation

Add both crates to your `Cargo.toml`:

```toml
[dependencies]
rusqlite_orm = "0.4"
rusqlite_orm_macros = "0.4"
```

`rusqlite_orm` re-exports `rusqlite`, accessible as `rusqlite_orm::rusqlite`, so most consumers won't need to add `rusqlite` as a separate dependency.

## Quick start

### 1. Define an entity

```rust
use rusqlite_orm::dao::Entity;
use rusqlite_orm_macros::Entity;

#[derive(Entity, Debug, Clone, Default)]
#[entity(table = "users", comparable = true, hashable = true)]
#[primary_key(id)]
#[index("email", (email))]
pub struct User {
    pub id: i64,
    #[column("email_address")]
    pub email: String,
    pub name: String,
    #[transient]
    pub transient_flag: bool,
}
```

`#[entity("users")]` is shorthand for `#[entity(table = "users")]` when you don't need `schema`/`comparable`/`hashable`.

This expands into:

- an `entity::columns` module with a typed constant per persisted column (`entity::columns::ID`, `entity::columns::EMAIL_ADDRESS`, `entity::columns::NAME`), plus `entity::TABLE` and `entity::SCHEMA`,
- an implementation of the `Entity` trait for `User` (`SCHEMA`, `TABLE_NAME`, `FIELDS`, `map_from_row`, `get_values`),
- `user.update_by_id()` / `user.delete_by_id()` **instance methods** on `User` (because the struct has a `#[primary_key(id)]` attribute), each with an `_in_conn` counterpart,
- a `UserRepository` unit struct implementing `rusqlite_orm::dao::Repository<User>`, with:
  - `UserRepository::exists(id)` / `UserRepository::select_by_id(id)` (and `_in_conn` variants),
  - `UserRepository::select_by_email(email, order_by)` and `UserRepository::count_by_email(email)` (because of the `#[index("email", (email))]` attribute), plus their `_in_conn` variants,
- `PartialEq` / `Eq` and `Hash` implementations for `User` based on `id` (because `comparable` and `hashable` are set to `true`).

`#[unique(...)]` works exactly like `#[index(...)]` but marks the index as unique. For example, replacing the attribute above with `#[unique("email", (email))]` generates `UserRepository::select_by_email(email)` returning `Option<User>` (no `order_by` parameter, since a unique index matches at most one row) and `UserRepository::exists_by_email(email)`, plus their `_in_conn` variants. An index can also mix variable columns with fixed conditions, e.g. `#[unique("active_tenant_username", (tenant_id, username), (status = "active"))]`. Note that the attribute only generates the lookup helpers — it does **not** create a `UNIQUE` constraint in the database; that still needs to be declared in your DDL. See [`macros/README.md`](../macros/README.md#indexes-and-unique-indexes) for the full syntax.

`#[transient]` fields are skipped when building `INSERT`/`SELECT` column lists and are restored to their `Default` value when a row is mapped back into the struct.

### 2. Define your schema as versioned SQL files

Create a directory (e.g. `migrations/`) next to your crate manifest with files named `<version>_<name>.sql`. The **first line must be a `--` comment** describing the migration:

```sql
-- create users table
CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    email_address TEXT NOT NULL,
    name TEXT NOT NULL
);
```

Embed them at compile time with the `dlls!` macro:

```rust
rusqlite_orm_macros::dlls!("migrations");
// expands to: pub static DDLS: [rusqlite_orm::database::DdlVersion; N] = [ ... ];
```

### 3. Open the database and apply the schema

```rust
use rusqlite_orm::database::Database;

Database::initialize("app.db")?;
Database::create_schema(&DDLS)?;
```

`Database::initialize` opens a pooled connection (up to 8 connections) and stores it in a process-wide singleton — call it once at startup; calling it again returns `DatabaseError::AlreadyInitialized`. `create_schema` reads the current `PRAGMA user_version`, applies every migration whose version is higher, and updates the schema inside a single transaction. If at least one migration was applied, it then runs `VACUUM` to reclaim space.

### 4. CRUD operations

All query builders (`select`, `insert`, `update`, `delete`) and the generated lookup helpers live on the `<Struct>Repository` type generated by `#[derive(Entity)]` — in this case, `UserRepository`. Most of them come in a **managed** form (opens its own pooled connection) and an **`_in_conn`** form (takes an explicit `&rusqlite::Connection`, for composing several statements together). Two builders are asymmetric: `SelectBuilder` has no managed multi-row `fetch()` (only the managed `fetch_one()`/`count()`, returning `Option<T>`/`i64`), and `DeleteBuilder` has no managed `execute()` at all — both need to be run inside `Database::run_in_connection`/`run_in_transaction` when you want more than a single generated helper call.

```rust
use rusqlite_orm::{
    database::Database,
    dao::{
        Repository,
        helpers::types::{order_by::OrderBy, where_clause::Where},
    },
};

// INSERT (managed: opens its own pooled connection)
let user = User { id: 0, email: "alice@example.com".into(), name: "Alice".into(), transient_flag: false };
UserRepository::insert().item(user.clone()).or_ignore(false).execute()?;

// SELECT with WHERE / ORDER BY / LIMIT / OFFSET — needs an explicit connection
let users = Database::run_in_connection(|conn| {
    Ok(UserRepository::select()
        .where_(Where::Eq(entity::columns::EMAIL_ADDRESS, "alice@example.com".into()))
        .order_by(OrderBy::Asc(entity::columns::ID))
        .limit(10)
        .offset(20)
        .fetch_in_conn(conn)?)
})?;

// Generated helpers (on the repository) — each has a managed form
let by_id = UserRepository::select_by_id(1)?;
let by_email = UserRepository::select_by_email("alice@example.com", None)?;
let count_by_email = UserRepository::count_by_email("alice@example.com")?;
let found = UserRepository::exists(1)?;

// UPDATE
user.update_by_id()?; // instance method: updates all non-id columns by id

// Or a manual UPDATE builder
UserRepository::update()
    .set(entity::columns::NAME, "Alicia".into())
    .where_(Where::Eq(entity::columns::ID, 1.into()))
    .execute()?;

// DELETE — no managed `execute()`, run it inside a connection
Database::run_in_connection(|conn| Ok(user.delete_by_id_in_conn(conn)?))?;
```

### 5. Composing statements in a single transaction

Every builder and repository helper exposes an `_in_conn` variant so several statements can share one connection or transaction (a `&mut Transaction` works anywhere a `&rusqlite::Connection` is expected, since `Transaction` derefs to `Connection`):

```rust
use rusqlite_orm::database::Database;

let fetched = Database::run_in_transaction(|tx| {
    UserRepository::insert().item(user.clone()).execute_in_conn(tx)?;
    Ok(UserRepository::select_by_id_in_conn(tx, 1)?)
})?;
```

The closure passed to `run_in_transaction` (and `run_in_connection`) returns `std::result::Result<R, Box<dyn std::error::Error + Send + Sync>>` rather than the crate's own `Result<R>`, so it can propagate any error type with `?` (not just `DatabaseError`) — useful when composing `_in_conn` calls with your own fallible logic inside the same connection/transaction. Any error returned from the closure is wrapped into `DatabaseError::Transaction` (for `run_in_transaction`) or `DatabaseError::RunningOnConnection` (for `run_in_connection`).

### 6. Relationships

A field of type `Option<T>` or `Vec<T>` can be annotated with `#[relationship(...)]` to link an entity to related row(s) in another table, without the field being part of the entity's own columns:

```rust
#[derive(Entity, Debug, Clone, Default)]
#[entity(table = "posts")]
#[primary_key(id)]
pub struct Post {
    pub id: i64,
    pub user_id: i64,
    pub title: String,

    // "belongs to" — Option<T>, loaded with fetch_one_in_conn
    #[relationship((user_id, user::entity::columns::ID))]
    pub author: Option<User>,

    // "has many" — Vec<T>, loaded with fetch_in_conn
    #[relationship((id, comment::entity::columns::POST_ID))]
    pub comments: Vec<Comment>,
}
```

Each `(local_field, remote_column)` pair builds an `Eq` condition between a field on the current struct and a typed column constant on the related entity; several pairs are combined with `AND`, which lets you model composite-key joins. Relationship fields behave like `#[transient]` fields under the hood: they're skipped by `INSERT`/`SELECT` and start out as `Default::default()`.

The macro adds instance methods on the struct itself (not the repository) to lazily populate the field:

```rust
let mut post = PostRepository::select_by_id(1)?.unwrap();

post.fetch_author_relationship()?;    // fills post.author: Option<User>
post.fetch_comments_relationship()?;  // fills post.comments: Vec<Comment>

// Composable in an existing connection/transaction:
Database::run_in_transaction(|tx| {
    post.fetch_author_relationship_in_conn(tx)?;
    Ok(post.fetch_comments_relationship_in_conn(tx)?)
})?;
```

See [`macros/README.md`](../macros/README.md#relationships) for the full attribute syntax, including composite joins.

## `WHERE` clause reference

`Where<T>` (in `rusqlite_orm::dao::helpers::types::where_clause`) supports:

| Variant                         | SQL                                       |
| ------------------------------- | ----------------------------------------- |
| `Where::Eq(col, val)`           | `col = ?`                                 |
| `Where::NotEq(col, val)`        | `col != ?`                                |
| `Where::Gt(col, val)`           | `col > ?`                                 |
| `Where::Lt(col, val)`           | `col < ?`                                 |
| `Where::In(col, vals)`          | `col IN (?, ?, ...)`                      |
| `Where::InMultiple(cols, rows)` | `(col_a, col_b) IN ((?, ?), (?, ?), ...)` |
| `Where::Null(col)`              | `col IS NULL`                             |
| `Where::NotNull(col)`           | `col IS NOT NULL`                         |
| `Where::And(conditions)`        | `(...) AND (...)`                         |
| `Where::Or(conditions)`         | `(...) OR (...)`                          |

## Error handling

All fallible operations return `rusqlite_orm::database::errors::Result<T>`, an alias for `Result<T, DatabaseError>`. `DatabaseError` (via `thiserror`) distinguishes: `ClosedConnection` (an operation was attempted before `Database::initialize`), `AlreadyInitialized` (`initialize` called more than once), `Connection`/`Pool` (opening or borrowing from the `r2d2` pool), `SchemaCreation`, `Insert`, `Update`, `Select` and `Delete` — the last five wrapping the underlying `rusqlite::Error` — and `Transaction`/`RunningOnConnection`, which wrap a `Box<dyn std::error::Error + Send + Sync>` returned from a `run_in_transaction`/`run_in_connection` closure (see [Composing statements in a single transaction](#5-composing-statements-in-a-single-transaction) above).

## Crate details

See the [top-level README](../README.md) for the workspace overview, and [`macros/README.md`](../macros/README.md) for the full list of supported `#[derive(Entity)]` attributes, including `#[relationship(...)]`.

## License

Both crates are distributed under the [MIT license](https://opensource.org/licenses/MIT).
