use crate::database::DatabasePool;
use crate::rusqlite::params_from_iter;

use crate::{
    builders::{QueryBuilder, log_query_ending, log_query_start},
    dao::Entity,
    errors::DatabaseError,
    types::{column_name::ColumnName, value::Value, where_clause::Where},
};

pub struct UpdateBuilder<T>
where
    T: Entity,
{
    condition: Option<Where<T>>,
    field_values: Vec<(ColumnName<T>, Value)>,
    _marker: std::marker::PhantomData<T>,
}

impl<T> Clone for UpdateBuilder<T>
where
    T: Entity,
{
    fn clone(&self) -> Self {
        Self {
            condition: self.condition.clone(),
            field_values: self.field_values.clone(),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T> QueryBuilder<T> for UpdateBuilder<T>
where
    T: Entity,
{
    fn new() -> Self {
        Self {
            condition: None,
            field_values: Vec::new(),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T> UpdateBuilder<T>
where
    T: Entity,
{
    pub fn where_(mut self, condition: Where<T>) -> Self {
        self.condition = Some(condition);
        self
    }

    pub fn set(mut self, field: ColumnName<T>, value: Value) -> Self {
        self.field_values.push((field, value));
        self
    }

    /// Runs on a pooled connection: a single statement is atomic on its own.
    pub async fn execute(&self, db: &DatabasePool) -> crate::errors::Result<usize> {
        let builder = self.clone();
        db.run_sync_in_connection(move |conn| Ok(builder.execute_sync(conn)?))
            .await
    }

    /// Runs on the given connection, which may be a `Transaction` through `Deref`.
    pub async fn execute_in(
        &self,
        conn: &crate::rusqlite::Connection,
    ) -> crate::errors::Result<usize> {
        self.execute_sync(conn)
    }

    fn execute_sync(&self, conn: &crate::rusqlite::Connection) -> crate::errors::Result<usize> {
        let mut sentence = format!("UPDATE '{}'.'{}' SET ", T::SCHEMA, T::TABLE_NAME);
        for (i, (field, _)) in self.field_values.iter().enumerate() {
            if i > 0 {
                sentence.push_str(", ");
            }
            sentence.push_str(field.as_ref());
            sentence.push_str("=?");
        }

        let mut params: Vec<&Value> = self.field_values.iter().map(|(_, v)| v).collect();
        if let Some(cond) = &self.condition {
            sentence.push_str(" WHERE ");
            cond.write_sql(&mut sentence);
            cond.push_params(&mut params);
        }

        log_query_start(&sentence, params.iter().copied());
        let updated = conn
            .prepare_cached(&sentence)
            .and_then(|mut stmt| stmt.execute(params_from_iter(params.iter().copied())))
            .map_err(DatabaseError::Update)?;
        log_query_ending(updated, "Updated");

        Ok(updated)
    }
}
