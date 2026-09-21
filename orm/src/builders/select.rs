use std::fmt::Write;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::database::DatabasePool;
use crate::rusqlite::{Statement, params_from_iter};

use crate::{
    builders::{QueryBuilder, log_query_ending, log_query_start},
    dao::Entity,
    errors::DatabaseError,
    types::{
        column_name::{ColumnName, write_column_list}, order_by::OrderBy, row::Row, row::Rows, subquery::Subquery,
        value::Value, where_clause::Where,
    },
};

#[derive(Clone, Copy)]
pub struct Mappeable;

pub struct NonMappeable<T>
where
    T: Entity,
{
    columns: Vec<ColumnName<T>>,
    distinct: bool,
}

impl<T> Clone for NonMappeable<T>
where
    T: Entity,
{
    fn clone(&self) -> Self {
        Self {
            columns: self.columns.clone(),
            distinct: self.distinct,
        }
    }
}

/// What a select projects and how each result row is turned into a value:
/// [`Mappeable`] selects every column and maps rows to the entity, [`NonMappeable`]
/// selects the given columns and returns raw [`Row`]s.
pub trait ColumnsOf<T>
where
    T: Entity,
{
    type Output;

    fn write_columns(&self, out: &mut String);

    fn query(
        &self,
        stmt: &mut Statement,
        params: &[&Value],
    ) -> Result<Vec<Self::Output>, crate::rusqlite::Error>;
}

impl<T> ColumnsOf<T> for Mappeable
where
    T: Entity,
{
    type Output = T;

    fn write_columns(&self, out: &mut String) {
        write_column_list(out, T::FIELDS);
    }

    fn query(
        &self,
        stmt: &mut Statement,
        params: &[&Value],
    ) -> Result<Vec<T>, crate::rusqlite::Error> {
        stmt.query_map(params_from_iter(params.iter().copied()), T::map_from_row)?
            .collect()
    }
}

impl<T> ColumnsOf<T> for NonMappeable<T>
where
    T: Entity,
{
    type Output = Row;

    fn write_columns(&self, out: &mut String) {
        if self.distinct {
            out.push_str("DISTINCT ");
        }
        write_column_list(out, &self.columns);
    }

    fn query(
        &self,
        stmt: &mut Statement,
        params: &[&Value],
    ) -> Result<Rows, crate::rusqlite::Error> {
        let names: Arc<[String]> = stmt.column_names().into_iter().map(String::from).collect();
        stmt.query_map(params_from_iter(params.iter().copied()), |row| {
            Row::from_row(&names, row)
        })?
        .collect()
    }
}

pub struct SelectBuilder<T, K = Mappeable>
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

impl<T, K> Clone for SelectBuilder<T, K>
where
    T: Entity,
    K: Clone,
{
    fn clone(&self) -> Self {
        Self {
            kind: self.kind.clone(),
            condition: self.condition.clone(),
            order: self.order.clone(),
            limit: self.limit,
            offset: self.offset,
            _marker_entity: PhantomData,
        }
    }
}

