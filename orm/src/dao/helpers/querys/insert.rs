use std::marker::PhantomData;

use crate::rusqlite::params_from_iter;

use crate::{
    dao::{
        Entity,
        helpers::{querys::QueryBuilder, types::value::Value},
    },
    database::{DATABASE_INST, errors::DatabaseError},
};

pub struct InsertBuilder<T> {
    items: Vec<T>,
    or_ignore: bool,
    _marker: PhantomData<T>,
}

impl<T> QueryBuilder<T> for InsertBuilder<T> {
    fn new() -> Self {
        InsertBuilder {
            items: Vec::new(),
            or_ignore: false,
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

    pub fn or_ignore(mut self, or_ignore: bool) -> Self {
        self.or_ignore = or_ignore;
        self
    }

    pub fn execute_in_tx(
        &self,
        tx: &crate::rusqlite::Transaction,
    ) -> crate::database::errors::Result<usize> {
        let mut sentence = "INSERT ".to_string();

        if self.or_ignore {
            sentence.push_str("OR IGNORE ");
        }

        sentence.push_str(&format!(
            "INTO {}.{} ({}) VALUES ",
            T::SCHEMA,
            T::TABLE_NAME,
            T::FIELDS
                .iter()
                .map(|f| f.as_ref().to_string())
                .collect::<Vec<String>>()
                .join(", "),
        ));

        let values_str =
            vec![format!("({})", vec!["?"; T::FIELDS.len()].join(", ")); self.items.len()]
                .join(", ");

        sentence.push_str(&values_str);

        let values = self
            .items
            .iter()
            .flat_map(|item| T::get_values(item).into_iter())
            .collect::<Vec<Value>>();

        Self::log_query_start(&sentence, &values);
        let inserted = tx
            .execute(&sentence, params_from_iter(values.iter()))
            .map_err(DatabaseError::Insert)?;
        Self::log_query_ending(inserted, "Inserted");

        Ok(inserted)
    }

    pub fn execute(&self) -> crate::database::errors::Result<usize> {
        let mut db = DATABASE_INST.lock().unwrap();
        db.run_in_tx(|tx| self.execute_in_tx(tx))
    }
}
