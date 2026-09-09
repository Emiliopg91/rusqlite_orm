use std::fmt::Display;

use crate::types::value::Value;

#[derive(Clone)]
pub enum ValueOrFunction {
    Value(Value),
    Function(Function),
}

#[derive(Clone)]
pub enum Function {
    Date(Vec<String>),
}

impl ValueOrFunction {
    pub fn value_or_none(&self) -> Option<Value> {
        match self {
            ValueOrFunction::Value(v) => Some(v.clone()),
            _ => None,
        }
    }
}

impl Display for Function {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Function::Date(items) => {
                    let mut date_string = "'".to_string();
                    date_string.push_str(&items.join("','"));
                    date_string.push('\'');
                    format!("date({})", date_string)
                }
            }
        )
    }
}
