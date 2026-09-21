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

    pub fn execute(&mut self, db: &DatabasePool) -> crate::errors::Result<usize> {
        db.run_in_transaction(|tx| {
            let res = self.execute_in(tx)?;
            Ok(res)
        })
    }

    pub fn execute_in(
        &mut self,
        tx: &crate::rusqlite::Transaction,
    ) -> crate::errors::Result<usize> {
        if self.items.is_empty() {
            return Ok(0);
        }

        let mut sentence = "INSERT ".to_string();

        if self.or_ignore {
            sentence.push_str("OR IGNORE ");
        } else {
            if self.or_replace {
                sentence.push_str("OR REPLACE ");
            }
        }

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
            let mut stmt = tx
                .prepare_cached(&sentence)
                .map_err(DatabaseError::Insert)?;

            let mut inserted = 0;
            for item in &mut self.items {
                let values = T::get_insert_values(item);
                log_query_start(&sentence, &values);
                let item_inserted = stmt
                    .execute(params_from_iter(values.iter()))
                    .map_err(DatabaseError::Insert)?;
                log_query_ending(item_inserted, "Inserted");
                if item_inserted > 0 {
                    let aid = tx.last_insert_rowid();
                    debug!("Asigned autoincrement value {}", aid);
                    item.set_autoincrement_id(aid);
                }
                inserted += item_inserted;
            }

            Ok(inserted)
        } else {
            // A single statement can bind at most SQLITE_LIMIT_VARIABLE_NUMBER
            // parameters, so large batches are split into several multi-row INSERTs
            // (all inside the caller's transaction, hence still atomic).
            let rows_per_statement = max_rows_per_statement(tx, T::INSERT_FIELDS.len());

            let mut inserted = 0;
            for chunk in self.items.chunks(rows_per_statement) {
                let mut chunk_sentence = sentence.clone();
                for i in 0..chunk.len() {
                    if i > 0 {
                        chunk_sentence.push_str(", ");
                    }
                    chunk_sentence.push_str(&row_placeholders);
                }

                let mut values: Vec<Value> =
                    Vec::with_capacity(chunk.len() * T::INSERT_FIELDS.len());
                for item in chunk {
                    values.extend(T::get_insert_values(item));
                }

                log_query_start(&chunk_sentence, &values);
                inserted += tx
                    .prepare_cached(&chunk_sentence)
                    .and_then(|mut stmt| stmt.execute(params_from_iter(values.iter())))
                    .map_err(DatabaseError::Insert)?;
            }
            log_query_ending(inserted, "Inserted");

            Ok(inserted)
        }
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