impl<T> QueryBuilder<T> for SelectBuilder<T, Mappeable>
where
    T: Entity,
{
    fn new() -> Self {
        Self {
            kind: Mappeable,
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

    /// Appends ` WHERE ...` (if there is a condition) and collects its bound parameters.
    fn push_where<'a>(&'a self, sentence: &mut String, params: &mut Vec<&'a Value>) {
        if let Some(condition) = &self.condition {
            sentence.push_str(" WHERE ");
            condition.write_sql(sentence);
            condition.push_params(params);
        }
    }

    fn build_sql(&self, limit: Option<u32>) -> (String, Vec<&Value>) {
        let mut sentence = String::from("SELECT ");
        self.kind.write_columns(&mut sentence);
        let _ = write!(sentence, " FROM '{}'.'{}'", T::SCHEMA, T::TABLE_NAME);

        let mut params = Vec::new();
        self.push_where(&mut sentence, &mut params);

        for (i, order) in self.order.iter().enumerate() {
            sentence.push_str(if i == 0 { " ORDER BY " } else { ", " });
            order.write_sql(&mut sentence);
        }

        if let Some(limit) = limit {
            let _ = write!(sentence, " LIMIT {}", limit);
        }

        if let Some(offset) = self.offset {
            let _ = write!(sentence, " OFFSET {}", offset);
        }

        (sentence, params)
    }

    pub fn to_subquery(&self) -> Subquery {
        let (sql, params) = self.build_sql(self.limit);
        Subquery::new(sql, params.into_iter().cloned().collect())
    }

    pub fn count(&self, db: &DatabasePool) -> crate::errors::Result<i64> {
        db.run_in_connection(|conn| {
            let res = self.count_in(conn)?;
            Ok(res)
        })
    }

    pub fn count_in(&self, conn: &crate::rusqlite::Connection) -> crate::errors::Result<i64> {
        let mut sentence = format!("SELECT COUNT(*) FROM '{}'.'{}'", T::SCHEMA, T::TABLE_NAME);
        let mut params = Vec::new();
        self.push_where(&mut sentence, &mut params);

        log_query_start(&sentence, params.iter().copied());
        let total: i64 = conn
            .prepare_cached(&sentence)
            .and_then(|mut stmt| {
                stmt.query_row(params_from_iter(params.iter().copied()), |row| row.get(0))
            })
            .map_err(DatabaseError::Select)?;
        log_query_ending(total as usize, "Counted");

        Ok(total)
    }

    pub fn fetch_in(
        &self,
        conn: &crate::rusqlite::Connection,
    ) -> crate::errors::Result<Vec<K::Output>> {
        self.fetch_limited(conn, self.limit)
    }

    /// First matching row, if any. Runs with `LIMIT 1` so SQLite stops after one row.
    pub fn fetch_one_in(
        &self,
        conn: &crate::rusqlite::Connection,
    ) -> crate::errors::Result<Option<K::Output>> {
        let limit = self.limit.map_or(1, |limit| limit.min(1));
        Ok(self.fetch_limited(conn, Some(limit))?.into_iter().next())
    }

    pub fn fetch_one(&self, db: &DatabasePool) -> crate::errors::Result<Option<K::Output>> {
        db.run_in_connection(|conn| {
            let res = self.fetch_one_in(conn)?;
            Ok(res)
        })
    }

    fn fetch_limited(
        &self,
        conn: &crate::rusqlite::Connection,
        limit: Option<u32>,
    ) -> crate::errors::Result<Vec<K::Output>> {
        let (sentence, params) = self.build_sql(limit);

        log_query_start(&sentence, params.iter().copied());
        let mut stmt = conn
            .prepare_cached(&sentence)
            .map_err(DatabaseError::Select)?;
        let res = self
            .kind
            .query(&mut stmt, &params)
            .map_err(DatabaseError::Select)?;
        log_query_ending(res.len(), "Selected");

        Ok(res)
    }
}

impl<T> SelectBuilder<T, NonMappeable<T>>
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

impl<T> SelectBuilder<T, Mappeable>
where
    T: Entity,
{
    pub fn distinct(self, fields: &[ColumnName<T>]) -> SelectBuilder<T, NonMappeable<T>> {
        self.into_non_mappeable(fields, true)
    }

    pub fn columns(self, fields: &[ColumnName<T>]) -> SelectBuilder<T, NonMappeable<T>> {
        self.into_non_mappeable(fields, false)
    }

    fn into_non_mappeable(
        self,
        fields: &[ColumnName<T>],
        distinct: bool,
    ) -> SelectBuilder<T, NonMappeable<T>> {
        SelectBuilder {
            kind: NonMappeable {
                columns: fields.to_vec(),
                distinct,
            },
            condition: self.condition,
            order: self.order,
            limit: self.limit,
            offset: self.offset,
            _marker_entity: PhantomData,
        }
    }
}
