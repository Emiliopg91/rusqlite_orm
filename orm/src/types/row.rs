use std::sync::Arc;

use crate::types::value::Value;

/// A result row of a non-mapped select. The column names are shared (one `Arc` per
/// query, not per row) and values are stored positionally.
///
/// Values come from SQLite, so they are always `Null`, `Int64`, `Float64`, `Text` or `Blob`;
/// the typed getters convert from those. Each getter panics if the column is missing, NULL or
/// of another type, and its `_nullable` counterpart returns `None` for a missing or NULL column
/// instead (still panicking on another type).
#[derive(Clone, Default)]
pub struct Row {
    names: Arc<[String]>,
    values: Vec<Value>,
}

/// Getters for integer types narrower than `i64`, panicking if the value is out of range.
macro_rules! narrowed_int_getters {
    ($($ty:ty => $get:ident, $get_nullable:ident;)*) => {
        $(
            #[doc = concat!("Reads an integer column as `", stringify!($ty), "`.")]
            ///
            /// # Panics
            /// If the column is missing, NULL, not an integer or out of range.
            pub fn $get(&self, column: &str) -> $ty {
                let v = self.get_int(column);
                <$ty>::try_from(v).unwrap_or_else(|_| {
                    panic!(
                        "column '{column}' value {v} is out of range for {}",
                        stringify!($ty)
                    )
                })
            }

            #[doc = concat!(
                "Reads a nullable integer column as `", stringify!($ty), "`; `None` if missing or NULL."
            )]
            ///
            /// # Panics
            /// If the value is not an integer or out of range.
            pub fn $get_nullable(&self, column: &str) -> Option<$ty> {
                self.is_present(column).then(|| self.$get(column))
            }
        )*
    };
}

impl Row {
    pub fn get(&self, column: &str) -> Option<&Value> {
        self.names
            .iter()
            .position(|name| name == column)
            .map(|idx| &self.values[idx])
    }

    /// Whether the column exists and is not NULL.
    fn is_present(&self, column: &str) -> bool {
        !matches!(self.get(column), Some(Value::Null) | None)
    }

    /// Reads an integer column.
    ///
    /// # Panics
    /// If the column is missing, NULL or not an integer.
    pub fn get_int(&self, column: &str) -> i64 {
        match self.get(column) {
            Some(Value::Int64(v)) => *v,
            _ => panic!("column '{column}' is not an integer"),
        }
    }

    /// Reads a nullable integer column; `None` if missing or NULL.
    ///
    /// # Panics
    /// If the value is not an integer.
    pub fn get_int_nullable(&self, column: &str) -> Option<i64> {
        self.is_present(column).then(|| self.get_int(column))
    }

    narrowed_int_getters! {
        i8 => get_i8, get_i8_nullable;
        i16 => get_i16, get_i16_nullable;
        i32 => get_i32, get_i32_nullable;
        isize => get_isize, get_isize_nullable;
        u8 => get_u8, get_u8_nullable;
        u16 => get_u16, get_u16_nullable;
        u32 => get_u32, get_u32_nullable;
        u64 => get_u64, get_u64_nullable;
        usize => get_usize, get_usize_nullable;
    }

    /// Reads a real column, accepting integers too as SQLite stores whole reals as such.
    ///
    /// # Panics
    /// If the column is missing, NULL or not numeric.
    pub fn get_f64(&self, column: &str) -> f64 {
        match self.get(column) {
            Some(Value::Float64(v)) => *v,
            Some(Value::Int64(v)) => *v as f64,
            _ => panic!("column '{column}' is not a real"),
        }
    }

    /// Reads a nullable real column; `None` if missing or NULL.
    ///
    /// # Panics
    /// If the value is not numeric.
    pub fn get_f64_nullable(&self, column: &str) -> Option<f64> {
        self.is_present(column).then(|| self.get_f64(column))
    }

    /// Reads a real column as `f32`, losing precision as `as` does.
    ///
    /// # Panics
    /// If the column is missing, NULL or not numeric.
    pub fn get_f32(&self, column: &str) -> f32 {
        self.get_f64(column) as f32
    }

    /// Reads a nullable real column as `f32`; `None` if missing or NULL.
    ///
    /// # Panics
    /// If the value is not numeric.
    pub fn get_f32_nullable(&self, column: &str) -> Option<f32> {
        self.is_present(column).then(|| self.get_f32(column))
    }

