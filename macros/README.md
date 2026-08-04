# rusqlite_orm_macros

Procedural macros for [`rusqlite_orm`](../orm). This crate is meant to be used together with `rusqlite_orm`, not on its own.

It provides two macros:

## `#[derive(Entity)]`

Generates the boilerplate needed to treat a struct as a database entity.

```rust
#[derive(Entity, Debug, Clone, Default)]
#[entity(table = "users", comparable = true, hashable = true)]
#[indexes((email))]
pub struct User {
    #[primary_key]
    pub id: i64,
    #[column(name = "email_address")]
    pub email: String,
    pub name: String,
    #[transient]
    pub transient_flag: bool,
}
```

**Struct-level attributes**

| Attribute                             | Effect                                                                                                                                                     |
| ------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `#[entity(table = "...")]`            | Overrides the SQL table name (defaults to the struct name, lowercased).                                                                                    |
| `#[entity(comparable = true)]`        | Derives `PartialEq`/`Eq` comparing only the `#[primary_key]` field(s). Requires at least one `#[primary_key]` field.                                       |
| `#[entity(hashable = true)]`          | Derives `Hash` based only on the `#[primary_key]` field(s). Requires at least one `#[primary_key]` field.                                                  |
| `#[indexes((col_a, col_b), (col_c))]` | Generates `select_by_col_a_and_col_b(..., order_by)` / `select_by_col_c(..., order_by)` (and `_in_tx`/`count_by_*` variants) for each listed column group. Returns `Vec<Self>`. |
| `#[uniques((col_d), (col_e, col_f))]` | Same syntax as `#[indexes(...)]`, but for column groups that are unique. Generates `select_by_col_d(...)` / `select_by_col_e_and_col_f(...)` (and `_in_tx`/`count_by_*` variants) returning `Option<Self>` instead of `Vec<Self>`, and without an `order_by` parameter (see [Indexes and unique indexes](#indexes-and-unique-indexes) below). |

**Field-level attributes**

| Attribute                                            | Effect                                                                                                                                                                                                                                             |
| ---------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `#[primary_key]`                                     | Marks the field as (part of) the primary key. Any struct with one or more `#[primary_key]` fields gets `select_by_id`, `exists`, `update_by_id`, `delete_by_id` (and `_in_tx` variants). Multiple `#[primary_key]` fields are combined with `AND`. |
| `#[column(name = "...")]`                            | Overrides the column name (defaults to the field name, lowercased).                                                                                                                                                                                |
| `#[transient]`                                       | Excludes the field from `INSERT`/`SELECT` column lists entirely. When mapping a row back into the struct, this field is filled in via `Default::default()` — the struct must implement `Default`.                                                  |
| `#[relationship((local_field, remote_column), ...)]` | Declares the field as a related entity rather than a persisted column (see [Relationships](#relationships) below).                                                                                                                                 |

**Generated code**

- `mod entity { pub mod columns { ... } }` — a typed `ColumnName<Self>` constant for every persisted field, named after the field in upper case (e.g. `entity::columns::EMAIL_ADDRESS`), plus `entity::TABLE`.
- An `impl rusqlite_orm::dao::Entity for YourStruct` providing `TABLE_NAME`, `FIELDS`, `map_from_row`, and `get_values`.
- A `YourStructRepository` struct implementing `rusqlite_orm::dao::Repository<YourStruct>`.
- `exists` / `select_by_id` / `update_by_id` / `delete_by_id` (+ `_in_tx`) when the struct has `#[primary_key]` field(s).
- `select_by_<fields>` / `count_by_<fields>` (+ `_in_tx`) for every group declared in `#[indexes(...)]` or `#[uniques(...)]`.
- `PartialEq`/`Eq` and/or `Hash` impls when `comparable`/`hashable` are enabled.
- `fetch_<field>_relationship` / `fetch_<field>_relationship_in_tx` for every field annotated with `#[relationship(...)]`.

## Indexes and unique indexes

`#[indexes(...)]` and `#[uniques(...)]` share the same syntax and are declared at struct level, alongside `#[entity(...)]`. Each takes one or more column groups, one per pair of parentheses:

```rust
#[derive(Entity, Debug, Clone, Default)]
#[entity(table = "users")]
#[indexes((last_name))]
#[uniques((email), (tenant_id, username))]
pub struct User {
    #[primary_key]
    pub id: i64,
    pub tenant_id: i64,
    pub username: String,
    pub email: String,
    pub last_name: String,
    pub status: String,
}
```

Within a group, a bare identifier (e.g. `tenant_id`) is a **variable column**: it becomes a parameter of the generated functions. An `ident = literal` pair (e.g. `status = "active"`) is instead a **fixed condition**: the index is restricted to that constant value and it does *not* become a parameter, but its sanitized value is appended to the function name (e.g. `select_by_tenant_id_where_status_eq_active`).

**`#[indexes(...)]` vs. `#[uniques(...)]`**

| | `#[indexes(...)]` | `#[uniques(...)]` |
| --- | --- | --- |
| Meaning | Non-unique lookup group | Unique lookup group (at most one matching row) |
| Return type | `Vec<Self>` | `Option<Self>` |
| `order_by` parameter | Yes | No — a unique index can match at most one row, so ordering is meaningless |
| Fetch method used internally | `fetch_in_tx` | `fetch_one_in_tx` |

For a group `(tenant_id, username)` declared with `#[uniques(...)]`, the macro generates on the repository impl:

- `select_by_tenant_id_and_username(tenant_id, username) -> Result<Option<Self>>`
- `select_by_tenant_id_and_username_in_tx(tx, tenant_id, username) -> Result<Option<Self>>`
- `count_by_tenant_id_and_username(tenant_id, username) -> Result<i64>`
- `count_by_tenant_id_and_username_in_tx(tx, tenant_id, username) -> Result<i64>`

The `#[uniques(...)]` attribute only generates lookup functions based on the assumption that the column group is unique; it does **not** create a `UNIQUE` constraint in the database schema itself — that still has to be declared in your DDL (see [`dlls!(path)`](#dllspath) below).

## Relationships

A field annotated with `#[relationship(...)]` doesn't map to a column in `TABLE_NAME`. Instead, it holds a related entity (or collection of entities) that can be lazily loaded from the database on demand.

```rust
#[derive(Entity, Debug, Clone, Default)]
#[entity(table = "posts")]
pub struct Post {
    #[primary_key]
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

| Field type  | Loaded via        | Meaning                                                     |
| ----------- | ----------------- | ----------------------------------------------------------- |
| `Option<T>` | `fetch_one_in_tx` | At most one related `T` row (e.g. a "belongs to" relation). |
| `Vec<T>`    | `fetch_in_tx`     | Zero or more related `T` rows (e.g. a "has many" relation). |

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

- Fields marked `#[relationship(...)]` are implicitly treated like `#[transient]`: they are excluded from `INSERT`/`SELECT` column lists and are populated via `Default::default()` when a row is first mapped into the struct, so the struct must implement `Default`.
- The macro generates two **instance methods** per relationship field (not on the repository, but directly on `YourStruct`):
  - `fetch_<field>_relationship(&mut self) -> rusqlite_orm::database::errors::Result<()>` — opens its own transaction against the global connection, runs `<T>Repository::select().where_(<condition>)`, and assigns the result into `self.<field>`.
  - `fetch_<field>_relationship_in_tx(&mut self, tx: &Transaction) -> rusqlite_orm::database::errors::Result<()>` — same, but reuses an existing transaction so it can be composed with other calls.
- These methods mutate `self` in place; they don't return the related data, so call them and then read `self.<field>` afterwards.

```rust
let mut post = PostRepository::select_by_id(1)?.unwrap();
post.fetch_author_relationship()?;
post.fetch_comments_relationship()?;

println!("{:?} has {} comments", post.author, post.comments.len());
```

## `dlls!(path)`

Reads every `<version>_<name>.sql` file in the given directory (resolved relative to `CARGO_MANIFEST_DIR`) at compile time and embeds them into:

```rust
pub static DDLS: [rusqlite_orm::database::DdlVersion; N] = [ ... ];
```

Each SQL file must start with a `--` comment line, used as the migration's human-readable description; the numeric prefix before the first `_` in the filename is used as the migration's version number. Blank lines and comment lines are stripped from the embedded SQL body. The resulting array is meant to be passed to `Database::create_schema(&DDLS)`.

## License

MIT
