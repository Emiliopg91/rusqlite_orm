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
        column_name::{ColumnName, write_column_list},
        expression::{Expression, write_expression_list},
        order_by::OrderBy,
        row::Row,
        row::Rows,
        subquery::Subquery,
        value::Value,
        where_clause::Where,
    },
};

#[derive(Clone, Copy)]
pub struct Mappable;

pub struct NonMappable<T>
where
    T: Entity,
{
    columns: Vec<Expression<T>>,
    distinct: bool,
    group_by: Vec<ColumnName<T>>,
}

impl<T> Clone for NonMappable<T>
where
    T: Entity,
{
    fn clone(&self) -> Self {
        Self {
            columns: self.columns.clone(),
            distinct: self.distinct,
            group_by: self.group_by.clone(),
        }
    }
}

/// What a select projects and how each result row is turned into a value:
/// [`Mappable`] selects every column and maps rows to the entity, [`NonMappable`]
/// selects the given columns and returns raw [`Row`]s.
pub trait ColumnsOf<T>
where
    T: Entity,
{
    type Output;

    fn write_columns(&self, out: &mut String);

    /// Appends ` GROUP BY ...`, if the select groups its rows.
    fn write_group_by(&self, _out: &mut String) {}

    /// Whether the select groups its rows, so counting it means counting the groups.
    fn is_grouped(&self) -> bool {
        false
    }

    fn query(
        &self,
        stmt: &mut Statement,
        params: &[&Value],
    ) -> Result<Vec<Self::Output>, crate::rusqlite::Error>;
}

impl<T> ColumnsOf<T> for Mappable
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

impl<T> ColumnsOf<T> for NonMappable<T>
where
    T: Entity,
{
    type Output = Row;

    fn write_columns(&self, out: &mut String) {
        if self.distinct {
            out.push_str("DISTINCT ");
        }
        write_expression_list(out, &self.columns);
    }

    fn write_group_by(&self, out: &mut String) {
        if self.is_grouped() {
            out.push_str(" GROUP BY ");
            write_column_list(out, &self.group_by);
        }
    }

    fn is_grouped(&self) -> bool {
        !self.group_by.is_empty()
    }

    fn query(
        &self,
        stmt: &mut Statement,
        params: &[&Value],
    ) -> Result<Rows, crate::rusqlite::Error> {
        let names: Arc<[String]> = stmt.column_names().map(String::from).collect();
        stmt.query_map(params_from_iter(params.iter().copied()), |row| {
            Row::from_row(&names, row)
        })?
        .collect()
    }
}

pub struct SelectBuilder<T, K = Mappable>
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

