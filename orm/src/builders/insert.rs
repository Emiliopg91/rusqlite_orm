use std::fmt::Write;
use std::marker::PhantomData;

use log::debug;

use crate::database::DatabasePool;
use crate::rusqlite::{limits::Limit, params_from_iter};

use crate::{
    builders::{QueryBuilder, log_query_ending, log_query_start},
    dao::Entity,
    errors::DatabaseError,
    types::{column_name::write_column_list, value::Value},
};

pub struct InsertBuilder<'a, T> {
    items: Vec<&'a mut T>,
    or_ignore: bool,
    or_replace: bool,
    _marker: PhantomData<T>,
}

impl<'a, T> QueryBuilder<T> for InsertBuilder<'a, T> {
    fn new() -> Self {
        InsertBuilder {
            items: Vec::new(),
            or_ignore: false,
            or_replace: false,
            _marker: PhantomData,
        }
    }
}

impl<'a, T> InsertBuilder<'a, T>
where
    T: Entity,
{
    pub fn item(mut self, item: &'a mut T) -> Self {
        self.items.push(item);
        self
    }

    pub fn or_replace(mut self) -> Self {
        self.or_replace = true;
        self
    }

    pub fn or_ignore(mut self) -> Self {
        self.or_ignore = true;
        self
    }

    /// A single item runs on a pooled connection; several run inside a transaction, as they
    /// may take more than one statement and must be inserted all or none.
    ///
    /// The items stay on the caller's side: only their values travel to the blocking thread,
    /// and the autoincrement ids it reads back are assigned once it finishes.
    pub async fn execute(&mut self, db: &DatabasePool) -> crate::errors::Result<usize> {
        let rows: Vec<Vec<Value>> = self
            .items
            .iter()
            .map(|item| item.get_insert_values())
            .collect();
        let mode = self.mode();

        let (inserted, ids) = if rows.len() <= 1 {
            db.run_in_connection(move |conn| Ok(insert_rows::<T>(conn, mode, &rows)?))
                .await?
        } else {
            db.run_in_transaction(move |tx| Ok(insert_rows::<T>(tx, mode, &rows)?))
                .await?
        };

        self.assign_ids(ids);
        Ok(inserted)
    }

    /// Runs on the given connection, which may be a `Transaction` through `Deref`.
    ///
    /// Several items may take more than one statement (one per item with an autoincrement
    /// field, or one per batch of `SQLITE_LIMIT_VARIABLE_NUMBER` parameters otherwise), so they
    /// are only inserted atomically when `conn` is a transaction.
    pub fn execute_in(
        &mut self,
        conn: &crate::rusqlite::Connection,
    ) -> crate::errors::Result<usize> {
        let rows: Vec<Vec<Value>> = self
            .items
            .iter()
            .map(|item| item.get_insert_values())
            .collect();
        let (inserted, ids) = insert_rows::<T>(conn, self.mode(), &rows)?;
        self.assign_ids(ids);
        Ok(inserted)
    }

    fn mode(&self) -> InsertMode {
        if self.or_ignore {
            InsertMode::OrIgnore
        } else if self.or_replace {
            InsertMode::OrReplace
        } else {
            InsertMode::Plain
        }
    }

    /// Sets the autoincrement id read back for each item that was actually inserted.
    fn assign_ids(&mut self, ids: Vec<Option<i64>>) {
        for (item, id) in self.items.iter_mut().zip(ids) {
            if let Some(id) = id {
                item.set_autoincrement_id(id);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum InsertMode {
    Plain,
    OrIgnore,
    OrReplace,
}

/// Inserts `rows` (each one the insert values of an item) and returns how many were inserted
/// plus, for entities with an autoincrement field, the id assigned to each row that was
/// (`None` for skipped rows, or for every row of entities without one).
fn insert_rows<T: Entity>(
    conn: &crate::rusqlite::Connection,
    mode: InsertMode,
    rows: &[Vec<Value>],
) -> crate::errors::Result<(usize, Vec<Option<i64>>)> {
    let mut ids = vec![None; rows.len()];
    if rows.is_empty() {
        return Ok((0, ids));
    }

    let mut sentence = match mode {
        InsertMode::Plain => "INSERT ",
        InsertMode::OrIgnore => "INSERT OR IGNORE ",
        InsertMode::OrReplace => "INSERT OR REPLACE ",
    }
    .to_string();

    let _ = write!(sentence, "INTO '{}'.'{}' (", T::SCHEMA, T::TABLE_NAME);
    write_column_list(&mut sentence, T::INSERT_FIELDS);
    sentence.push_str(") VALUES ");

    let mut row_placeholders = String::with_capacity(T::INSERT_FIELDS.len() * 3 + 2);
    row_placeholders.push('(');
    for i in 0..T::INSERT_FIELDS.len() {
        row_placeholders.push_str(if i == 0 { "?" } else { ", ?" });
    }
    row_placeholders.push(')');

    if T::AUTOINCREMENT_FIELD {
        sentence.push_str(&row_placeholders);

        // Same statement for every item: prepare it once.
        let mut stmt = conn
            .prepare_cached(&sentence)
            .map_err(DatabaseError::Insert)?;

        let mut inserted = 0;
        for (values, id) in rows.iter().zip(ids.iter_mut()) {
            log_query_start(&sentence, values);
            let item_inserted = stmt
                .execute(params_from_iter(values.iter()))
                .map_err(DatabaseError::Insert)?;
            log_query_ending(item_inserted, "Inserted");
            if item_inserted > 0 {
                let aid = conn.last_insert_rowid();
                debug!("Asigned autoincrement value {}", aid);
                *id = Some(aid);
            }
            inserted += item_inserted;
        }

        Ok((inserted, ids))
    } else {
        // A single statement can bind at most SQLITE_LIMIT_VARIABLE_NUMBER
        // parameters, so large batches are split into several multi-row INSERTs
        // (atomic only when the caller runs them inside a transaction).
        let rows_per_statement = max_rows_per_statement(conn, T::INSERT_FIELDS.len());

        let mut inserted = 0;
        for chunk in rows.chunks(rows_per_statement) {
            let mut chunk_sentence = sentence.clone();
            for i in 0..chunk.len() {
                if i > 0 {
                    chunk_sentence.push_str(", ");
                }
                chunk_sentence.push_str(&row_placeholders);
            }

            let values = chunk.iter().flatten();

            log_query_start(&chunk_sentence, values.clone());
            inserted += conn
                .prepare_cached(&chunk_sentence)
                .and_then(|mut stmt| stmt.execute(params_from_iter(values)))
                .map_err(DatabaseError::Insert)?;
        }
        log_query_ending(inserted, "Inserted");

        Ok((inserted, ids))
    }
}

/// SQLite's default `SQLITE_MAX_VARIABLE_NUMBER` before 3.32, the lowest limit any
/// build is expected to have; used if the connection can't report its own.
const FALLBACK_MAX_VARIABLES: usize = 999;

/// How many rows of `columns` bound parameters each fit in one statement.
fn max_rows_per_statement(conn: &crate::rusqlite::Connection, columns: usize) -> usize {
    let max_variables = conn
        .limit(Limit::SQLITE_LIMIT_VARIABLE_NUMBER)
        .ok()
        .and_then(|limit| usize::try_from(limit).ok())
        .filter(|limit| *limit > 0)
        .unwrap_or(FALLBACK_MAX_VARIABLES);

    (max_variables / columns.max(1)).max(1)
}
