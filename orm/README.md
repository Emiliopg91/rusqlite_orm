# rusqlite_orm

Runtime ORM layer for [`rusqlite`](https://crates.io/crates/rusqlite). This crate provides:

- The `Entity` trait, implemented for you by the `#[derive(Entity)]` macro from [`rusqlite_orm_macros`](../macros).
- Fluent query builders: `SelectBuilder`, `InsertBuilder`, `UpdateBuilder`, `DeleteBuilder`.
- A typed `Where<T>` condition tree and `OrderBy<T>` for building `WHERE` / `ORDER BY` clauses.
- A `Database` wrapper around a single `rusqlite::Connection`, with transaction helpers and SQL-file-based schema migrations tracked via `PRAGMA user_version`.
- A global, lazily-initialized `DATABASE_INST: Mutex<Database>` used by the non-`_in_tx` builder methods.

This crate re-exports `rusqlite` (`rusqlite_orm::rusqlite`), so downstream users generally don't need a direct dependency on `rusqlite`.

See the [workspace README](../README.md) for full usage examples (defining entities, running migrations, and CRUD operations).

## Module overview

```
rusqlite_orm
├── dao
│   ├── Entity                  // trait implemented via #[derive(Entity)]
│   └── helpers
│       ├── querys
│       │   ├── select.rs       // SelectBuilder: where_, order_by, limit, fetch(_in_tx)
│       │   ├── insert.rs       // InsertBuilder: item, or_ignore, execute(_in_tx)
│       │   ├── update.rs       // UpdateBuilder: set, where_, execute(_in_tx)
│       │   └── delete.rs       // DeleteBuilder: where_, execute(_in_tx)
│       └── types
│           ├── column_name.rs  // ColumnName<T>: typed, table-scoped column identifier
│           ├── value.rs        // Value: dynamic SQL value wrapper implementing ToSql
│           ├── where_clause.rs // Where<T>: Eq/NotEq/Gt/Lt/In/InMultiple/Null/NotNull/And/Or
│           └── order_by.rs     // OrderBy<T>: Asc/Desc
└── database
    ├── Database                // open, run_in_tx, create_schema
    ├── DdlVersion               // { version, description, sql } — produced by the dlls! macro
    └── errors                  // DatabaseError / Result<T>
```

## License

MIT
