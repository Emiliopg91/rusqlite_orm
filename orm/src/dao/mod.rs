#![allow(dead_code)]

use crate::{
    builders::{
        QueryBuilder, delete::DeleteBuilder, insert::InsertBuilder, select::SelectBuilder,
        update::UpdateBuilder,
    },
    types::{column_name::ColumnName, schema::Schema, table_name::TableName, value::Value},
};

pub trait Entity: Sized + Send + Sync + 'static {
    const SCHEMA: &'static Schema<Self>;
    const TABLE_NAME: &'static TableName<Self>;
    const FIELDS: &'static [ColumnName<Self>];
    const INSERT_FIELDS: &'static [ColumnName<Self>];
    const AUTOINCREMENT_FIELD: bool;

    type Repository;

    fn get_insert_values(&self) -> Vec<Value>;
    fn map_from_row(row: &crate::rusqlite::Row) -> Result<Self, crate::rusqlite::Error>;
    fn after_map_from_row() -> crate::errors::Result<()> {
        Ok(())
    }
    fn set_autoincrement_id(&mut self, _: i64) {}
    /// Whether `id` fits the type of the autoincrement field; an insert assigned an id that
    /// doesn't fails instead of writing back a truncated one.
    fn autoincrement_id_fits(_: i64) -> bool {
        true
    }
}

pub trait Repository<T>: Sized + 'static
where
    T: Entity,
{
    fn delete() -> DeleteBuilder<T> {
        DeleteBuilder::new()
    }

    fn insert() -> InsertBuilder<'static, T> {
        InsertBuilder::new()
    }

    fn select() -> SelectBuilder<T> {
        SelectBuilder::new()
    }

    fn update() -> UpdateBuilder<T> {
        UpdateBuilder::new()
    }
}
