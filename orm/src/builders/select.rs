use std::marker::PhantomData;

use crate::database::DatabasePool;
use crate::rusqlite::params_from_iter;

use crate::{
    builders::QueryBuilder,
    dao::Entity,
    errors::DatabaseError,
    types::{
        column_name::ColumnName, order_by::OrderBy, subquery::Subquery, value::Value,
        where_clause::Where,
    },
};

pub struct Selectable;

pub struct NonSelectable<T>
where
    T: Entity,
{
    columns: Vec<ColumnName<T>>,
    distinct: bool,
}

pub trait ColumnsOf<T>
where
    T: Entity,
{
    fn columns(&self) -> String;
}

impl<T> ColumnsOf<T> for Selectable
where
    T: Entity,
{
    fn columns(&self) -> String {
        T::FIELDS
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<String>>()
            .join(", ")
    }
}

impl<T> ColumnsOf<T> for NonSelectable<T>
where
    T: Entity,
{
    fn columns(&self) -> String {
        let cols = self
            .columns
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<String>>()
            .join(", ");

        if self.distinct {
            format!("DISTINCT {}", cols)
        } else {
            cols
        }
    }
}

pub struct SelectBuilder<T, K = Selectable>
where
    T: Entity,
{
    kind: K,
    condition: Option<Where<T>>,
    order: Vec<OrderBy<T>>,
    limit: Option<u32>,
    offset: Option<u32>,
    _marker_entity: PhantomData<T>,
}

impl<T> QueryBuilder<T> for SelectBuilder<T, Selectable>
where
    T: Entity,
{
    fn new() -> Self {
        Self {
            kind: Selectable,
            condition: None,
            order: Vec::new(),
            limit: None,
            offset: None,
            _marker_entity: PhantomData,
        }
    }
}

impl<T, K> SelectBuilder<T, K>
where
    T: Entity,
    K: ColumnsOf<T>,
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

    fn build_sql(&self) -> (String, Vec<Value>) {
        let mut sentence = format!(
            "SELECT {} FROM {}.{}",
            self.kind.columns(),
            T::SCHEMA,
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

        (sentence, params)
    }

    pub fn to_subquery(&self) -> Subquery {
        let (sql, params) = self.build_sql();
        Subquery::new(sql, params)
    }
}

impl<T> SelectBuilder<T, NonSelectable<T>>
where
    T: Entity,
{
    pub fn distinct(mut self, fields: &[ColumnName<T>]) -> Self {
        self.kind.columns = fields.to_vec();
        self.kind.distinct = true;
        self
    }

    pub fn columns(mut self, fields: &[ColumnName<T>]) -> Self {
        self.kind.columns = fields.to_vec();
        self
    }
}

impl<T> SelectBuilder<T, Selectable>
where
    T: Entity,
{
    pub fn distinct(self, fields: &[ColumnName<T>]) -> SelectBuilder<T, NonSelectable<T>> {
        SelectBuilder {
            kind: NonSelectable {
                columns: fields.to_vec(),
                distinct: true,
            },
            condition: self.condition,
            order: self.order,
            limit: self.limit,
            offset: self.offset,
            _marker_entity: PhantomData,
        }
    }

    pub fn columns(self, fields: &[ColumnName<T>]) -> SelectBuilder<T, NonSelectable<T>> {
        SelectBuilder {
            kind: NonSelectable {
                columns: fields.to_vec(),
                distinct: false,
            },
            condition: self.condition,
            order: self.order,
            limit: self.limit,
            offset: self.offset,
            _marker_entity: PhantomData,
        }
    }

    pub fn fetch_in(&self, conn: &crate::rusqlite::Connection) -> crate::errors::Result<Vec<T>> {
        let (sentence, params) = self.build_sql();

        Self::log_query_start(&sentence, &params);
        let mut stmt = conn
            .prepare_cached(&sentence)
            .map_err(DatabaseError::Select)?;
        let rows = stmt
            .query_map(params_from_iter(params.iter()), T::map_from_row)
            .map_err(DatabaseError::Select)?;

        let res: Vec<T> = rows
            .collect::<Result<Vec<T>, crate::rusqlite::Error>>()
            .map_err(DatabaseError::Select)?;
        Self::log_query_ending(res.len(), "Selected");

        Ok(res)
    }

    pub fn fetch_one_in(
        &self,
        conn: &crate::rusqlite::Connection,
    ) -> crate::errors::Result<Option<T>> {
        let res = self.fetch_in(conn)?;
        Ok(res.into_iter().next())
    }

    pub fn fetch_one(&self, db: &DatabasePool) -> crate::errors::Result<Option<T>> {
        db.run_in_connection(|conn| {
            let res = self.fetch_one_in(conn)?;
            Ok(res)
        })
    }

    pub fn count(&self, db: &DatabasePool) -> crate::errors::Result<i64> {
        db.run_in_connection(|conn| {
            let res = self.count_in(conn)?;
            Ok(res)
        })
    }

    pub fn count_in(&self, conn: &crate::rusqlite::Connection) -> crate::errors::Result<i64> {
        let mut sentence = format!("SELECT COUNT(*) FROM {}.{}", T::SCHEMA, T::TABLE_NAME);

        let mut params = Vec::new();
        if let Some(condition) = &self.condition {
            sentence.push_str(&format!(" WHERE {}", condition.to_sql()));
            params = condition.clone().into_params();
        }

        Self::log_query_start(&sentence, &params);
        let total: i64 = conn
            .query_row(&sentence, params_from_iter(params), |row| row.get(0))
            .map_err(DatabaseError::Select)?;
        Self::log_query_ending(total as usize, "Counted");

        Ok(total)
    }
}
