pub mod delete;
pub mod insert;
pub mod select;
pub mod update;

use log::{Level, debug, log_enabled};

use super::types::value::Value;

pub trait QueryBuilder<T> {
    fn new() -> Self;
}

/// Logs the statement with its parameters interpolated. The interpolation is skipped
/// entirely when `debug` logging is disabled, since it is O(statement + params).
pub(crate) fn log_query_start<'a>(sql: &str, params: impl IntoIterator<Item = &'a Value>) {
    if !log_enabled!(Level::Debug) {
        return;
    }

    let mut params = params.into_iter();
    let mut sentence = String::with_capacity(sql.len());
    for ch in sql.chars() {
        if ch != '?' {
            sentence.push(ch);
            continue;
        }
        match params.next() {
            Some(Value::Text(text)) => {
                sentence.push('\'');
                sentence.push_str(text);
                sentence.push('\'');
            }
            Some(param) => sentence.push_str(&param.to_sql_str()),
            None => sentence.push('?'),
        }
    }

    debug!("Running statement \"{}\"", sentence);
}

pub(crate) fn log_query_ending(rows: usize, action: &str) {
    let s = if rows > 1 { "s" } else { "" };
    debug!("{} {} row{}", action, rows, s);
}
