# rusqlite_orm

A lightweight, compile-time-checked ORM layer for [`rusqlite`](https://crates.io/crates/rusqlite), built around a `#[derive(Entity)]` procedural macro. It generates table metadata, row mapping, and typed query helpers (select / insert / update / delete) for your structs, plus a small SQL-file-based schema migration system.

This repository is a Cargo workspace made up of two crates:

| Crate                             | Path      | Description                                                                                                                                            |
| --------------------------------- | --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| [`rusqlite_orm`](./orm)           | `orm/`    | The runtime ORM: `Entity` trait, `Repository` trait, query builders, `Where`/`OrderBy` types, connection/transaction management and schema migrations. |
| [`rusqlite_orm_macros`](./macros) | `macros/` | The `#[derive(Entity)]` procedural macro and the `dlls!` macro used to embed SQL migration files at compile time.                                      |

> Both crates are versioned and published together and are intended to be used as a pair — `rusqlite_orm` re-exports `rusqlite` itself, so you don't need to depend on `rusqlite` directly.

## Features

- **Derive-based entities** — annotate a struct with `#[derive(Entity)]` and get table metadata, row-to-struct mapping, and column constants for free.
- **A generated `Repository`** — every `#[derive(Entity)]` struct gets a companion `<Struct>Repository` unit struct implementing `rusqlite_orm::dao::Repository<Struct>`, which is where the query builders and generated lookups (`select_by_id`, `exists`, index helpers, ...) live.
- **Typed query builders** — `select()`, `insert()`, `update()`, `delete()` builders with a fluent API, called on the generated `<Struct>Repository`.
- **Rich `WHERE` clauses** — `Eq`, `NotEq`, `Gt`, `Lt`, `In`, `InMultiple` (tuple `IN`), `Null`, `NotNull`, combinable with `And` / `Or`.
- **Ordering, limits & pagination** — `OrderBy::Asc` / `OrderBy::Desc`, `.limit(n)` and `.offset(n)`.
- **Generated convenience methods** for entities with `#[primary_key]` field(s):
  - on the **repository**: `exists`, `select_by_id` (and `_in_tx` variants);
  - on the **entity instance** itself: `update_by_id`, `delete_by_id` (and `_in_tx` variants).
- **Generated index lookups** — declare `#[indexes((col_a, col_b), (col_c))]` on the struct to get, on the repository, `select_by_col_a_and_col_b(...)` / `select_by_col_c(...)` plus their `count_by_*` and `_in_tx` counterparts.
- **Optional derived `PartialEq` / `Eq` / `Hash`** based on the entity's id column(s), via `comparable` / `hashable` attribute flags.
- **Fields excluded from the schema** with `#[dont_map]`, populated via `Default::default()` when mapping rows back (requires the struct to implement `Default`).
- **Transaction support** — every query builder exposes both a "managed" method (`fetch`, `execute`, ...) that opens its own transaction against a global connection, and an `_in_tx` counterpart for composing multiple statements atomically.
- **SQL-file schema migrations** — the `dlls!("path")` macro embeds every `<version>_<description>.sql` file found in a directory (relative to the crate manifest) into a static array of `DdlVersion`s, applied in order and tracked via SQLite's `PRAGMA user_version`.
- **Query logging** — every generated statement is logged (via the `log` crate) with parameters interpolated, plus the number of affected/fetched rows.

## Installation

Add both crates to your `Cargo.toml`:

```toml
[dependencies]
rusqlite_orm = "0.1"
rusqlite_orm_macros = "0.1"
```

`rusqlite_orm` re-exports `rusqlite`, accessible as `rusqlite_orm::rusqlite`, so most consumers won't need to add `rusqlite` as a separate dependency.

## Quick start

### 1. Define an entity

```rust
use rusqlite_orm::dao::Entity;
use rusqlite_orm_macros::Entity;

#[derive(Entity, Debug, Clone, Default)]
#[entity(table = "users", comparable = true, hashable = true)]
#[indexes((email))]
pub struct User {
    #[primary_key]
    pub id: i64,
    #[column(name = "email_address")]
    pub email: String,
    pub name: String,
    #[dont_map]
    pub transient_flag: bool,
}
```

This expands into:

- an `entity::columns` module with a typed constant per persisted column (`entity::columns::ID`, `entity::columns::EMAIL_ADDRESS`, `entity::columns::NAME`), plus `entity::TABLE`,
- an implementation of the `Entity` trait for `User` (`TABLE_NAME`, `FIELDS`, `map_from_row`, `get_values`),
- `user.update_by_id()` / `user.delete_by_id()` **instance methods** on `User` (because the struct has a `#[primary_key]` field), each with an `_in_tx` counterpart,
- a `UserRepository` unit struct implementing `rusqlite_orm::dao::Repository<User>`, with:
  - `UserRepository::exists(id)` / `UserRepository::select_by_id(id)` (and `_in_tx` variants),
  - `UserRepository::select_by_email(email, order_by)` and `UserRepository::count_by_email(email)` (because of the `#[indexes((email))]` attribute), plus their `_in_tx` variants,
- `PartialEq` / `Eq` and `Hash` implementations for `User` based on `id` (because `comparable` and `hashable` are set to `true`).

`#[dont_map]` fields are skipped when building `INSERT`/`SELECT` column lists and are restored to their `Default` value when a row is mapped back into the struct.

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
use rusqlite_orm::database::DATABASE_INST;

DATABASE_INST.lock().unwrap().open("app.db")?;
DATABASE_INST.lock().unwrap().create_schema(&DDLS)?;
```

`create_schema` reads the current `PRAGMA user_version`, applies every migration whose version is higher, and updates the schema inside a single transaction.

### 4. CRUD operations

All query builders (`select`, `insert`, `update`, `delete`) and the generated lookup helpers live on the `<Struct>Repository` type generated by `#[derive(Entity)]` — in this case, `UserRepository`.

```rust
use rusqlite_orm::dao::{
    Repository,
    helpers::types::{order_by::OrderBy, where_clause::Where},
};

// INSERT
let user = User { id: 0, email: "alice@example.com".into(), name: "Alice".into(), transient_flag: false };
UserRepository::insert().item(user.clone()).or_ignore(false).execute()?;

// SELECT with WHERE / ORDER BY / LIMIT / OFFSET
let users = UserRepository::select()
    .where_(Where::Eq(entity::columns::EMAIL_ADDRESS, "alice@example.com".into()))
    .order_by(OrderBy::Asc(entity::columns::ID))
    .limit(10)
    .offset(20)
    .fetch()?;

// Generated helpers (on the repository)
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

// DELETE
user.delete_by_id()?; // instance method
```

### 5. Composing statements in a single transaction

Every builder and repository helper exposes an `_in_tx` variant so several statements can share one transaction:

```rust
DATABASE_INST.lock().unwrap().run_in_tx(|tx| {
    UserRepository::insert().item(user.clone()).execute_in_tx(tx)?;
    let fetched = UserRepository::select_by_id_in_tx(tx, 1)?;
    Ok(fetched)
})?;
```

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

All fallible operations return `rusqlite_orm::database::errors::Result<T>`, an alias for `Result<T, DatabaseError>`. `DatabaseError` (via `thiserror`) distinguishes connection, transaction, schema-creation, insert, update, select and delete failures, each wrapping the underlying `rusqlite::Error`.

## Crate details

- **[`orm/`](./orm)** — see [`orm/README.md`](./orm/README.md) for crate-specific documentation.
- **[`macros/`](./macros)** — the proc-macro crate. It has no runtime dependencies beyond `syn`, `quote`, and `proc-macro2`, and is intended to be used together with `rusqlite_orm`, not standalone. See [`macros/README.md`](./macros/README.md) for the full list of supported attributes.

## License

Both crates are distributed under the [MIT license](https://opensource.org/licenses/MIT).