impl<T> QueryBuilder<T> for SelectBuilder<T, Mappable>
where
    T: Entity,
{
    fn new() -> Self {
        Self {
            kind: Mappable,
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

    /// `SELECT ... FROM ... [WHERE ...] [GROUP BY ...]`, with its bound parameters.
    fn build_grouped_sql(&self) -> (String, Vec<&Value>) {
        let mut sentence = String::from("SELECT ");
        self.kind.write_columns(&mut sentence);
        let _ = write!(sentence, " FROM '{}'.'{}'", T::SCHEMA, T::TABLE_NAME);

        let mut params = Vec::new();
        self.push_where(&mut sentence, &mut params);
        self.kind.write_group_by(&mut sentence);

        (sentence, params)
    }

    fn build_sql(&self, limit: Option<u32>) -> (String, Vec<&Value>) {
        let (mut sentence, params) = self.build_grouped_sql();

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

    pub async fn count(&self, db: &DatabasePool) -> crate::errors::Result<i64>
    where
        K: Clone + Send + 'static,
    {
        let builder = self.clone();
        db.run_sync_in_connection(move |conn| Ok(builder.count_sync(conn)?))
            .await
    }

    pub async fn count_in(&self, conn: &crate::rusqlite::Connection) -> crate::errors::Result<i64> {
        self.count_sync(conn)
    }

    /// Matching rows, or groups if the select groups them. Ordering, limit and offset are ignored.
    fn count_sync(&self, conn: &crate::rusqlite::Connection) -> crate::errors::Result<i64> {
        let (sentence, params) = if self.kind.is_grouped() {
            let (grouped, params) = self.build_grouped_sql();
            (format!("SELECT COUNT(*) FROM ({grouped})"), params)
        } else {
            let mut sentence = format!("SELECT COUNT(*) FROM '{}'.'{}'", T::SCHEMA, T::TABLE_NAME);
            let mut params = Vec::new();
            self.push_where(&mut sentence, &mut params);
            (sentence, params)
        };

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

    pub async fn fetch_in(
        &self,
        conn: &crate::rusqlite::Connection,
    ) -> crate::errors::Result<Vec<K::Output>> {
        self.fetch_limited(conn, self.limit)
    }

    pub async fn fetch(&self, db: &DatabasePool) -> crate::errors::Result<Vec<K::Output>>
    where
        K: Clone + Send + 'static,
        K::Output: Send + 'static,
    {
        let builder = self.clone();
        db.run_sync_in_connection(move |conn| Ok(builder.fetch_limited(conn, builder.limit)?))
            .await
    }

    /// First matching row, if any. Runs with `LIMIT 1` so SQLite stops after one row.
    pub async fn fetch_one_in(
        &self,
        conn: &crate::rusqlite::Connection,
    ) -> crate::errors::Result<Option<K::Output>> {
        self.fetch_one_sync(conn)
    }

    fn fetch_one_sync(
        &self,
        conn: &crate::rusqlite::Connection,
    ) -> crate::errors::Result<Option<K::Output>> {
        let limit = self.limit.map_or(1, |limit| limit.min(1));
        Ok(self.fetch_limited(conn, Some(limit))?.into_iter().next())
    }

    pub async fn fetch_one(&self, db: &DatabasePool) -> crate::errors::Result<Option<K::Output>>
    where
        K: Clone + Send + 'static,
        K::Output: Send + 'static,
    {
        let builder = self.clone();
        db.run_sync_in_connection(move |conn| Ok(builder.fetch_one_sync(conn)?))
            .await
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

/// Converts the projected items given to `columns`/`distinct`, columns or [`Expression`]s.
fn to_expressions<T, C>(fields: &[C]) -> Vec<Expression<T>>
where
    T: Entity,
    C: Into<Expression<T>> + Copy,
{
    fields.iter().map(|field| (*field).into()).collect()
}

impl<T> SelectBuilder<T, NonMappable<T>>
where
    T: Entity,
{
    /// Selects `DISTINCT` the given columns or [`Expression`]s.
    pub fn distinct<C>(mut self, fields: &[C]) -> Self
    where
        C: Into<Expression<T>> + Copy,
    {
        self.kind.columns = to_expressions(fields);
        self.kind.distinct = true;
        self
    }

    /// Selects the given columns or [`Expression`]s (aggregate functions, aliases).
    pub fn columns<C>(mut self, fields: &[C]) -> Self
    where
        C: Into<Expression<T>> + Copy,
    {
        self.kind.columns = to_expressions(fields);
        self
    }

    /// Groups the rows by the given columns, adding ` GROUP BY ...` after the `WHERE` clause. Pair
    /// it with aggregate [`Expression`]s in `columns` (e.g. `.max()`, `.count()`).
    pub fn group_by(mut self, fields: &[ColumnName<T>]) -> Self {
        self.kind.group_by = fields.to_vec();
        self
    }
}

impl<T> SelectBuilder<T, Mappable>
where
    T: Entity,
{
    /// Selects `DISTINCT` the given columns or [`Expression`]s, returning [`Row`]s.
    pub fn distinct<C>(self, fields: &[C]) -> SelectBuilder<T, NonMappable<T>>
    where
        C: Into<Expression<T>> + Copy,
    {
        self.into_non_mappeable(to_expressions(fields), true)
    }

    /// Selects the given columns or [`Expression`]s (aggregate functions, aliases), returning
    /// [`Row`]s.
    pub fn columns<C>(self, fields: &[C]) -> SelectBuilder<T, NonMappable<T>>
    where
        C: Into<Expression<T>> + Copy,
    {
        self.into_non_mappeable(to_expressions(fields), false)
    }

    fn into_non_mappeable(
        self,
        columns: Vec<Expression<T>>,
        distinct: bool,
    ) -> SelectBuilder<T, NonMappable<T>> {
        SelectBuilder {
            kind: NonMappable {
                columns,
                distinct,
                group_by: Vec::new(),
            },
            condition: self.condition,
            order: self.order,
            limit: self.limit,
            offset: self.offset,
            _marker_entity: PhantomData,
        }
    }
}
