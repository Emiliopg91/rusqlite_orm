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
| `#[entity(schema = "...")]`           | Overrides the SQL schema name (defaults to `"main"`).                                                                                                      |
| `#[entity(comparable = true)]`        | Derives `PartialEq`/`Eq` comparing only the `#[primary_key(...)]` field(s). Requires a `#[primary_key(...)]` attribute.                                    |
| `#[entity(hashable = true)]`          | Derives `Hash` based only on the `#[primary_key(...)]` field(s). Requires a `#[primary_key(...)]` attribute.                                               |
| `#[primary_key(field_a, field_b, ...)]` | Struct-level attribute marking the listed fields as the primary key. Gets you, on the repository, `select_by_id`/`exists`, and on the entity instance, `update_by_id`/`delete_by_id` (each with an `_in_tx` variant). Multiple fields are combined with `AND`. Referencing a field that doesn't exist on the struct is a compile error. |
| `#[index("name", (col_a, col_b))]`    | Generates `select_by_name(..., order_by)` (and `_in_tx`/`count_by_name`/`count_by_name_in_tx` variants) for the given column group. Can be repeated for multiple indexes. Returns `Vec<Self>`. |
| `#[unique("name", (col_d, col_e))]`   | Same syntax as `#[index(...)]`, but for a column group that is unique. Generates `select_by_name(...)` (and `_in_tx`/`exists_by_name`/`exists_by_name_in_tx` variants) returning `Option<Self>` instead of `Vec<Self>`, and without an `order_by` parameter (see [Indexes and unique indexes](#indexes-and-unique-indexes) below). |

**Field-level attributes**

| Attribute                                            | Effect                                                                                                                                                                                                                                             |
| ---------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `#[column("...")]`                                   | Overrides the column name (defaults to the field name, lowercased).                                                                                                                                                                                |
| `#[transient]`                                       | Excludes the field from `INSERT`/`SELECT` column lists entirely. When mapping a row back into the struct, this field is filled in via `Default::default()` — the struct must implement `Default`.                                                  |
| `#[relationship((local_field, remote_column), ...)]` | Declares the field as a related entity rather than a persisted column (see [Relationships](#relationships) below).                                                                                                                                 |

**Generated code**

- `mod entity { pub mod columns { ... } }` — a typed `ColumnName<Self>` constant for every persisted field, named after the field in upper case (e.g. `entity::columns::EMAIL_ADDRESS`), plus `entity::TABLE`.
- An `impl rusqlite_orm::dao::Entity for YourStruct` providing `TABLE_NAME`, `FIELDS`, `map_from_row`, and `get_values`.
- A `YourStructRepository` struct implementing `rusqlite_orm::dao::Repository<YourStruct>`.
- `exists` / `select_by_id` / `update_by_id` / `delete_by_id` (+ `_in_tx`) when the struct has a `#[primary_key(...)]` attribute.
- `select_by_<name>` / `count_by_<name>` (or `exists_by_<name>` for `#[unique(...)]`) (+ `_in_tx`) for every index declared with `#[index(...)]` or `#[unique(...)]`.
- `PartialEq`/`Eq` and/or `Hash` impls when `comparable`/`hashable` are enabled.
- `fetch_<field>_relationship` / `fetch_<field>_relationship_in_tx` for every field annotated with `#[relationship(...)]`.

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
| Fetch method used internally | `fetch_in_tx` | `fetch_one_in_tx` |

For `#[unique("tenant_username", (tenant_id, username))]`, the macro generates on the repository impl:

- `select_by_tenant_username(tenant_id, username) -> Result<Option<Self>>`
- `select_by_tenant_username_in_tx(tx, tenant_id, username) -> Result<Option<Self>>`
- `exists_by_tenant_username(tenant_id, username) -> Result<bool>`
- `exists_by_tenant_username_in_tx(tx, tenant_id, username) -> Result<bool>`

For `#[index("last_name", (last_name))]`, the equivalent non-unique set is generated with an extra `order_by` parameter and `count_by_last_name(...)`/`count_by_last_name_in_tx(...)` returning `i64` instead of `exists_by_*`/`bool`.

The `#[unique(...)]` attribute only generates lookup functions based on the assumption that the column group is unique; it does **not** create a `UNIQUE` constraint in the database schema itself — that still has to be declared in your DDL (see [`dlls!(path)`](#dllspath) below).

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
