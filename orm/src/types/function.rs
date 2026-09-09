use std::fmt::Display;

#[derive(Clone)]
pub enum Function {
    DateTime(Vec<String>),
}

impl Display for Function {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Function::DateTime(items) => {
                    let mut date_string = "'".to_string();
                    date_string.push_str(&items.join("','"));
                    date_string.push('\'');
                    format!("date({})", date_string)
                }
            }
        )
    }
}
