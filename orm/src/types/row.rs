use std::sync::Arc;

use crate::types::value::Value;

/// A result row of a non-mapped select. The column names are shared (one `Arc` per
/// query, not per row) and values are stored positionally.
#[derive(Clone, Default)]
pub struct Row {
    names: Arc<[String]>,
    values: Vec<Value>,
}

impl Row {
    pub fn get(&self, column: &str) -> Option<&Value> {
        self.names
            .iter()
            .position(|name| name == column)
            .map(|idx| &self.values[idx])
    }

    pub(crate) fn from_row(
        names: &Arc<[String]>,
        row: &crate::rusqlite::Row,
    ) -> Result<Self, crate::rusqlite::Error> {
        let mut values = Vec::with_capacity(names.len());
        for idx in 0..names.len() {
            values.push(row.get_ref(idx)?.into());
        }
        Ok(Self {
            names: Arc::clone(names),
            values,
        })
    }
}

pub type Rows = Vec<Row>;
