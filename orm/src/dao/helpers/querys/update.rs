use crate::database::Database;
use crate::rusqlite::params_from_iter;

use crate::{
    dao::{
        Entity,
        helpers::{
            querys::QueryBuilder,
            types::{column_name::ColumnName, value::Value, where_clause::Where},
        },
    },
    database::errors::DatabaseError,
};

pub struct UpdateBuilder<T>
where
    T: Entity,
{
    condition: Option<Where<T>>,
    field_values: Vec<(ColumnName<T>, Value)>,
    _marker: std::marker::PhantomData<T>,
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

    pub fn execute(&self) -> crate::database::errors::Result<usize> {
        Database::run_in_connection(|conn| {
            let res = self.execute_in_conn(conn)?;
            Ok(res)
        })
    }

    pub fn execute_in_conn(
        &self,
        conn: &crate::rusqlite::Connection,
    ) -> crate::database::errors::Result<usize> {
        let mut sentence = format!("UPDATE {}.{} SET ", T::SCHEMA, T::TABLE_NAME);
        sentence.push_str(
            &self
                .field_values
                .iter()
                .map(|(f, _)| format!("{}=?", f))
                .collect::<Vec<String>>()
                .join(", "),
        );

        let mut cond_params: Vec<Value> = Vec::new();
        if let Some(cond) = &self.condition {
            sentence.push_str(&format!(" WHERE {}", cond.to_sql()));
            cond_params = <Where<T> as Clone>::clone(cond).into_params();
        }

        let mut params = self
            .field_values
            .iter()
            .map(|(_, v)| v)
            .cloned()
            .collect::<Vec<Value>>();
        params.extend(cond_params);

        Self::log_query_start(&sentence, &params);
        let updated = conn
            .execute(&sentence, params_from_iter(params))
            .map_err(DatabaseError::Update)?;
        Self::log_query_ending(updated, "Updated");

        Ok(updated)
    }
}
