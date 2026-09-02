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
pub struct NonSelectable;

pub struct SelectBuilder<T, K = Selectable>
where
    T: Entity,
{
    columns: Option<Vec<ColumnName<T>>>,
    condition: Option<Where<T>>,
    order: Vec<OrderBy<T>>,
    limit: Option<u32>,
    offset: Option<u32>,
    distinct: Option<Vec<ColumnName<T>>>,
    _marker_entity: PhantomData<T>,
    _marker_kind: PhantomData<K>,
}

impl<T> QueryBuilder<T> for SelectBuilder<T, Selectable>
where
    T: Entity,
{
    fn new() -> Self {
        Self {
            columns: None,
            condition: None,
            order: Vec::new(),
            limit: None,
            offset: None,
            distinct: None,
            _marker_entity: PhantomData,
            _marker_kind: PhantomData,
        }
    }
}

impl<T> SelectBuilder<T, NonSelectable>
where
    T: Entity,
{
    fn new(prev: SelectBuilder<T, Selectable>, fields: &[ColumnName<T>]) -> Self {
        Self {
            columns: prev.columns,
            condition: prev.condition,
            order: prev.order,
            limit: prev.limit,
            offset: prev.offset,
            distinct: Some(fields.to_vec()),
            _marker_entity: PhantomData,
            _marker_kind: PhantomData,
        }
    }
}

impl<T, K> SelectBuilder<T, K>
where
    T: Entity,
{
    pub fn where_(mut self, condition: Where<T>) -> Self {
        self.condition = Some(condition);
        self
    }

    /// Restricts the SELECT projection to a single column. Intended for building
    /// scalar/list subqueries via [`Self::to_subquery`] (e.g. `col IN (SELECT ...)`).
    pub fn column(mut self, column: ColumnName<T>) -> Self {
        self.columns = Some(vec![column]);
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
        let fields = match &self.distinct {
            Some(fields) => fields,
            None => match &self.columns {
                Some(cols) => cols,
                None => T::FIELDS,
            },
        }
        .iter()
        .map(|c| c.to_string())
        .collect::<Vec<String>>()
        .join(", ");

        let mut sentence = format!("SELECT {} FROM {}.{}", fields, T::SCHEMA, T::TABLE_NAME);

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

    /// Renders this SELECT as a [`Subquery`] so it can be embedded in another
    /// entity's `Where` condition, e.g. `Where::InSub(Other::COL, builder.column(Foo::ID).to_subquery())`.
    pub fn to_subquery(&self) -> Subquery {
        let (sql, params) = self.build_sql();
        Subquery::new(sql, params)
    }
}

impl<T> SelectBuilder<T, Selectable>
where
    T: Entity,
{
    pub fn distinct(self, fields: &[ColumnName<T>]) -> SelectBuilder<T, NonSelectable> {
        SelectBuilder::<T, NonSelectable>::new(self, fields)
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
