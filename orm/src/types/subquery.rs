use super::value::Value;

/// A fully rendered SELECT statement (SQL text plus bound parameters) meant to be
/// embedded inside another statement's WHERE clause, e.g. `col IN (SELECT ...)`.
#[derive(Clone)]
pub struct Subquery {
    pub(crate) sql: String,
    pub(crate) params: Vec<Value>,
}

impl Subquery {
    pub fn new(sql: String, params: Vec<Value>) -> Self {
        Self { sql, params }
    }
}
