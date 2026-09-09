use crate::types::{function::Function, value::Value};

#[derive(Clone)]
pub enum ValueOrFunction {
    Value(Value),
    Function(Function),
}

impl ValueOrFunction {
    pub fn value_or_none(&self) -> Option<Value> {
        match self {
            ValueOrFunction::Value(v) => Some(v.clone()),
            _ => None,
        }
    }
}

impl From<Function> for ValueOrFunction {
    fn from(value: Function) -> Self {
        ValueOrFunction::Function(value)
    }
}

impl From<isize> for ValueOrFunction {
    fn from(v: isize) -> Self {
        ValueOrFunction::Value(Value::IntSize(v))
    }
}
impl From<i8> for ValueOrFunction {
    fn from(v: i8) -> Self {
        ValueOrFunction::Value(Value::Int8(v))
    }
}
impl From<i16> for ValueOrFunction {
    fn from(v: i16) -> Self {
        ValueOrFunction::Value(Value::Int16(v))
    }
}
impl From<i32> for ValueOrFunction {
    fn from(v: i32) -> Self {
        ValueOrFunction::Value(Value::Int32(v))
    }
}
impl From<i64> for ValueOrFunction {
    fn from(v: i64) -> Self {
        ValueOrFunction::Value(Value::Int64(v))
    }
}
impl From<usize> for ValueOrFunction {
    fn from(v: usize) -> Self {
        ValueOrFunction::Value(Value::UntSize(v))
    }
}
impl From<u8> for ValueOrFunction {
    fn from(v: u8) -> Self {
        ValueOrFunction::Value(Value::Unt8(v))
    }
}
impl From<u16> for ValueOrFunction {
    fn from(v: u16) -> Self {
        ValueOrFunction::Value(Value::Unt16(v))
    }
}
impl From<u32> for ValueOrFunction {
    fn from(v: u32) -> Self {
        ValueOrFunction::Value(Value::Unt32(v))
    }
}
impl From<u64> for ValueOrFunction {
    fn from(v: u64) -> Self {
        ValueOrFunction::Value(Value::Unt64(v))
    }
}
impl From<f32> for ValueOrFunction {
    fn from(v: f32) -> Self {
        ValueOrFunction::Value(Value::Float32(v))
    }
}
impl From<f64> for ValueOrFunction {
    fn from(v: f64) -> Self {
        ValueOrFunction::Value(Value::Float64(v))
    }
}
impl From<bool> for ValueOrFunction {
    fn from(v: bool) -> Self {
        ValueOrFunction::Value(Value::Bool(v))
    }
}
impl From<String> for ValueOrFunction {
    fn from(v: String) -> Self {
        ValueOrFunction::Value(Value::Text(v))
    }
}
impl From<&str> for ValueOrFunction {
    fn from(v: &str) -> Self {
        ValueOrFunction::Value(Value::Text(v.to_string()))
    }
}
impl From<Vec<u8>> for ValueOrFunction {
    fn from(v: Vec<u8>) -> Self {
        ValueOrFunction::Value(Value::Blob(v))
    }
}

impl From<&[u8]> for ValueOrFunction {
    fn from(v: &[u8]) -> Self {
        ValueOrFunction::Value(Value::Blob(v.to_vec()))
    }
}

impl<T> From<Option<T>> for ValueOrFunction
where
    T: Into<ValueOrFunction>,
{
    fn from(opt: Option<T>) -> Self {
        match opt {
            Some(v) => v.into(),
            None => ValueOrFunction::Value(Value::Null),
        }
    }
}
