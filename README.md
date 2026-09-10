# rusqlite_orm

A lightweight, compile-time-checked ORM layer for [`rusqlite`](https://crates.io/crates/rusqlite), built around a `#[derive(Entity)]` procedural macro. It generates table metadata, row mapping, and typed query helpers (select / insert / update / delete) for your structs, gives you a configurable pooled SQLite connection, and ships a small SQL-file-based schema migration system.

This repository is a Cargo workspace made up of two crates:

| Crate                             | Path      | Description                                                                                                                        |
| --------------------------------- | --------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| [`rusqlite_orm`](./orm)           | `orm/`    | The runtime ORM: `Entity` trait, `Repository` trait, query builders, `Where`/`OrderBy` types, pooled connections and schema migrations. |
| [`rusqlite_orm_macros`](./macros) | `macros/` | The `#[derive(Entity)]` procedural macro and the `dlls!` macro used to embed SQL migration files at compile time.                  |

> Both crates are versioned and published together and are intended to be used as a pair — `rusqlite_orm` re-exports `rusqlite` itself, so you don't need to depend on `rusqlite` directly.

## Features

- **Derive-based entities** — annotate a struct with `#[derive(Entity)]` and get table metadata, row-to-struct mapping, and column constants for free.
- **A generated `Repository`** — every `#[derive(Entity)]` struct gets a companion `<Struct>Repository` unit struct implementing `rusqlite_orm::dao::Repository<Struct>`, which is where the query builders and generated lookups (`select_by_id`, `exists`, index helpers, ...) live.
- **Typed query builders** — `select()`, `insert()`, `update()`, `delete()` builders with a fluent API, called on the generated `<Struct>Repository`.
- **Rich `WHERE` clauses** — `Eq`, `NotEq`, `Gt`, `Gte`, `Lt`, `Lte`, `In`, `InMultiple` (tuple `IN`), `Null`, `NotNull`, combinable with `And` / `Or`, plus subquery variants (`EqSub`, `NotEqSub`, `InSub`, `NotInSub`, `InMultipleSub`) that embed another `SelectBuilder` (via `.to_subquery()`) inside the condition. Any comparison value can also be `Value::Raw(sql)` to splice a SQL expression (e.g. a SQLite function call like `CURRENT_TIMESTAMP`) in place of a bound parameter.
- **Ordering, limits & pagination** — `OrderBy::Asc` / `OrderBy::Desc`, `.limit(n)` and `.offset(n)`.
- **Raw / non-mapped selects** — calling `.columns(&[...])` or `.distinct(&[...])` on `select()` switches it from returning `Vec<Entity>` to returning `Row`/`Rows` (a simple column-name → `Value` map), for projections that don't need to cover every persisted column.
- **Generated convenience methods** for entities with a struct-level `#[primary_key(field_a, field_b, ...)]` attribute:
  - on the **repository**: `exists`, `select_by_id` (and `_in` variants);
  - on the **entity instance** itself: `update_by_id`, `delete_by_id` (and `_in` variants).
- **Generated index lookups** — declare `#[index("name", (col_a, col_b))]` on the struct (repeatable) to get, on the repository, `select_by_name(...)` plus its `count_by_name` and `_in_conn` counterparts (index/unique/relationship helpers use the `_in_conn` suffix, unlike the builders and primary-key helpers described above, which use plain `_in`).
- **Generated unique-index lookups** — `#[unique("name", (col_d, col_e))]` uses the same syntax as `#[index(...)]`, but the generated `select_by_name` returns `Option<Self>` (at most one row) instead of `Vec<Self>`, has no `order_by` parameter, and its count counterpart is `exists_by_name` returning `bool`.
- **Relationships between entities** — annotate an `Option<T>` or `Vec<T>` field with `#[relationship((local_field, remote_column), ...)]` to get `fetch_<field>_relationship` / `fetch_<field>_relationship_in_conn` instance methods that lazily load the related row(s).
- **Optional derived `PartialEq` / `Eq` / `Hash`** based on the entity's id column(s), via `comparable` / `hashable` attribute flags.
- **Fields excluded from the schema** with `#[transient]`, populated via `Default::default()` when mapping rows back (requires the struct to implement `Default`). Relationship fields are excluded automatically the same way.
- **Columns excluded from `INSERT` only** with `#[default]` — the field stays in `SELECT`/`FIELDS`, but is left out of the generated `INSERT` statement so SQLite applies its own column default.
- **Autoincrement primary keys** — `#[autoincrement]` (implies `#[default]`) on a single `i64` field gets the generated `rowid` written back into that field after each insert.
- **Multiple SQLite schemas** — `#[entity(schema = "...")]` attaches an entity to a schema other than `"main"` (e.g. an `ATTACH`ed database); every generated statement is qualified as `<schema>.<table>`.
- **Wide column type support** — every signed/unsigned integer width, `f32`/`f64`, `bool`, `String`, `Vec<u8>` (BLOB) and `Option<T>` map onto `rusqlite_orm::types::value::Value` out of the box, plus a `Value::Raw(String)` variant for SQL literals/function calls that must not be bound as a parameter.
- **Configurable pooled connections** — `DatabaseConnectionBuilder` is a typestate builder: it starts in an in-memory state (single connection, `PRAGMA journal_mode = MEMORY`), and calling `.location(path)` switches it to a file-backed state with its own defaults (pool of 5, `PRAGMA journal_mode = DELETE`). Pool size, min idle connections, connection/busy timeouts, journal mode and `PRAGMA foreign_keys` are all configurable before calling `.build(name)`, which opens an [`r2d2`](https://crates.io/crates/r2d2)-backed pool (via `r2d2_sqlite`) and hands you back an owned `DatabasePool` — there is no global singleton, so you're free to build more than one.
- **Two ways to run statements** — `DatabasePool::run_in_connection(...)` borrows a pooled connection for one or more statements (not atomic across calls unless you wrap them yourself), and `DatabasePool::run_in_transaction(...)` runs the closure inside a single `rusqlite::Transaction`. Every query builder and generated helper exposes both a "managed" method (`execute`, `fetch_one`, ...) that takes `&DatabasePool` and opens its own pooled connection/transaction, and an `_in` counterpart for composing multiple statements: the read side (`fetch_in`, `fetch_one_in`, `count_in`) takes a `&rusqlite::Connection`, while the write side (`execute_in`, and the generated `update_by_id_in`/`delete_by_id_in`) takes a `&rusqlite::Transaction`, since `Transaction` derefs to `Connection` but not the other way around.
- **Cached prepared statements** — `SELECT` statements are prepared via `Connection::prepare_cached`, so repeated queries with the same shape reuse the cached statement.
- **SQL-file schema migrations** — the `dlls!("path")` macro embeds every `<version>_<description>.sql` file found in a directory (relative to the crate manifest) into a static array of `DdlVersion`s; `DatabasePool::create_schema(&DDLS)` applies them in order, tracked via SQLite's `PRAGMA user_version`, and runs `VACUUM` afterwards if anything was applied.
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

- an `entity::columns` module with a typed constant per persisted field, named after the **field** (not the `#[column(...)]` override) in upper case — `entity::columns::ID`, `entity::columns::EMAIL` (the field is `email`, even though its SQL column is `email_address`), `entity::columns::NAME` — plus `entity::TABLE` and `entity::SCHEMA`,
- an implementation of the `Entity` trait for `User` (`SCHEMA`, `TABLE_NAME`, `FIELDS`, `INSERT_FIELDS`, `AUTOINCREMENT_FIELD`, `map_from_row`, `get_insert_values`),
- `user.update_by_id(&db)` / `user.delete_by_id(&db)` **instance methods** on `User` (because the struct has a `#[primary_key(id)]` attribute), each with an `_in` counterpart taking a `&rusqlite::Transaction`,
- a `UserRepository` unit struct implementing `rusqlite_orm::dao::Repository<User>`, with `UserRepository::exists(&db, id)` / `UserRepository::select_by_id(&db, id)` (plus their `_in` variants) and `UserRepository::select_by_email(&db, email, order_by)` (because of the `#[index("email", (email))]` attribute, plus its `_in_conn` variant),
- `PartialEq` / `Eq` and `Hash` implementations based on `id` (because `comparable` and `hashable` are set to `true`).

`#[unique(...)]` works exactly like `#[index(...)]` but marks the index as unique, generating a `select_by_name` that returns `Option<Self>` (and `exists_by_name`) instead. See [`macros/README.md`](./macros/README.md#indexes-and-unique-indexes) for the full syntax.

`#[transient]` fields are skipped when building `INSERT`/`SELECT` column lists and are restored to their `Default` value when a row is mapped back into the struct.

`#[default]` skips a field in the `INSERT` column list only (it's still part of `SELECT`/`FIELDS`), for columns that have a SQL-level `DEFAULT` you want SQLite to apply rather than sending a value from Rust. `#[autoincrement]` implies `#[default]` and, additionally, marks the field as the one that receives the SQLite-assigned `rowid` after each insert — it's only allowed on one `i64` field per struct (a compile error otherwise). Because an insert can write the generated id back into the struct, `InsertBuilder::item(...)` takes `&mut T` rather than an owned `T`.

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

There is no process-wide singleton: you build a `DatabasePool` yourself with `DatabaseConnectionBuilder` and keep it around (e.g. as Tauri/app state) to pass to every call.

```rust
use std::time::Duration;
use rusqlite_orm::database::{
    DatabasePool,
    builder::{DatabaseConnectionBuilder, JournalMode},
};

let db: DatabasePool = DatabaseConnectionBuilder::default()
    .location("app.db")
    .pool_size(5)
    .min_idle(5)
    .connection_timeout(Duration::from_secs(5))
    .busy_timeout(Duration::from_secs(5))
    .enable_foreign_keys()
    .journal_mode(JournalMode::Delete)
    .build("app")?;

db.create_schema(&DDLS)?;
```

`DatabaseConnectionBuilder::default()` starts out configured for an **in-memory** database (a single connection, no min-idle, `PRAGMA journal_mode = MEMORY`) — calling `.location(path)` is a one-way transition to a **file-backed** builder with its own defaults (pool of 5, 5 min-idle, 10s busy timeout, `PRAGMA journal_mode = DELETE`), which is also where `.pool_size(...)`, `.min_idle(...)`, `.busy_timeout(...)` and `.journal_mode(JournalMode)` become available (an in-memory SQLite connection is a private, empty database per connection, so pooling more than one doesn't make sense there — the in-memory state intentionally has no pool-size knob). `.enable_foreign_keys()` and `.connection_timeout(...)` work in either state. `.build(name)` opens the pool and returns the `DatabasePool`; `name` is only a label used in log messages, not a registry key, so nothing stops you from building several independent `DatabasePool`s (e.g. one per attached schema). `create_schema` reads the current `PRAGMA user_version`, applies every migration whose version is higher inside a single transaction, updates `user_version`, and — if at least one migration was applied — runs `VACUUM` to reclaim space.

### 4. CRUD operations

All query builders (`select`, `insert`, `update`, `delete`) and the generated lookup helpers live on the `<Struct>Repository` type generated by `#[derive(Entity)]` — in this case, `UserRepository`. Most of them come in a **managed** form (takes `&DatabasePool`, opens its own pooled connection or transaction) and an **`_in`** form (takes an explicit `&rusqlite::Connection`/`&rusqlite::Transaction`, for composing several statements together). `SelectBuilder` is the asymmetric case: it has no managed multi-row `fetch()`, only the managed `fetch_one()`/`count()` (both taking `&DatabasePool`, returning `Option<T>`/`i64`) — fetching more than one row needs `db.run_in_connection(...)` plus the builder's `fetch_in(conn)`.

```rust
use rusqlite_orm::{
    dao::Repository,
    types::{order_by::OrderBy, where_clause::Where},
};

// INSERT (managed: takes `&DatabasePool`)
let mut user = User { id: 0, email: "alice@example.com".into(), name: "Alice".into(), transient_flag: false };
UserRepository::insert().item(&mut user).or_ignore().execute(&db)?;

// SELECT with WHERE / ORDER BY / LIMIT / OFFSET — needs an explicit connection
let users = db.run_in_connection(|conn| {
    Ok(UserRepository::select()
        .where_(Where::Eq(entity::columns::EMAIL, "alice@example.com".into()))
        .order_by(OrderBy::Asc(entity::columns::ID))
        .limit(10)
        .offset(20)
        .fetch_in(conn)?)
})?;

// Generated helpers (on the repository) — each has a managed form taking `&DatabasePool`
let by_id = UserRepository::select_by_id(&db, 1)?;
let by_email = UserRepository::select_by_email(&db, "alice@example.com", None)?;
let found = UserRepository::exists(&db, 1)?;

// UPDATE
user.update_by_id(&db)?; // instance method: updates all non-id columns by id

// Or a manual UPDATE builder
UserRepository::update()
    .set(entity::columns::NAME, "Alicia".into())
    .where_(Where::Eq(entity::columns::ID, 1.into()))
    .execute(&db)?;

// DELETE
user.delete_by_id(&db)?;
```

`InsertBuilder::or_ignore()` / `or_replace()` take no arguments — call the one you want to turn `INSERT` into `INSERT OR IGNORE` / `INSERT OR REPLACE`; calling neither keeps a plain `INSERT`.

### 5. Composing statements in a single transaction

Every builder and repository helper exposes a connection-taking variant (`_in` on the builders and primary-key helpers, `_in_conn` on index/unique/relationship helpers — see the note above) so several statements can share one connection or transaction:

```rust
use rusqlite_orm::database::DatabasePool;

let fetched = db.run_in_transaction(|tx| {
    UserRepository::insert().item(&mut user).execute_in(tx)?;
    Ok(UserRepository::select_by_id_in(tx, 1)?)
})?;
```

`run_in_transaction`'s closure receives a `&mut rusqlite::Transaction`, which reborrows as the `&rusqlite::Transaction`/`&rusqlite::Connection` the various `_in` methods expect (`Transaction` derefs to `Connection`), so both read (`fetch_in`, `select_by_id_in`, ...) and write (`execute_in`, `update_by_id_in`, `delete_by_id_in`, ...) calls can be composed here. `run_in_connection`'s closure only gets a plain pooled connection (no transaction), so it can compose reads but not the write-side `_in` methods, which specifically require a `&rusqlite::Transaction`.

The closure passed to `run_in_transaction` (and `run_in_connection`) returns `std::result::Result<R, Box<dyn std::error::Error + Send + Sync>>` rather than the crate's own `Result<R>`, so it can propagate any error type with `?` (not just `DatabaseError`) — useful when composing `_in` calls with your own fallible logic inside the same connection/transaction. Any error returned from the closure is wrapped into `DatabaseError::Transaction` (for `run_in_transaction`) or `DatabaseError::RunningOnConnection` (for `run_in_connection`).

## `WHERE` clause reference

`Where<T>` (in `rusqlite_orm::types::where_clause`) supports:

| Variant                         | SQL                                       |
| ------------------------------- | ----------------------------------------- |
| `Where::Eq(col, val)`           | `col = ?`                                 |
| `Where::NotEq(col, val)`        | `col != ?`                                |
| `Where::Gt(col, val)`           | `col > ?`                                 |
| `Where::Gte(col, val)`          | `col >= ?`                                |
| `Where::Lt(col, val)`           | `col < ?`                                 |
| `Where::Lte(col, val)`          | `col <= ?`                                |
| `Where::In(col, vals)`          | `col IN (?, ?, ...)`                      |
| `Where::InMultiple(cols, rows)` | `(col_a, col_b) IN ((?, ?), (?, ?), ...)` |
| `Where::Null(col)`              | `col IS NULL`                             |
| `Where::NotNull(col)`           | `col IS NOT NULL`                         |
| `Where::EqSub(col, sub)`        | `col = (<subquery>)`                      |
| `Where::NotEqSub(col, sub)`     | `col != (<subquery>)`                     |
| `Where::InSub(col, sub)`        | `col IN (<subquery>)`                     |
| `Where::NotInSub(col, sub)`     | `col NOT IN (<subquery>)`                 |
| `Where::InMultipleSub(cols, sub)` | `(col_a, col_b) IN (<subquery>)`        |
| `Where::And(conditions)`        | `(...) AND (...)`                         |
| `Where::Or(conditions)`         | `(...) OR (...)`                          |

A `sub` is a `rusqlite_orm::types::subquery::Subquery`, obtained by calling `.to_subquery()` on a `SelectBuilder` (built with `.columns(&[single_col])` so it renders as a single-column `SELECT`), or by calling `Subquery::raw(sql)` for a fragment with no bound parameters; its SQL and bound parameters are spliced verbatim into the outer statement, so it composes with any `where_`/`order_by`/`limit`/`offset` you added to the inner builder.

In `Eq`/`NotEq`/`Gt`/`Gte`/`Lt`/`Lte`/`In`, `val`/`vals` normally bind as `?` parameters, but a `Value::Raw(sql)` is spliced into the statement text instead and skipped when collecting bound parameters — useful for comparing a column against a SQLite function call:

```rust
use rusqlite_orm::types::value::Value;

// expires_at < CURRENT_TIMESTAMP   (no bound parameter for the right-hand side)
Where::Lt(entity::columns::EXPIRES_AT, Value::Raw("CURRENT_TIMESTAMP".into()));
```

Since the string is spliced verbatim and unescaped, only build `Value::Raw` from trusted/static SQL — never from unsanitized user input.

## Error handling

All fallible operations return `rusqlite_orm::errors::Result<T>`, an alias for `Result<T, DatabaseError>`. `DatabaseError` (via `thiserror`) distinguishes: `Connection`/`Pool` (opening or borrowing from the `r2d2` pool), `SchemaCreation`, `Insert`, `Update`, `Select` and `Delete` (wrapping the underlying `rusqlite::Error`), and `Transaction`/`RunningOnConnection`, which wrap a `Box<dyn std::error::Error + Send + Sync>` returned from a `run_in_transaction`/`run_in_connection` closure (see [Composing statements in a single transaction](#5-composing-statements-in-a-single-transaction) above). The enum also has `ClosedConnection`, `AlreadyInitialized` and `Savepoint` variants that exist for forward-compatibility but aren't currently returned by anything in the crate.

## Crate details

- **[`orm/`](./orm)** — see [`orm/README.md`](./orm/README.md) for crate-specific documentation.
- **[`macros/`](./macros)** — the proc-macro crate. It has no runtime dependencies beyond `syn`, `quote`, and `proc-macro2`, and is intended to be used together with `rusqlite_orm`, not standalone. See [`macros/README.md`](./macros/README.md) for the full list of supported attributes, including `#[relationship(...)]`.

## License

Both crates are distributed under the [MIT license](https://opensource.org/licenses/MIT).
