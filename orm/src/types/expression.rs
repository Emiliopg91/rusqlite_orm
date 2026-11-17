use crate::dao::Entity;

use super::column_name::ColumnName;

/// SQL aggregate function applied to a column of a non-mapped select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Function {
    /// `COUNT(column)`: non-NULL values.
    Count,
    /// `MAX(column)`.
    Max,
    /// `MIN(column)`.
    Min,
    /// `SUM(column)`: NULL if every value is NULL.
    Sum,
    /// `AVG(column)`: always a real, NULL if every value is NULL.
    Avg,
}

impl Function {
    fn name(self) -> &'static str {
        match self {
            Function::Count => "COUNT",
            Function::Max => "MAX",
            Function::Min => "MIN",
            Function::Sum => "SUM",
            Function::Avg => "AVG",
        }
    }
}

/// One projected item of a non-mapped select: a column, or an aggregate function over one (or
/// `COUNT(*)`), optionally aliased.
///
/// Build it from a column with `.into()` or the shortcuts on [`ColumnName`] (`.max()`,
/// `.count()`, ...), or with [`Expression::count_all`]. In the returned [`Row`](super::row::Row)s
/// a column is read by its name, an aliased expression by its alias, and an unaliased function by
/// its SQL text (e.g. `"MAX(session)"`).
pub struct Expression<T>
where
    T: Entity,
{
    function: Option<Function>,
    /// `None` only for `COUNT(*)`.
    column: Option<ColumnName<T>>,
    alias: Option<&'static str>,
}

impl<T> Clone for Expression<T>
where
    T: Entity,
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Expression<T> where T: Entity {}

impl<T> From<ColumnName<T>> for Expression<T>
where
    T: Entity,
{
    fn from(column: ColumnName<T>) -> Self {
        Self {
            function: None,
            column: Some(column),
            alias: None,
        }
    }
}

impl<T> Expression<T>
where
    T: Entity,
{
    /// `function(column)`.
    pub const fn function(function: Function, column: ColumnName<T>) -> Self {
        Self {
            function: Some(function),
            column: Some(column),
            alias: None,
        }
    }

    /// `COUNT(*)`: rows, NULLs included.
    pub const fn count_all() -> Self {
        Self {
            function: Some(Function::Count),
            column: None,
            alias: None,
        }
    }

    /// Renders the expression as `... AS alias`, the name it is then read by from the rows.
    pub const fn alias(mut self, alias: &'static str) -> Self {
        self.alias = Some(alias);
        self
    }

    pub(crate) fn write_sql(&self, out: &mut String) {
        let column = self.column.as_ref().map_or("*", |c| c.as_ref());
        match self.function {
            Some(function) => {
                out.push_str(function.name());
                out.push('(');
                out.push_str(column);
                out.push(')');
            }
            None => out.push_str(column),
        }
        if let Some(alias) = self.alias {
            out.push_str(" AS ");
            out.push_str(alias);
        }
    }
}

impl<T> ColumnName<T>
where
    T: Entity,
{
    /// `COUNT(column)`: non-NULL values.
    pub const fn count(self) -> Expression<T> {
        Expression::function(Function::Count, self)
    }

    /// `MAX(column)`.
    pub const fn max(self) -> Expression<T> {
        Expression::function(Function::Max, self)
    }

    /// `MIN(column)`.
    pub const fn min(self) -> Expression<T> {
        Expression::function(Function::Min, self)
    }

    /// `SUM(column)`.
    pub const fn sum(self) -> Expression<T> {
        Expression::function(Function::Sum, self)
    }

    /// `AVG(column)`.
    pub const fn avg(self) -> Expression<T> {
        Expression::function(Function::Avg, self)
    }
}

/// Writes `a, MAX(b), ...` into `out`.
pub(crate) fn write_expression_list<T: Entity>(out: &mut String, expressions: &[Expression<T>]) {
    for (i, expression) in expressions.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        expression.write_sql(out);
    }
}
