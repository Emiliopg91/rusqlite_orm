use std::marker::PhantomData;

use crate::database::DatabasePool;
use crate::rusqlite::params_from_iter;


use crate::types::column_name::ColumnName;
use crate::{builders::QueryBuilder, dao::Entity, errors::DatabaseError, types::value::Value};

pub struct InsertBuilder<T> {
    items: Vec<T>,
    or_ignore: bool,
    or_replace: bool,
    skipped_fields: Vec<String>,
    _marker: PhantomData<T>,
}

impl<T> QueryBuilder<T> for InsertBuilder<T> {
    fn new() -> Self {
        InsertBuilder {
            items: Vec::new(),
            or_ignore: false,
            or_replace: false,
            skipped_fields: Vec::new(),
            _marker: PhantomData,
        }
    }
}

impl<T> InsertBuilder<T>
where
    T: Entity,
{
    pub fn item(mut self, item: T) -> Self {
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

    pub fn skip_field(mut self, col: ColumnName<T>) -> Self {
        self.skipped_fields.push(col.to_string());
        self
    }

    pub fn execute(&self, db: &DatabasePool) -> crate::errors::Result<usize> {
        db.run_in_transaction(|tx| {
            let res = self.execute_in(tx)?;
            Ok(res)
        })
    }

    pub fn execute_in(&self, tx: &crate::rusqlite::Transaction) -> crate::errors::Result<usize> {
        let mut sentence = "INSERT ".to_string();

        if self.or_ignore {
            sentence.push_str("OR IGNORE ");
        } else {
            if self.or_replace {
                sentence.push_str("OR REPLACE ");
            }
        }

        let fields_without_skipped = T::FIELDS
            .iter()
            .map(|f| f.as_ref().to_string())
            .filter(|f| !self.skipped_fields.contains(f))
            .collect::<Vec<String>>();

        sentence.push_str(&format!(
            "INTO {}.{} ({}) VALUES ",
            T::SCHEMA,
            T::TABLE_NAME,
            fields_without_skipped.join(", "),
        ));

        let values_str = vec![
            format!("({})", vec!["?"; fields_without_skipped.len()].join(", "));
            self.items.len()
        ]
        .join(", ");

        sentence.push_str(&values_str);

        let values = self
            .items
            .iter()
            .flat_map(|item| {
                T::get_values_with_field_names(item)
                    .into_iter()
                    .filter(|f| !self.skipped_fields.contains(&f.0))
                    .map(|f| f.1)
            })
            .collect::<Vec<Value>>();

        Self::log_query_start(&sentence, &values);
        let inserted = tx
            .execute(&sentence, params_from_iter(values.iter()))
            .map_err(DatabaseError::Insert)?;
        Self::log_query_ending(inserted, "Inserted");

        Ok(inserted)
    }
}
