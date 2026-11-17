# rusqlite_orm

A lightweight, compile-time-checked ORM layer for [`rusqlite`](https://crates.io/crates/rusqlite), built around a `#[derive(Entity)]` procedural macro. It generates table metadata, row mapping, and typed query helpers (select / insert / update / delete) for your structs, gives you a configurable pooled SQLite connection, and ships a small SQL-file-based schema migration system.

This repository is a Cargo workspace made up of two crates:

| Crate                             | Path      | Description                                                                                                                        |
| --------------------------------- | --------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| [`rusqlite_orm`](./orm)           | `orm/`    | The runtime ORM: `Entity` trait, `Repository` trait, query builders, `Where`/`OrderBy` types, pooled connections and schema migrations. |
| [`rusqlite_orm_macros`](./macros) | `macros/` | The `#[derive(Entity)]` procedural macro and the `ddls!` macro used to embed SQL migration files at compile time.                  |

> Both crates are versioned and published together. `rusqlite_orm` re-exports `rusqlite` itself, so you don't need to depend on `rusqlite` directly, and — with its opt-in `derive` feature — it also re-exports the `Entity` derive and the `ddls!` macro, so you don't need to depend on `rusqlite_orm_macros` directly either.

## Features

- **Derive-based entities** — annotate a struct with `#[derive(Entity)]` and get table metadata, row-to-struct mapping, and column constants for free.
- **A generated `Repository`** — every `#[derive(Entity)]` struct gets a companion `<Struct>Repository` unit struct implementing `rusqlite_orm::dao::Repository<Struct>`, which is where the query builders and generated lookups (`select_by_id`, `exists`, index helpers, ...) live.
- **Typed query builders** — `select()`, `insert()`, `update()`, `delete()` builders with a fluent API, called on the generated `<Struct>Repository`.
- **Rich `WHERE` clauses** — `Eq`, `NotEq`, `Gt`, `Gte`, `Lt`, `Lte`, `In`, `InMultiple` (tuple `IN`), `Null`, `NotNull`, combinable with `And` / `Or`, plus subquery variants (`EqSub`, `NotEqSub`, `InSub`, `NotInSub`, `InMultipleSub`) that embed another `SelectBuilder` (via `.to_subquery()`) inside the condition. Any comparison value can also be `Value::Raw(sql)` to splice a SQL expression (e.g. a SQLite function call like `CURRENT_TIMESTAMP`) in place of a bound parameter.
- **Ordering, limits & pagination** — `OrderBy::Asc` / `OrderBy::Desc`, `.limit(n)` and `.offset(n)`. `SelectBuilder` is `Clone` (when its kind is); `fetch`/`fetch_in` return every matching row, while `fetch_one`/`fetch_one_in` run the query with `LIMIT 1` (the builder itself is left untouched) instead of fetching every matching row and discarding all but the first.
- **Raw / non-mapped selects** — calling `.columns(&[...])` or `.distinct(&[...])` on `select()` switches it from returning `Vec<Entity>` to returning `Row`/`Rows` (a simple column-name → `Value` map), for projections that don't need to cover every persisted column. `Row` has typed getters (`get_int`, `get_u16`, `get_f64`, `get_bool`, `get_text`, `get_blob`, ... each with a `_nullable` variant), see [Reading non-mapped rows](#reading-non-mapped-rows).
- **Aggregate functions & grouping** — `.columns(...)` also takes `Expression`s: `COUNT`, `MAX`, `MIN`, `SUM` and `AVG` over a column (`entity::columns::DATE.max()`, ...), `Expression::count_all()` for `COUNT(*)`, each optionally `.alias("name")`d. `.group_by(&[...])` adds a `GROUP BY` clause, and `count()` on a grouped select counts its groups. Grouped selects can be embedded as subqueries too, see [Aggregates and GROUP BY](#aggregates-and-group-by).
- **Generated convenience methods** for entities with a struct-level `#[primary_key(field_a, field_b, ...)]` attribute:
  - on the **repository**: `exists`, `select_by_id` (and `_in` variants);
  - on the **entity instance** itself: `update_by_id`, `delete_by_id` (and `_in` variants).
- **Generated index lookups** — declare `#[index("name", (col_a, col_b))]` on the struct (repeatable) to get, on the repository, `select_by_name(...)` plus its `count_by_name` and `_in` counterparts (every connection-taking variant — builders, primary-key, index/unique and relationship helpers — uses the same `_in` suffix).
- **Generated unique-index lookups** — `#[unique("name", (col_d, col_e))]` uses the same syntax as `#[index(...)]`, but the generated `select_by_name` returns `Option<Self>` (at most one row) instead of `Vec<Self>`, has no `order_by` parameter, and its count counterpart is `exists_by_name` returning `bool`.
- **Relationships between entities** — annotate an `Option<T>` or `Vec<T>` field with `#[relationship((local_field, remote_column), ...)]` to get `fetch_<field>_relationship` / `fetch_<field>_relationship_in` instance methods that lazily load the related row(s).
- **Optional derived `PartialEq` / `Eq` / `Hash`** based on the entity's id column(s), via `comparable` / `hashable` attribute flags.
- **Fields excluded from the schema** with `#[transient]`, populated via `Default::default()` when mapping rows back (requires the struct to implement `Default`). Relationship fields are excluded automatically the same way.
- **Columns excluded from `INSERT` only** with `#[default]` — the field stays in `SELECT`/`FIELDS`, but is left out of the generated `INSERT` statement so SQLite applies its own column default.
- **Autoincrement primary keys** — `#[autoincrement]` (implies `#[default]`) on a single `i64` field gets the generated `rowid` written back into that field after each insert.
- **Multiple SQLite schemas** — `#[entity(schema = "...")]` attaches an entity to a schema other than `"main"` (e.g. an `ATTACH`ed database); every generated statement is qualified as `<schema>.<table>`.
- **Wide column type support** — every signed/unsigned integer width, `f32`/`f64`, `bool`, `String`, `Vec<u8>` (BLOB) and `Option<T>` map onto `rusqlite_orm::types::value::Value` out of the box, plus a `Value::Raw(String)` variant for SQL literals/function calls that must not be bound as a parameter.
- **Configurable pooled connections** — `DatabaseConnectionBuilder` is a typestate builder: it starts in an in-memory state (single connection, `PRAGMA journal_mode = MEMORY`), and calling `.location(path)` switches it to a file-backed state with its own defaults (pool of 5, `PRAGMA journal_mode = DELETE`). Pool size, min idle connections, connection/busy timeouts, journal mode and `PRAGMA foreign_keys` are all configurable before calling `.build(name)`, which opens an [`r2d2`](https://crates.io/crates/r2d2)-backed pool (via `r2d2_sqlite`) and hands you back an owned `DatabasePool` — there is no global singleton, so you're free to build more than one.
- **Async API, any runtime** — every method that takes a `&DatabasePool` is `async`. rusqlite itself is synchronous, so each call runs on the [`blocking`](https://crates.io/crates/blocking) thread pool, which works under any executor (tokio, async-std, smol, or a plain `block_on`) and never stalls an async worker thread. The connection-taking `_in` methods are `async` too: they are what you await inside the async closures passed to `run_in_connection`/`run_in_transaction`, which are driven to completion on that same blocking thread, so a connection or transaction never leaves it and the closure's future doesn't need to be `Send`. Inside those closures only await the ORM's `_in` methods (or other runtime-agnostic futures): there is no runtime reactor on that thread, so e.g. tokio I/O or timers panic. The futures returned by the managed (pool-taking) methods are `Send`, and entities must be `Send + Sync`.
- **Two ways to run statements** — `DatabasePool::run_in_connection(...)` borrows a pooled connection for one or more statements (not atomic across calls unless you wrap them yourself), and `DatabasePool::run_in_transaction(...)` runs the closure inside a single `rusqlite::Transaction`. Every query builder and generated helper exposes both a "managed" method (`execute`, `fetch_one`, ...) that takes `&DatabasePool` and opens its own pooled connection/transaction, and an `_in` counterpart for composing multiple statements: the read side (`fetch_in`, `fetch_one_in`, `count_in`) takes a `&rusqlite::Connection`, and so does the write side (`execute_in`, and the generated `update_by_id_in`/`delete_by_id_in`); a `Transaction` can be passed anywhere, since it derefs to `Connection`. The managed write methods only open a transaction when it buys atomicity: `update`/`delete` `execute`, `update_by_id` and `delete_by_id` are a single statement and run on a plain pooled connection, while `InsertBuilder::execute` uses a transaction only for more than one item (which may take several statements).
- **Cached prepared statements** — every statement the builders run (`SELECT`, `COUNT`, `INSERT`, `UPDATE`, `DELETE`) is prepared via `Connection::prepare_cached`, so repeated queries with the same shape reuse the cached statement; an autoincrement multi-item `INSERT` prepares its statement once and reuses it for every item. Statement logging (parameters interpolated into the SQL) is only built when `debug` logging is enabled.
- **SQL-file schema migrations** — the `ddls!("path")` macro embeds every `<version>_<description>.sql` file found in a directory (relative to the crate manifest) as an array expression of `DdlVersion`s; `DatabasePool::create_schema(&ddls)` applies them in order, tracked via SQLite's `PRAGMA user_version`, and runs `VACUUM` afterwards if anything was applied. Each `DdlVersion` also has an optional `update_fn` hook, run inside the same transaction right after that migration's SQL, for data migrations that plain SQL can't express.
- **Serializable errors** — `DatabaseError` implements `serde::Serialize` (as its `Display` string), so it can be returned directly from, e.g., a Tauri command.
- **Optional `derive` feature** — `rusqlite_orm = { version = "3", features = ["derive"] }` re-exports `Entity` and `ddls!` from `rusqlite_orm_macros` at the crate root (`use rusqlite_orm::Entity;`, `rusqlite_orm::ddls!(...)`).
- **Bundled SQLite by default** — the default `bundled` feature compiles SQLite from source into your binary. Set `default-features = false` to link against the system `libsqlite3` instead (smaller binary, but a runtime dependency on the system library): `rusqlite_orm = { version = "3", default-features = false, features = ["derive"] }`.
- **Query logging** — every generated statement is logged (via the `log` crate) with parameters interpolated, plus the number of affected/fetched rows.

## Installation

Enable the `derive` feature, which re-exports the macros from `rusqlite_orm_macros` so you only need one dependency:

```toml
[dependencies]
rusqlite_orm = { version = "3", features = ["derive"] }
```

Or, without the feature, depend on both crates explicitly (and import the macros from `rusqlite_orm_macros` instead of `rusqlite_orm`):

```toml
[dependencies]
rusqlite_orm = "3"
rusqlite_orm_macros = "3"
```

`rusqlite_orm` re-exports `rusqlite`, accessible as `rusqlite_orm::rusqlite`, so most consumers won't need to add `rusqlite` as a separate dependency.

## Quick start

### 1. Define an entity

```rust
use rusqlite_orm::Entity; // with the `derive` feature; otherwise `use rusqlite_orm_macros::Entity;`

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
- `user.update_by_id(&db)` / `user.delete_by_id(&db)` **instance methods** on `User` (because the struct has a `#[primary_key(id)]` attribute), each with an `_in` counterpart taking a `&rusqlite::Connection` (or a `Transaction`, through `Deref`),
- a `UserRepository` unit struct implementing `rusqlite_orm::dao::Repository<User>`, with `UserRepository::exists(&db, id)` / `UserRepository::select_by_id(&db, id)` (plus their `_in` variants) and `UserRepository::select_by_email(&db, email, order_by)` (because of the `#[index("email", (email))]` attribute, plus its `_in` variant),
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

Embed them at compile time with the `ddls!` macro, which expands to an **array expression** of `rusqlite_orm::database::DdlVersion` (one entry per file), so you bind it yourself:

```rust
let ddls = rusqlite_orm::ddls!("migrations"); // `rusqlite_orm_macros::ddls!` without the `derive` feature
// expands to: [DdlVersion { version, description, sql, update_fn: None }, ...]
```

To keep it around as a global, give it an explicit array type, e.g. `static DDLS: [DdlVersion; 3] = rusqlite_orm::ddls!("migrations");`.

Every entry starts with `update_fn: None`. `DdlVersion::update_fn` is an `Option<rusqlite_orm::database::UpdateFn>`, an async hook taking `&mut rusqlite::Transaction` (write it as an `async fn` and box its future); set it on any migration that needs Rust code (e.g. recomputing a derived column) to have it run in the same transaction, right after that migration's SQL:

```rust
let mut ddls = rusqlite_orm::ddls!("migrations");
for ddl in &mut ddls {
    if ddl.version == 5 {
        ddl.update_fn = Some(|tx| Box::pin(recalculate_derived_columns(tx)));
    }
}
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

db.create_schema(&ddls).await?;
```

`DatabaseConnectionBuilder::default()` starts out configured for an **in-memory** database (a single connection, no min-idle, `PRAGMA journal_mode = MEMORY`) — calling `.location(path)` is a one-way transition to a **file-backed** builder with its own defaults (pool of 5, 5 min-idle, 10s busy timeout, `PRAGMA journal_mode = DELETE`), which is also where `.pool_size(...)`, `.min_idle(...)`, `.busy_timeout(...)` and `.journal_mode(JournalMode)` become available (an in-memory SQLite connection is a private, empty database per connection, so pooling more than one doesn't make sense there — the in-memory state intentionally has no pool-size knob). `.enable_foreign_keys()` and `.connection_timeout(...)` work in either state. `.build(name)` opens the pool and returns the `DatabasePool`; `name` is only a label used in log messages, not a registry key, so nothing stops you from building several independent `DatabasePool`s (e.g. one per attached schema). `create_schema` reads the current `PRAGMA user_version`, applies every migration whose version is higher inside a single transaction (running each one's `update_fn`, if set, right after its SQL), updates `user_version`, and — if at least one migration was applied — runs `VACUUM` to reclaim space. A failure in a migration's SQL or `update_fn` rolls back the whole transaction; since it happens inside `run_in_transaction`, it surfaces as a `DatabaseError::Transaction` wrapping a `DatabaseError::SchemaCreation` (a failing final `VACUUM` is returned as a plain `SchemaCreation`).

### 4. CRUD operations

All query builders (`select`, `insert`, `update`, `delete`) and the generated lookup helpers live on the `<Struct>Repository` type generated by `#[derive(Entity)]` — in this case, `UserRepository`. Every one of them comes in a **managed** form (takes `&DatabasePool`, opens its own pooled connection — or transaction, for multi-item inserts) and an **`_in`** form (takes an explicit `&rusqlite::Connection`, which may be a `Transaction`, for composing several statements together). On `SelectBuilder` these are `fetch`/`fetch_in` (`Vec<T>`), `fetch_one`/`fetch_one_in` (`Option<T>`) and `count`/`count_in` (`i64`).

```rust
use rusqlite_orm::{
    dao::Repository,
    types::{order_by::OrderBy, where_clause::Where},
};

// INSERT (managed: takes `&DatabasePool`)
let mut user = User { id: 0, email: "alice@example.com".into(), name: "Alice".into(), transient_flag: false };
UserRepository::insert().item(&mut user).or_ignore().execute(&db).await?;

// SELECT with WHERE / ORDER BY / LIMIT / OFFSET
let users = UserRepository::select()
    .where_(Where::Eq(entity::columns::EMAIL, "alice@example.com".into()))
    .order_by(OrderBy::Asc(entity::columns::ID))
    .limit(10)
    .offset(20)
    .fetch(&db).await?;

// Generated helpers (on the repository) — each has a managed form taking `&DatabasePool`
let by_id = UserRepository::select_by_id(&db, 1).await?;
let by_email = UserRepository::select_by_email(&db, "alice@example.com", None).await?;
let found = UserRepository::exists(&db, 1).await?;

// UPDATE
user.update_by_id(&db).await?; // instance method: updates all non-id columns by id

// Or a manual UPDATE builder
UserRepository::update()
    .set(entity::columns::NAME, "Alicia".into())
    .where_(Where::Eq(entity::columns::ID, 1.into()))
    .execute(&db).await?;

// DELETE
user.delete_by_id(&db).await?;
```

`InsertBuilder::or_ignore()` / `or_replace()` take no arguments — call the one you want to turn `INSERT` into `INSERT OR IGNORE` / `INSERT OR REPLACE`; calling neither keeps a plain `INSERT`. Inserting several items at once uses multi-row `INSERT` statements, split automatically into batches that respect the connection's `SQLITE_LIMIT_VARIABLE_NUMBER` (so any number of items works). The managed `execute` runs several items inside a transaction, so a failure in any batch rolls back the whole insert; `execute_in` is only atomic when the connection you pass is a transaction. Entities with an `#[autoincrement]` field are inserted one row at a time (to read back each `rowid`), reusing a single prepared statement.

#### Reading non-mapped rows

A `.columns(...)`/`.distinct(...)` select returns `Row`s. Besides the raw `row.get(column) -> Option<&Value>`, `Row` has typed getters that convert from the values SQLite returns (`Int64`, `Float64`, `Text`, `Blob`):

| Getter | Returns | Accepts |
| ------ | ------- | ------- |
| `get_int` | `i64` | integer |
| `get_i8`, `get_i16`, `get_i32`, `get_isize`, `get_u8`, `get_u16`, `get_u32`, `get_u64`, `get_usize` | that type | integer within the type's range |
| `get_f64`, `get_f32` | `f64` / `f32` | real or integer |
| `get_bool` | `bool` | integer (anything but `0` is `true`) |
| `get_text` | `String` | text |
| `get_blob` | `Vec<u8>` | blob |

Each getter panics if the column is missing, NULL, of another type or out of range; its `_nullable` counterpart (`get_u16_nullable`, `get_text_nullable`, ...) returns `None` for a missing or NULL column instead, still panicking on another type.

```rust
let names: Vec<(u32, String)> = UserRepository::select()
    .columns(&[entity::columns::ID, entity::columns::NAME])
    .fetch(&db).await?
    .iter()
    .map(|row| {
        (
            row.get_u32(entity::columns::ID.as_ref()),
            row.get_text(entity::columns::NAME.as_ref()),
        )
    })
    .collect();
```

#### Aggregates and GROUP BY

`.columns(...)` and `.distinct(...)` take columns or `Expression`s, so plain columns and aggregate functions can be mixed (convert columns with `.into()` when they share a slice with functions). `ColumnName` has `.count()`, `.max()`, `.min()`, `.sum()` and `.avg()`, and `Expression::count_all()` renders `COUNT(*)`. In the returned rows a column is read by its name, an aliased expression (`.alias("name")`) by its alias, and an unaliased function by its SQL text, e.g. `"MAX(date)"`.

`.group_by(&[...])` groups the rows by the given columns, after the `WHERE` clause. `count()`/`count_in()` on a grouped select count its groups (ordering, limit and offset are ignored, as for any count).

```rust
use rusqlite_orm::types::expression::Expression;

// Number of orders and latest order date of each customer
let per_customer: Vec<(i64, i64, i64)> = OrderRepository::select()
    .columns(&[
        entity::columns::CUSTOMER.into(),
        entity::columns::DATE.max().alias("latest"),
        Expression::count_all().alias("orders"),
    ])
    .group_by(&[entity::columns::CUSTOMER])
    .fetch(&db).await?
    .iter()
    .map(|row| (row.get_int("customer"), row.get_int("latest"), row.get_int("orders")))
    .collect();

// As a subquery: the latest order of each customer
let latest = OrderRepository::select()
    .columns(&[entity::columns::CUSTOMER.into(), entity::columns::DATE.max()])
    .group_by(&[entity::columns::CUSTOMER])
    .to_subquery();
let orders = OrderRepository::select()
    .where_(Where::InMultipleSub(
        vec![entity::columns::CUSTOMER, entity::columns::DATE],
        latest,
    ))
    .fetch(&db).await?;
```

### Async model

All the managed methods (those taking `&DatabasePool`) are `async fn`s, and `DatabasePool` is cheap to `Clone` (clones share the same pool). Internally each call moves its work to the [`blocking`](https://crates.io/crates/blocking) thread pool: builders are cloned (they only hold `Value`s), and `InsertBuilder` sends only the items' insert values, assigning the autoincrement ids it reads back to the borrowed items once it finishes. Nothing ties the crate to an async runtime.

The closures passed to `run_in_connection`/`run_in_transaction` are async closures (`async move |tx| { ... }`) and must be `Send + 'static`, since they run on another thread: move what they need into them and return what you need back, instead of borrowing locals. They are driven with `block_on` on that blocking thread, so inside them await only the `_in` methods (or other runtime-agnostic futures), never tokio I/O or timers.

### 5. Composing statements in a single transaction

Every builder and repository helper exposes a connection-taking, `async` `_in` variant so several statements can share one connection or transaction:

```rust
use rusqlite_orm::database::DatabasePool;

// The closure runs on another thread: it owns `user` and hands it back.
let (user, fetched) = db
    .run_in_transaction(async move |tx| {
        UserRepository::insert().item(&mut user).execute_in(tx).await?;
        let fetched = UserRepository::select_by_id_in(tx, user.id).await?;
        Ok((user, fetched))
    })
    .await?;
```

`run_in_transaction`'s closure receives a `&mut rusqlite::Transaction`, which reborrows as the `&rusqlite::Transaction`/`&rusqlite::Connection` the various `_in` methods expect (`Transaction` derefs to `Connection`), so both read (`fetch_in`, `select_by_id_in`, ...) and write (`execute_in`, `update_by_id_in`, `delete_by_id_in`, ...) calls can be composed here, all committed or rolled back together. `run_in_connection`'s closure gets a plain pooled connection (no transaction), which every `_in` method accepts as well, reads and writes alike: use it when the statements don't need to be atomic together, and `run_in_transaction` when they do.

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

All fallible operations return `rusqlite_orm::errors::Result<T>`, an alias for `Result<T, DatabaseError>`. `DatabaseError` (via `thiserror`) distinguishes: `Connection`/`Pool` (opening or borrowing from the `r2d2` pool), `Insert`, `Update`, `Select` and `Delete` (wrapping the underlying `rusqlite::Error`), and `SchemaCreation`/`Transaction`/`RunningOnConnection`, which wrap a `Box<dyn std::error::Error + Send + Sync>` — `SchemaCreation` because a migration's `update_fn` can fail with any `DatabaseError`, and `Transaction`/`RunningOnConnection` because they carry whatever a `run_in_transaction`/`run_in_connection` closure returned (see [Composing statements in a single transaction](#5-composing-statements-in-a-single-transaction) above). `DatabaseError` also implements `serde::Serialize`, serializing to its `Display` message as a plain string. The enum also has `ClosedConnection`, `AlreadyInitialized` and `Savepoint` variants that exist for forward-compatibility but aren't currently returned by anything in the crate.

## Crate details

- **[`orm/`](./orm)** — see [`orm/README.md`](./orm/README.md) for crate-specific documentation.
- **[`macros/`](./macros)** — the proc-macro crate. It has no runtime dependencies beyond `syn`, `quote`, and `proc-macro2`, and is intended to be used together with `rusqlite_orm`, not standalone. See [`macros/README.md`](./macros/README.md) for the full list of supported attributes, including `#[relationship(...)]`.

## License

Both crates are distributed under the [MIT license](https://opensource.org/licenses/MIT).
