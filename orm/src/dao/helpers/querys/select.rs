use std::marker::PhantomData;

use crate::rusqlite::params_from_iter;

use crate::{
    dao::{
        Entity,
        helpers::{
            querys::QueryBuilder,
            types::{order_by::OrderBy, where_clause::Where},
        },
    },
    database::{DATABASE_INST, errors::DatabaseError},
};

pub struct SelectBuilder<T>
where
    T: Entity,
{
    condition: Option<Where<T>>,
    order: Vec<OrderBy<T>>,
    limit: Option<u32>,
    offset: Option<u32>,
    _marker: PhantomData<T>,
}

impl<T> QueryBuilder<T> for SelectBuilder<T>
where
    T: Entity,
{
    fn new() -> Self {
        Self {
            condition: None,
            order: Vec::new(),
            limit: None,
            offset: None,
            _marker: PhantomData,
        }
    }
}

impl<T> SelectBuilder<T>
where
    T: Entity,
{
    pub fn where_(mut self, condition: Where<T>) -> Self {
        self.condition = Some(condition);
        self
    }

    pub fn order_by(mut self, order: OrderBy<T>) -> Self {
        self.order.push(order);
        self
    }

    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn offset(mut self, offset: u32) -> Self {
        self.offset = Some(offset);
        self
    }

    pub fn fetch_in_tx(
        &self,
        tx: &crate::rusqlite::Transaction,
    ) -> crate::database::errors::Result<Vec<T>> {
        let mut sentence = format!(
            "SELECT {} FROM {}",
            T::FIELDS
                .iter()
                .map(|f| f.as_ref().to_string())
                .collect::<Vec<String>>()
                .join(", "),
            T::TABLE_NAME
        );

        let mut params = Vec::new();
        if let Some(condition) = &self.condition {
            sentence.push_str(&format!(" WHERE {}", condition.to_sql()));
            params = condition.clone().into_params();
        }

        if !self.order.is_empty() {
            sentence.push_str(" ORDER BY ");
            sentence.push_str(
                &self
                    .order
                    .iter()
                    .map(|o| o.to_sql())
                    .collect::<Vec<String>>()
                    .join(", "),
            );
        }

        if let Some(limit) = self.limit {
            sentence.push_str(&format!(" LIMIT {}", limit));
        }

        if let Some(offset) = self.offset {
            sentence.push_str(&format!(" OFFSET {}", offset));
        }

        Self::log_query_start(&sentence, &params);
        let mut stmt = tx.prepare(&sentence).map_err(DatabaseError::Select)?;
        let rows = stmt
            .query_map(params_from_iter(params.iter()), T::map_from_row)
            .map_err(DatabaseError::Select)?;

        let res: Vec<T> = rows
            .collect::<Result<Vec<T>, crate::rusqlite::Error>>()
            .map_err(DatabaseError::Select)?;
        Self::log_query_ending(res.len(), "Selected");

        Ok(res)
    }

    pub fn fetch(&self) -> crate::database::errors::Result<Vec<T>> {
        let mut db = DATABASE_INST.lock().unwrap();
        db.run_in_tx(|tx| self.fetch_in_tx(tx))
    }

    pub fn fetch_one_in_tx(
        &self,
        tx: &crate::rusqlite::Transaction,
    ) -> crate::database::errors::Result<Option<T>> {
        Ok(self.fetch_in_tx(tx)?.into_iter().next())
    }

    pub fn fetch_one(&self) -> crate::database::errors::Result<Option<T>> {
        let mut db = DATABASE_INST.lock().unwrap();
        let res = db.run_in_tx(|tx| self.fetch_in_tx(tx))?;
        Ok(res.into_iter().next())
    }

    pub fn count_in_tx(
        &self,
        tx: &crate::rusqlite::Transaction,
    ) -> crate::database::errors::Result<i64> {
        let mut sentence = format!("SELECT COUNT(*) FROM {}", T::TABLE_NAME);

        let mut params = Vec::new();
        if let Some(condition) = &self.condition {
            sentence.push_str(&format!(" WHERE {}", condition.to_sql()));
            params = condition.clone().into_params();
        }

        Self::log_query_start(&sentence, &params);
        let total: i64 = tx
            .query_row(&sentence, params_from_iter(params), |row| row.get(0))
            .map_err(DatabaseError::Select)?;
        Self::log_query_ending(1, "Counted ");

        Ok(total)
    }

    pub fn count(&self) -> crate::database::errors::Result<i64> {
        let mut db = DATABASE_INST.lock().unwrap();
        db.run_in_tx(|tx| self.count_in_tx(tx))
    }
}
