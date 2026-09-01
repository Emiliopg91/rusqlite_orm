use std::marker::PhantomData;

use crate::dao::Entity;

#[repr(transparent)]
pub struct TableName<T>(&'static str, PhantomData<T>)
where
    T: Entity;

impl<T> Clone for TableName<T>
where
    T: Entity,
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for TableName<T> where T: Entity {}

impl<T> AsRef<str> for TableName<T>
where
    T: Entity,
{
    fn as_ref(&self) -> &str {
        self.0
    }
}

impl<T> std::fmt::Display for TableName<T>
where
    T: Entity,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<T> TableName<T>
where
    T: Entity,
{
    pub const fn new(value: &'static str) -> Self {
        Self(value, PhantomData)
    }
}
