use std::marker::PhantomData;

use crate::dao::Entity;

#[repr(transparent)]
pub struct Schema<T>(&'static str, PhantomData<T>)
where
    T: Entity;

impl<T> Clone for Schema<T>
where
    T: Entity,
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Schema<T> where T: Entity {}

impl<T> AsRef<str> for Schema<T>
where
    T: Entity,
{
    fn as_ref(&self) -> &str {
        self.0
    }
}

impl<T> std::fmt::Display for Schema<T>
where
    T: Entity,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<T> Schema<T>
where
    T: Entity,
{
    pub const fn new(value: &'static str) -> Self {
        Self(value, PhantomData)
    }
}
