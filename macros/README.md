# rusqlite_orm_macros

Procedural macros for [`rusqlite_orm`](../orm). This crate is meant to be used together with `rusqlite_orm`, not on its own.

It provides two macros:

## `#[derive(Entity)]`

Generates the boilerplate needed to treat a struct as a database entity.

```rust
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

**Struct-level attributes**

| Attribute                             | Effect                                                                                                                                                     |
| ------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `#[entity("...")]`                    | Shorthand for `#[entity(table = "...")]`. Cannot be combined with `schema`/`comparable`/`hashable` — use the `table = "..."` form for that.                |
| `#[entity(table = "...")]`            | Overrides the SQL table name (defaults to the struct name, lowercased).                                                                                    |
| `#[entity(schema = "...")]`           | Overrides the SQL schema name (defaults to `"main"`). Useful when the entity lives in an `ATTACH`ed database; every generated statement is qualified as `<schema>.<table>`. |
| `#[entity(comparable = true)]`        | Derives `PartialEq`/`Eq` comparing only the `#[primary_key(...)]` field(s). Requires a `#[primary_key(...)]` attribute.                                    |
| `#[entity(hashable = true)]`          | Derives `Hash` based only on the `#[primary_key(...)]` field(s). Requires a `#[primary_key(...)]` attribute.                                               |
| `#[primary_key(field_a, field_b, ...)]` | Struct-level attribute marking the listed fields as the primary key. Gets you, on the repository, `select_by_id`/`exists` (each with an `_in` variant), and on the entity instance, `update_by_id`/`delete_by_id` (each with an `_in` variant taking a `&rusqlite::Transaction`). Multiple fields are combined with `AND`. Referencing a field that doesn't exist on the struct is a compile error. |
| `#[index("name", (col_a, col_b))]`    | Generates `select_by_name(db, ..., order_by)` (and `_in_conn`/`count_by_name`/`count_by_name_in_conn` variants) for the given column group. Can be repeated for multiple indexes. Returns `Vec<Self>`. |
| `#[unique("name", (col_d, col_e))]`   | Same syntax as `#[index(...)]`, but for a column group that is unique. Generates `select_by_name(db, ...)` (and `_in_conn`/`exists_by_name`/`exists_by_name_in_conn` variants) returning `Option<Self>` instead of `Vec<Self>`, and without an `order_by` parameter (see [Indexes and unique indexes](#indexes-and-unique-indexes) below). |

**Field-level attributes**

| Attribute                                            | Effect                                                                                                                                                                                                                                             |
| ---------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `#[column("...")]`                                   | Overrides the SQL column name (defaults to the field name, lowercased). Only the SQL name changes — the generated `entity::columns::*` constant is still named after the Rust field, not this override.                                          |
| `#[transient]`                                       | Excludes the field from `INSERT`/`SELECT` column lists entirely. When mapping a row back into the struct, this field is filled in via `Default::default()` — the struct must implement `Default`.                                                  |
| `#[default]`                                         | Excludes the field from the generated `INSERT` column list only — it's still part of `SELECT`/`FIELDS`. Use it for a column with a SQL-level `DEFAULT` you want SQLite to apply instead of sending a value from Rust (see [Default and autoincrement columns](#default-and-autoincrement-columns) below). |
| `#[autoincrement]`                                   | Implies `#[default]`, and additionally marks this field as the one that receives the SQLite-assigned `rowid` after each insert. Only one field per struct may carry this attribute, and it must be typed `i64` — both are compile errors otherwise (see [Default and autoincrement columns](#default-and-autoincrement-columns) below). |
| `#[relationship((local_field, remote_column), ...)]` | Declares the field as a related entity rather than a persisted column (see [Relationships](#relationships) below).                                                                                                                                 |

Every persisted field's type must implement `Into<rusqlite_orm::types::value::Value>` — this covers all integer widths (`i8`…`i64`, `isize`, `u8`…`u64`, `usize`), `f32`/`f64`, `bool`, `String`, `Vec<u8>` (mapped to a BLOB column) and `Option<T>` for any of the above (mapped to `NULL` when absent).

**Generated code**

- `mod entity { pub mod columns { ... } }` — a typed `ColumnName<Self>` constant for every persisted field, named after the **field** in upper case (e.g. field `email` → `entity::columns::EMAIL`) — note that this is the field's own name, not its `#[column("...")]` override, so a field named `email` with `#[column("email_address")]` still gets `entity::columns::EMAIL`, not `entity::columns::EMAIL_ADDRESS` — plus `entity::TABLE` and `entity::SCHEMA`.
- An `impl rusqlite_orm::dao::Entity for YourStruct` providing `SCHEMA`, `TABLE_NAME`, `FIELDS`, `INSERT_FIELDS`, `AUTOINCREMENT_FIELD`, `map_from_row`, `get_insert_values`, and `set_autoincrement_id`.
- A `YourStructRepository` struct implementing `rusqlite_orm::dao::Repository<YourStruct>`.
- `exists(db, ...)` / `select_by_id(db, ...)` (+ `_in`) and, as instance methods, `update_by_id(db)` / `delete_by_id(db)` (+ `_in`) when the struct has a `#[primary_key(...)]` attribute.
- `select_by_<name>(db, ...)` / `count_by_<name>(db, ...)` (or `exists_by_<name>(db, ...)` for `#[unique(...)]`) (+ `_in_conn`) for every index declared with `#[index(...)]` or `#[unique(...)]`.
- `PartialEq`/`Eq` and/or `Hash` impls when `comparable`/`hashable` are enabled.
- `fetch_<field>_relationship(db)` / `fetch_<field>_relationship_in_conn(conn)` for every field annotated with `#[relationship(...)]`.

Every generated function comes in two forms: a **managed** one (e.g. `select_by_id(db, id)`) that takes `db: &rusqlite_orm::database::DatabasePool` and opens its own pooled connection or transaction internally, and a connection-taking one for composing several calls atomically. The suffix for that second form isn't fully uniform across the crate: primary-key helpers (`exists_in`, `select_by_id_in`, and the instance methods `update_by_id_in`/`delete_by_id_in`) use plain `_in` and take a `conn: &rusqlite_orm::rusqlite::Connection` (the primary-key instance methods specifically need a `&rusqlite_orm::rusqlite::Transaction`, since they go through `UpdateBuilder`/`DeleteBuilder`), while index/unique/relationship helpers (`select_by_id_in_conn`, `count_by_name_in_conn`, `fetch_<field>_relationship_in_conn`, ...) use `_in_conn` and take a `conn: &rusqlite_orm::rusqlite::Connection`. Pass a `&mut rusqlite::Transaction` anywhere a `&rusqlite::Connection` is expected — it reborrows, since `Transaction` derefs to `Connection`.

Index/primary-key parameter types mirror the field types, with two exceptions to avoid unnecessary cloning: a `String` field becomes a `&str` parameter, and a `Vec<u8>` field becomes a `&[u8]` parameter.

## Indexes and unique indexes

`#[index(...)]` and `#[unique(...)]` share the same syntax and are declared at struct level, alongside `#[entity(...)]`. Each declares a single named index, and the attribute can be repeated as many times as needed:

```rust
#[derive(Entity, Debug, Clone, Default)]
#[entity(table = "users")]
#[primary_key(id)]
#[index("last_name", (last_name))]
#[unique("email", (email))]
#[unique("tenant_username", (tenant_id, username))]
#[unique("active_by_tenant", (tenant_id), (status = "active"))]
pub struct User {
    pub id: i64,
    pub tenant_id: i64,
    pub username: String,
    pub email: String,
    pub last_name: String,
    pub status: String,
}
```

Each attribute takes:

1. A string literal **name** (e.g. `"last_name"`), used to build the generated function names (`select_by_<name>`, etc.) and doc comments.
2. A parenthesized list of **columns** (e.g. `(tenant_id, username)`): these become parameters of the generated functions.
3. An optional second parenthesized list of **fixed conditions** (e.g. `(status = "active")`), as `ident = literal` pairs: the index is restricted to that constant value and it does *not* become a parameter.

**`#[index(...)]` vs. `#[unique(...)]`**

| | `#[index(...)]` | `#[unique(...)]` |
| --- | --- | --- |
| Meaning | Non-unique lookup index | Unique lookup index (at most one matching row) |
| Return type | `Vec<Self>` | `Option<Self>` |
| `order_by` parameter | Yes | No — a unique index can match at most one row, so ordering is meaningless |
| Count/exists function | `count_by_<name>` -> `i64` | `exists_by_<name>` -> `bool` |
| Fetch method used internally | `fetch_in` | `fetch_one_in` |

For `#[unique("tenant_username", (tenant_id, username))]`, the macro generates on the repository impl:

- `select_by_tenant_username(db, tenant_id, username) -> Result<Option<Self>>`
- `select_by_tenant_username_in_conn(conn, tenant_id, username) -> Result<Option<Self>>`
- `exists_by_tenant_username(db, tenant_id, username) -> Result<bool>`
- `exists_by_tenant_username_in_conn(conn, tenant_id, username) -> Result<bool>`

For `#[index("last_name", (last_name))]`, the equivalent non-unique set is generated with an extra `order_by` parameter and `count_by_last_name(db, ...)`/`count_by_last_name_in_conn(conn, ...)` returning `i64` instead of `exists_by_*`/`bool`.

The `#[unique(...)]` attribute only generates lookup functions based on the assumption that the column group is unique; it does **not** create a `UNIQUE` constraint in the database schema itself — that still has to be declared in your DDL (see [`dlls!(path)`](#dllspath) below).

## Default and autoincrement columns

```rust
#[derive(Entity, Debug, Clone, Default)]
#[entity(table = "users")]
#[primary_key(id)]
pub struct User {
    #[autoincrement]
    pub id: i64,
    pub email: String,
    #[default]
    pub created_at: i64,
}
```

- `#[default]` removes a field from the generated `INSERT` column list — the statement omits the column entirely, so SQLite falls back to whatever `DEFAULT` (or `NULL`) is declared for it in your DDL. The field is still part of `FIELDS`/`SELECT` and mapped back normally by `map_from_row`.
- `#[autoincrement]` implies `#[default]` (it's also skipped on `INSERT`) and additionally:
  - sets `Entity::AUTOINCREMENT_FIELD` to `true` for the struct,
  - generates an `Entity::set_autoincrement_id(&mut self, id: i64)` that assigns the field,
  - is only valid on an `i64` field — a compile error is raised otherwise,
  - can only be used once per struct — a second `#[autoincrement]` field is a compile error.
- Fields without either attribute are still part of `INSERT` (the pre-existing behavior) and are collected into `Entity::INSERT_FIELDS`; `get_insert_values()` returns values for exactly those fields, in the same order.

**Effect on `InsertBuilder`**

Because inserting a row with an autoincrement field needs to read back SQLite's `last_insert_rowid()` and write it into the struct, `InsertBuilder::item(...)` takes `&'a mut T` rather than an owned `T`, and `execute`/`execute_in` take `&mut self`:

```rust
let mut user = User { id: 0, email: "alice@example.com".into(), created_at: 0 };
UserRepository::insert().item(&mut user).execute(&db)?;
// user.id now holds the rowid SQLite assigned to the row
```

When `AUTOINCREMENT_FIELD` is `true`, each item is inserted with its own `INSERT` statement (instead of one multi-row statement) so the generated id can be read and assigned per row; entities without an autoincrement field are still batched into a single multi-row `INSERT`.

## Relationships

A field annotated with `#[relationship(...)]` doesn't map to a column in `TABLE_NAME`. Instead, it holds a related entity (or collection of entities) that can be lazily loaded from the database on demand.

```rust
#[derive(Entity, Debug, Clone, Default)]
#[entity(table = "posts")]
#[primary_key(id)]
pub struct Post {
    pub id: i64,
    pub user_id: i64,
    pub title: String,

    #[relationship((user_id, super::user::entity::columns::ID))]
    pub author: Option<User>,

    #[relationship((id, super::comment::entity::columns::POST_ID))]
    pub comments: Vec<Comment>,
}
```

**Field type determines cardinality**

| Field type  | Loaded via     | Meaning                                                     |
| ----------- | --------------- | ----------------------------------------------------------- |
| `Option<T>` | `fetch_one_in`  | At most one related `T` row (e.g. a "belongs to" relation). |
| `Vec<T>`    | `fetch_in`      | Zero or more related `T` rows (e.g. a "has many" relation). |

In both cases `T` must implement `rusqlite_orm::dao::Entity` (i.e. it must itself be a `#[derive(Entity)]` struct).

**Join arguments**

`#[relationship(...)]` takes one or more `(local_field, remote_column)` pairs:

- `local_field` is the name of a field on the _current_ struct whose value is used for the join.
- `remote_column` is a path to the typed column constant on the _related_ entity's `entity::columns` module (e.g. `other::entity::columns::USER_ID`).

A single pair generates a simple `Where::Eq(remote_column, self.local_field.into())` condition. Multiple pairs are combined with `Where::And(...)`, letting you express composite-key joins:

```rust
#[relationship(
    (tenant_id, super::membership::entity::columns::TENANT_ID),
    (user_id, super::membership::entity::columns::USER_ID)
)]
pub membership: Option<Membership>,
```

**Behavior**

- Fields marked `#[relationship(...)]` are implicitly treated like `#[transient]`: they are excluded from `INSERT`/`SELECT` column lists and are populated via `Default::default()` when a row is first mapped into the struct, so the struct must implement `Default`. `#[transient]` and `#[relationship(...)]` cannot be combined on the same field — that's a compile error.
- The macro generates two **instance methods** per relationship field (not on the repository, but directly on `YourStruct`):
  - `fetch_<field>_relationship(&mut self, db: &rusqlite_orm::database::DatabasePool) -> rusqlite_orm::errors::Result<()>` — opens its own pooled connection via `db.run_in_connection(...)`, runs `<T>Repository::select().where_(<condition>)`, and assigns the result into `self.<field>`.
  - `fetch_<field>_relationship_in_conn(&mut self, conn: &rusqlite_orm::rusqlite::Connection) -> rusqlite_orm::errors::Result<()>` — same, but reuses an existing connection or transaction so it can be composed with other calls.
- These methods mutate `self` in place; they don't return the related data, so call them and then read `self.<field>` afterwards.

```rust
let mut post = PostRepository::select_by_id(&db, 1)?.unwrap();
post.fetch_author_relationship(&db)?;
post.fetch_comments_relationship(&db)?;

println!("{:?} has {} comments", post.author, post.comments.len());
```

## `dlls!(path)`

Reads every `<version>_<name>.sql` file in the given directory (resolved relative to `CARGO_MANIFEST_DIR`) at compile time and embeds them into:

```rust
pub static DDLS: [rusqlite_orm::database::DdlVersion; N] = [ ... ];
```

Each SQL file must start with a `--` comment line, used as the migration's human-readable description; the numeric prefix before the first `_` in the filename is used as the migration's version number. Blank lines and comment lines are stripped from the embedded SQL body. The resulting array is meant to be passed to `DatabasePool::create_schema(&DDLS)` (see [`orm/README.md`](../orm/README.md#3-open-the-database-and-apply-the-schema) for how to build the `DatabasePool` itself).

## License

MIT