    /// Reads a boolean column, stored by SQLite as an integer where anything but 0 is true.
    ///
    /// # Panics
    /// If the column is missing, NULL or not an integer.
    pub fn get_bool(&self, column: &str) -> bool {
        self.get_int(column) != 0
    }

    /// Reads a nullable boolean column; `None` if missing or NULL.
    ///
    /// # Panics
    /// If the value is not an integer.
    pub fn get_bool_nullable(&self, column: &str) -> Option<bool> {
        self.is_present(column).then(|| self.get_bool(column))
    }

    /// Reads a text column.
    ///
    /// # Panics
    /// If the column is missing, NULL or not text.
    pub fn get_text(&self, column: &str) -> String {
        match self.get(column) {
            Some(Value::Text(v)) => v.clone(),
            _ => panic!("column '{column}' is not text"),
        }
    }

    /// Reads a nullable text column; `None` if missing or NULL.
    ///
    /// # Panics
    /// If the value is not text.
    pub fn get_text_nullable(&self, column: &str) -> Option<String> {
        self.is_present(column).then(|| self.get_text(column))
    }

    /// Reads a blob column.
    ///
    /// # Panics
    /// If the column is missing, NULL or not a blob.
    pub fn get_blob(&self, column: &str) -> Vec<u8> {
        match self.get(column) {
            Some(Value::Blob(v)) => v.clone(),
            _ => panic!("column '{column}' is not a blob"),
        }
    }

    /// Reads a nullable blob column; `None` if missing or NULL.
    ///
    /// # Panics
    /// If the value is not a blob.
    pub fn get_blob_nullable(&self, column: &str) -> Option<Vec<u8>> {
        self.is_present(column).then(|| self.get_blob(column))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> Row {
        let columns = [
            ("int", Value::Int64(-3)),
            ("big", Value::Int64(300)),
            ("real", Value::Float64(1.5)),
            ("text", Value::Text("abc".to_string())),
            ("blob", Value::Blob(vec![1, 2])),
            ("null", Value::Null),
        ];
        Row {
            names: columns.iter().map(|(n, _)| n.to_string()).collect(),
            values: columns.into_iter().map(|(_, v)| v).collect(),
        }
    }

    #[test]
    fn integers_are_narrowed() {
        let row = row();
        assert_eq!(row.get_int("int"), -3);
        assert_eq!(row.get_i8("int"), -3);
        assert_eq!(row.get_i32("int"), -3);
        assert_eq!(row.get_u16("big"), 300);
        assert_eq!(row.get_usize("big"), 300);
        assert!(row.get_bool("int"));
    }

    #[test]
    #[should_panic(expected = "out of range for u8")]
    fn out_of_range_integer_panics() {
        row().get_u8("big");
    }

    #[test]
    #[should_panic(expected = "out of range for u32")]
    fn negative_into_unsigned_panics() {
        row().get_u32("int");
    }

    #[test]
    fn reals_accept_integers() {
        let row = row();
        assert_eq!(row.get_f64("real"), 1.5);
        assert_eq!(row.get_f32("real"), 1.5);
        assert_eq!(row.get_f64("int"), -3.0);
    }

    #[test]
    fn text_and_blob() {
        let row = row();
        assert_eq!(row.get_text("text"), "abc");
        assert_eq!(row.get_blob("blob"), [1, 2]);
    }

    #[test]
    fn nullable_getters_return_none_for_null_or_missing() {
        let row = row();
        for column in ["null", "missing"] {
            assert_eq!(row.get_int_nullable(column), None);
            assert_eq!(row.get_u16_nullable(column), None);
            assert_eq!(row.get_f32_nullable(column), None);
            assert_eq!(row.get_bool_nullable(column), None);
            assert_eq!(row.get_text_nullable(column), None);
            assert_eq!(row.get_blob_nullable(column), None);
        }
        assert_eq!(row.get_u16_nullable("big"), Some(300));
        assert_eq!(row.get_text_nullable("text"), Some("abc".to_string()));
    }

    #[test]
    #[should_panic(expected = "is not text")]
    fn nullable_getter_panics_on_other_type() {
        row().get_text_nullable("int");
    }
}
