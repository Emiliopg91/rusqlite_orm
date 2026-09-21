use std::marker::PhantomData;

use crate::database::DatabasePool;
use crate::rusqlite::params_from_iter;

use crate::{
    builders::{QueryBuilder, log_query_ending, log_query_start},
    dao::Entity,
    errors::DatabaseError,
    types::{value::Value, where_clause::Where},
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

    pub fn execute(&self, db: &DatabasePool) -> crate::errors::Result<usize> {
        db.run_in_transaction(|tx| {
            let res = self.execute_in(tx)?;
            Ok(res)
        })
    }

    pub fn execute_in(&self, tx: &crate::rusqlite::Transaction) -> crate::errors::Result<usize> {
        let mut sentence = format!("DELETE FROM '{}'.'{}'", T::SCHEMA, T::TABLE_NAME);

        let mut params: Vec<&Value> = Vec::new();
        if let Some(cond) = &self.condition {
            sentence.push_str(" WHERE ");
            cond.write_sql(&mut sentence);
            cond.push_params(&mut params);
        }

        log_query_start(&sentence, params.iter().copied());
        let deleted = tx
            .prepare_cached(&sentence)
            .and_then(|mut stmt| stmt.execute(params_from_iter(params.iter().copied())))
            .map_err(DatabaseError::Delete)?;
        log_query_ending(deleted, "Deleted");

        Ok(deleted)
    }
}
