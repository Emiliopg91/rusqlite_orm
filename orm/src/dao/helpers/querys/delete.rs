use std::marker::PhantomData;

use crate::rusqlite::params_from_iter;

use crate::{
    dao::{
        Entity,
        helpers::{
            querys::QueryBuilder,
            types::{value::Value, where_clause::Where},
        },
    },
    database::{DATABASE_INST, errors::DatabaseError},
};

pub struct DeleteBuilder<T>
where
    T: Entity,
{
    condition: Option<Where<T>>,
    _marker: PhantomData<T>,
}

impl<T> QueryBuilder<T> for DeleteBuilder<T>
where
    T: Entity,
{
    fn new() -> Self {
        DeleteBuilder {
            condition: None,
            _marker: PhantomData,
        }
    }
}

impl<T> DeleteBuilder<T>
where
    T: Entity,
{
    pub fn where_(mut self, condition: Where<T>) -> Self {
        self.condition = Some(condition);
        self
    }

    pub fn execute_in_tx(
        &self,
        tx: &crate::rusqlite::Transaction,
    ) -> crate::database::errors::Result<usize> {
        let mut sentence = format!("DELETE FROM {}.{} ", T::SCHEMA, T::TABLE_NAME);

        let params: Vec<Value> = Vec::new();
        if let Some(cond) = &self.condition {
            sentence.push_str(&format!(" WHERE {}", cond.to_sql()));
        }

        Self::log_query_start(&sentence, &params);
        let deleted = tx
            .execute(&sentence, params_from_iter(params))
            .map_err(DatabaseError::Delete)?;
        Self::log_query_ending(deleted, "Deleted");

        Ok(deleted)
    }

    pub fn execute(&self) -> crate::database::errors::Result<usize> {
        let mut db = DATABASE_INST.lock().unwrap();
        db.run_in_tx(|tx| Ok(self.execute_in_tx(tx)?))
    }
}
