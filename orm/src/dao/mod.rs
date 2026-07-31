#![allow(dead_code)]

pub mod helpers;

use crate::dao::helpers::{
    querys::{
        QueryBuilder, delete::DeleteBuilder, insert::InsertBuilder, select::SelectBuilder,
        update::UpdateBuilder,
    },
    types::table_name::TableName,
};

use self::helpers::types::{column_name::ColumnName, value::Value};

pub trait Entity: Sized + 'static {
    const TABLE_NAME: &'static TableName<Self>;
    const FIELDS: &'static [ColumnName<Self>];

    fn get_values(&self) -> Vec<Value>;
    fn map_from_row(row: &crate::rusqlite::Row) -> Result<Self, crate::rusqlite::Error>;
}

pub trait Repository<T>: Sized + 'static
where
    T: Entity,
{
    fn delete() -> DeleteBuilder<T> {
        DeleteBuilder::new()
    }

    fn insert() -> InsertBuilder<T> {
        InsertBuilder::new()
    }

    fn select() -> SelectBuilder<T> {
        SelectBuilder::new()
    }

    fn update() -> UpdateBuilder<T> {
        UpdateBuilder::new()
    }
}
