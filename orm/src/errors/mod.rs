use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("Cannot operate on database. Closed connection")]
    ClosedConnection(),
    #[error("Database is already initialized")]
    AlreadyInitialized(),
    #[error("Error while opening connection to {0}: {1}")]
    Connection(String, r2d2::Error),
    #[error("Error obtaining pooled connection: {0}")]
    Pool(r2d2::Error),
    #[error("Error while creating schema: {0}")]
    SchemaCreation(crate::rusqlite::Error),
    #[error("Error on transaction: {0}")]
    Transaction(Box<dyn std::error::Error + Send + Sync>),
    #[error("Error running on connection: {0}")]
    RunningOnConnection(Box<dyn std::error::Error + Send + Sync>),
    #[error("Error on insert: {0}")]
    Insert(crate::rusqlite::Error),
    #[error("Error on update: {0}")]
    Update(crate::rusqlite::Error),
    #[error("Error on select: {0}")]
    Select(crate::rusqlite::Error),
    #[error("Error on delete: {0}")]
    Delete(crate::rusqlite::Error),
    #[error("Error on savepoint: {0}")]
    Savepoint(crate::rusqlite::Error),
}

impl Serialize for DatabaseError {
    fn serialize<S>(&self, serializer: S) -> std::prelude::v1::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.to_string().as_ref())
    }
}

pub type Result<T> = std::result::Result<T, DatabaseError>;
