use std::marker::PhantomData;

use log::debug;

use crate::database::DatabasePool;
use crate::rusqlite::params_from_iter;

use crate::{builders::QueryBuilder, dao::Entity, errors::DatabaseError, types::value::Value};

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
        let mut sentence = "INSERT ".to_string();

        if self.or_ignore {
            sentence.push_str("OR IGNORE ");
        } else {
            if self.or_replace {
                sentence.push_str("OR REPLACE ");
            }
        }

        sentence.push_str(&format!(
            "INTO '{}'.'{}' ({}) VALUES ",
            T::SCHEMA,
            T::TABLE_NAME,
            T::INSERT_FIELDS
                .iter()
                .map(|f| f.as_ref().to_string())
                .collect::<Vec<String>>()
                .join(", "),
        ));

        if T::AUTOINCREMENT_FIELD {
            sentence.push_str(&format!(
                "({})",
                vec!["?"; T::INSERT_FIELDS.len()].join(", ")
            ));

            let mut inserted = 0;
            for item in &mut self.items {
                let values = T::get_insert_values(item);
                Self::log_query_start(&sentence, &values);
                let item_inserted = tx
                    .execute(&sentence, params_from_iter(values.iter()))
                    .map_err(DatabaseError::Insert)?;
                Self::log_query_ending(item_inserted, "Inserted");
                if item_inserted > 0 {
                    let aid = tx.last_insert_rowid();
                    debug!("Asigned autoincrement value {}", aid);
                    item.set_autoincrement_id(aid);
                }
                inserted += item_inserted;
            }

            Ok(inserted)
        } else {
            sentence.push_str(
                &vec![
                    format!("({})", vec!["?"; T::INSERT_FIELDS.len()].join(", "));
                    self.items.len()
                ]
                .join(", "),
            );

            let values = self
                .items
                .iter()
                .flat_map(|item| T::get_insert_values(item).into_iter())
                .collect::<Vec<Value>>();

            Self::log_query_start(&sentence, &values);
            let inserted = tx
                .execute(&sentence, params_from_iter(values.iter()))
                .map_err(DatabaseError::Insert)?;
            Self::log_query_ending(inserted, "Inserted");

            Ok(inserted)
        }
    }
}
