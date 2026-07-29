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

    fn map_from_row(row: &crate::rusqlite::Row) -> Result<Self, crate::rusqlite::Error>;

    fn insert() -> InsertBuilder<Self> {
        InsertBuilder::new()
    }

    fn select() -> SelectBuilder<Self> {
        SelectBuilder::<Self>::new()
    }

    fn update() -> UpdateBuilder<Self> {
        UpdateBuilder::new()
    }

    fn delete() -> DeleteBuilder<Self> {
        DeleteBuilder::new()
    }

    fn get_values(&self) -> Vec<Value>;
}
