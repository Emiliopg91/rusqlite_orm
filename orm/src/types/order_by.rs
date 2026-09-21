use crate::dao::Entity;

use super::column_name::ColumnName;

pub enum OrderBy<T>
where
    T: Entity,
{
    Asc(ColumnName<T>),
    Desc(ColumnName<T>),
}

impl<T> Clone for OrderBy<T>
where
    T: Entity,
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for OrderBy<T> where T: Entity {}

impl<T> OrderBy<T>
where
    T: Entity,
{
    pub fn to_sql(self) -> String {
        let mut out = String::new();
        self.write_sql(&mut out);
        out
    }

    pub(crate) fn write_sql(self, out: &mut String) {
        let (col, direction) = match self {
            OrderBy::Asc(col) => (col, " ASC"),
            OrderBy::Desc(col) => (col, " DESC"),
        };
        out.push_str(col.as_ref());
        out.push_str(direction);
    }
}
