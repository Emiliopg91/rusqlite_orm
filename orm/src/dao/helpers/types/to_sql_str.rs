use std::fmt::Debug;

use crate::rusqlite::ToSql;

pub trait ToSqlStr: ToSql + Debug {}
impl<T> ToSqlStr for T where T: ToSql + Debug + Clone + 'static {}
