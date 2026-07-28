use thiserror::Error;

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("Cannot operate on database. Closed connection")]
    ClosedConnection(),
    #[error("Error while opening connection to {0}: {1}")]
    Connection(String, crate::rusqlite::Error),
    #[error("Error while enabling foreign keys pragma: {0}")]
    ForeignKeysPragma(crate::rusqlite::Error),
    #[error("Error while creating schema: {0}")]
    SchemaCreation(crate::rusqlite::Error),
    #[error("Error on transaction: {0}")]
    Transaction(crate::rusqlite::Error),
    #[error("Error on insert: {0}")]
    Insert(crate::rusqlite::Error),
    #[error("Error on update: {0}")]
    Update(crate::rusqlite::Error),
    #[error("Error on select: {0}")]
    Select(crate::rusqlite::Error),
    #[error("Error on delete: {0}")]
    Delete(crate::rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, DatabaseError>;
