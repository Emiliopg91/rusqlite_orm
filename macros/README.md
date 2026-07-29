# rusqlite_orm_macros

Procedural macros for [`rusqlite_orm`](../orm). This crate is meant to be used together with `rusqlite_orm`, not on its own.

It provides two macros:

## `#[derive(Entity)]`

Generates the boilerplate needed to treat a struct as a database entity.

```rust
#[derive(Entity, Debug, Clone, Default)]
#[entity(table = "users", comparable = true, hasheable = true)]
#[indexes((email))]
pub struct User {
    #[primary_key]
    pub id: i64,
    #[column(name = "email_address")]
    pub email: String,
    pub name: String,
    #[no_column]
    pub transient_flag: bool,
}
```

**Struct-level attributes**

| Attribute                             | Effect                                                                                                                                        |
| ------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `#[entity(table = "...")]`            | Overrides the SQL table name (defaults to the struct name, lowercased).                                                                       |
| `#[entity(comparable = true)]`        | Derives `PartialEq`/`Eq` comparing only the `#[primary_key]` field(s). Requires at least one `#[primary_key]` field.                          |
| `#[entity(hasheable = true)]`         | Derives `Hash` based only on the `#[primary_key]` field(s). Requires at least one `#[primary_key]` field.                                     |
| `#[indexes((col_a, col_b), (col_c))]` | Generates `select_by_col_a_and_col_b(..., order_by)` / `select_by_col_c(..., order_by)` (and `_in_tx` variants) for each listed column group. |

**Field-level attributes**

| Attribute                 | Effect                                                                                                                                                                                                                                   |
| ------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `#[primary_key]`          | Marks the field as (part of) the primary key. Any struct with one or more `#[primary_key]` fields gets `select_by_id`, `update_by_id`, `delete_by_id` (and `_in_tx` variants). Multiple `#[primary_key]` fields are combined with `AND`. |
| `#[column(name = "...")]` | Overrides the column name (defaults to the field name, lowercased).                                                                                                                                                                      |
| `#[no_column]`            | Excludes the field from `INSERT`/`SELECT` column lists entirely. When mapping a row back into the struct, this field is filled in via `Default::default()` — the struct must implement `Default`.                                        |

**Generated code**

- `mod entity { pub mod columns { ... } }` — a typed `ColumnName<Self>` constant for every persisted field, named after the field in upper case (e.g. `entity::columns::EMAIL_ADDRESS`).
- An `impl rusqlite_orm::dao::Entity for YourStruct` providing `TABLE_NAME`, `FIELDS`, `map_from_row`, and `get_values`.
- `select_by_id` / `update_by_id` / `delete_by_id` (+ `_in_tx`) when the struct has `#[primary_key]` field(s).
- `select_by_<fields>` (+ `_in_tx`) for every group declared in `#[indexes(...)]`.
- `PartialEq`/`Eq` and/or `Hash` impls when `comparable`/`hasheable` are enabled.

## `dlls!(path)`

Reads every `<version>_<name>.sql` file in the given directory (resolved relative to `CARGO_MANIFEST_DIR`) at compile time and embeds them into:

```rust
pub static DDLS: [rusqlite_orm::database::DdlVersion; N] = [ ... ];
```

Each SQL file must start with a `--` comment line, used as the migration's human-readable description; the numeric prefix before the first `_` in the filename is used as the migration's version number. Blank lines and comment lines are stripped from the embedded SQL body. The resulting array is meant to be passed to `Database::create_schema(&DDLS)`.

## License

MIT
