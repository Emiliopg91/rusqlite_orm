use crate::{dao::Entity, types::value::Value};

use super::column_name::{ColumnName, write_column_list};
use super::subquery::Subquery;

pub enum Where<T>
where
    T: Entity,
{
    Eq(ColumnName<T>, Value),
    NotEq(ColumnName<T>, Value),
    Gt(ColumnName<T>, Value),
    Gte(ColumnName<T>, Value),
    Lt(ColumnName<T>, Value),
    Lte(ColumnName<T>, Value),
    In(ColumnName<T>, Vec<Value>),
    InMultiple(Vec<ColumnName<T>>, Vec<Vec<Value>>),
    Null(ColumnName<T>),
    NotNull(ColumnName<T>),
    EqSub(ColumnName<T>, Subquery),
    NotEqSub(ColumnName<T>, Subquery),
    InSub(ColumnName<T>, Subquery),
    InMultipleSub(Vec<ColumnName<T>>, Subquery),
    NotInSub(ColumnName<T>, Subquery),
    And(Vec<Where<T>>),
    Or(Vec<Where<T>>),
}

fn write_token(out: &mut String, val: &Value) {
    match val {
        Value::Raw(sql) => out.push_str(sql),
        _ => out.push('?'),
    }
}

fn write_joined<T: Entity>(out: &mut String, conditions: &[Where<T>], separator: &str) {
    for (i, condition) in conditions.iter().enumerate() {
        if i > 0 {
            out.push_str(separator);
        }
        match condition {
            Where::And(_) | Where::Or(_) => {
                out.push('(');
                condition.write_sql(out);
                out.push(')');
            }
            _ => condition.write_sql(out),
        }
    }
}

impl<T> Where<T>
where
    T: Entity,
{
    pub fn to_sql(&self) -> String {
        let mut out = String::new();
        self.write_sql(&mut out);
        out
    }

    /// Renders this condition into `out`, without intermediate allocations.
    pub(crate) fn write_sql(&self, out: &mut String) {
        let comparison = |out: &mut String, col: &ColumnName<T>, op: &str, val: &Value| {
            out.push_str(col.as_ref());
            out.push_str(op);
            write_token(out, val);
        };

        match self {
            Self::Eq(col, val) => comparison(out, col, "=", val),
            Self::NotEq(col, val) => comparison(out, col, "!=", val),
            Self::Gt(col, val) => comparison(out, col, ">", val),
            Self::Gte(col, val) => comparison(out, col, ">=", val),
            Self::Lt(col, val) => comparison(out, col, "<", val),
            Self::Lte(col, val) => comparison(out, col, "<=", val),
            Self::In(col, values) => {
                out.push_str(col.as_ref());
                out.push_str(" IN (");
                for (i, val) in values.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    write_token(out, val);
                }
                out.push(')');
            }
            Self::InMultiple(cols, rows) => {
                out.push('(');
                write_column_list(out, cols);
                out.push_str(") IN (");
                for (i, row) in rows.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push('(');
                    for (j, val) in row.iter().enumerate() {
                        if j > 0 {
                            out.push_str(", ");
                        }
                        write_token(out, val);
                    }
                    out.push(')');
                }
                out.push(')');
            }
            Self::Null(col) => {
                out.push_str(col.as_ref());
                out.push_str(" IS NULL");
            }
            Self::NotNull(col) => {
                out.push_str(col.as_ref());
                out.push_str(" IS NOT NULL");
            }
            Self::EqSub(col, sub) => {
                out.push_str(col.as_ref());
                out.push_str("=(");
                out.push_str(&sub.sql);
                out.push(')');
            }
            Self::NotEqSub(col, sub) => {
                out.push_str(col.as_ref());
                out.push_str("!=(");
                out.push_str(&sub.sql);
                out.push(')');
            }
            Self::InSub(col, sub) => {
                out.push_str(col.as_ref());
                out.push_str(" IN (");
                out.push_str(&sub.sql);
                out.push(')');
            }
            Self::NotInSub(col, sub) => {
                out.push_str(col.as_ref());
                out.push_str(" NOT IN (");
                out.push_str(&sub.sql);
                out.push(')');
            }
            Self::InMultipleSub(cols, sub) => {
                out.push('(');
                write_column_list(out, cols);
                out.push_str(") IN (");
                out.push_str(&sub.sql);
                out.push(')');
            }
            Self::And(conditions) => write_joined(out, conditions, " AND "),
            Self::Or(conditions) => write_joined(out, conditions, " OR "),
        }
    }

    /// Bound parameters of this condition, in the order their `?` appear in
    /// [`Where::to_sql`] (`Value::Raw` is spliced into the SQL, so it is skipped).
    pub(crate) fn params(&self) -> Vec<&Value> {
        let mut out = Vec::new();
        self.push_params(&mut out);
        out
    }

    pub(crate) fn push_params<'a>(&'a self, out: &mut Vec<&'a Value>) {
        match self {
            Self::Eq(_, val)
            | Self::NotEq(_, val)
            | Self::Gt(_, val)
            | Self::Gte(_, val)
            | Self::Lt(_, val)
            | Self::Lte(_, val) => {
                if !matches!(val, Value::Raw(_)) {
                    out.push(val);
                }
            }
            Self::In(_, vals) => {
                out.extend(vals.iter().filter(|val| !matches!(val, Value::Raw(_))));
            }
            Self::InMultiple(_, rows) => {
                for row in rows {
                    out.extend(row.iter().filter(|val| !matches!(val, Value::Raw(_))));
                }
            }
            Self::Null(_) | Self::NotNull(_) => {}
            Self::And(conditions) | Self::Or(conditions) => {
                for condition in conditions {
                    condition.push_params(out);
                }
            }
            Self::EqSub(_, sub)
            | Self::NotEqSub(_, sub)
            | Self::InSub(_, sub)
            | Self::NotInSub(_, sub)
            | Self::InMultipleSub(_, sub) => out.extend(sub.params.iter()),
        }
    }

    pub fn into_params(self) -> Vec<Value> {
        self.params().into_iter().cloned().collect()
    }
}

impl<T> Clone for Where<T>
where
    T: Entity,
{
    fn clone(&self) -> Self {
        match self {
            Self::Eq(col, val) => Self::Eq(*col, val.clone()),
            Self::NotEq(col, val) => Self::NotEq(*col, val.clone()),
            Self::Gt(col, val) => Self::Gt(*col, val.clone()),
            Self::Gte(col, val) => Self::Gte(*col, val.clone()),
            Self::Lt(col, val) => Self::Lt(*col, val.clone()),
            Self::Lte(col, val) => Self::Lte(*col, val.clone()),
            Self::In(col, vals) => Self::In(*col, vals.clone()),
            Self::InMultiple(cols, vals) => Self::InMultiple(cols.clone(), vals.clone()),
            Self::Null(col) => Self::Null(*col),
            Self::NotNull(col) => Self::NotNull(*col),
            Self::EqSub(col, sub) => Self::EqSub(*col, sub.clone()),
            Self::NotEqSub(col, sub) => Self::NotEqSub(*col, sub.clone()),
            Self::InSub(col, sub) => Self::InSub(*col, sub.clone()),
            Self::InMultipleSub(col, sub) => Self::InMultipleSub(col.clone(), sub.clone()),
            Self::NotInSub(col, sub) => Self::NotInSub(*col, sub.clone()),
            Self::And(conds) => Self::And(conds.clone()),
            Self::Or(conds) => Self::Or(conds.clone()),
        }
    }
}
