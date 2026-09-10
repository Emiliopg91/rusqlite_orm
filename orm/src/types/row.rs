use std::collections::HashMap;

use crate::types::value::Value;

#[derive(Clone, Default)]
pub struct Row {
    columns: HashMap<String, Value>,
}

impl Row {
    pub fn get(&self, column: &str) -> Option<&Value> {
        self.columns.get(column)
    }

    pub(crate) fn from_row(row: &crate::rusqlite::Row) -> Result<Self, crate::rusqlite::Error> {
        let mut columns = HashMap::with_capacity(row.as_ref().column_count());
        for name in row.as_ref().column_names() {
            columns.insert(name.to_string(), row.get_ref(name)?.into());
        }
        Ok(Self { columns })
    }
}

pub type Rows = Vec<Row>;
