pub struct Error(String);

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<E: std::fmt::Display> From<E> for Error {
    fn from(e: E) -> Self {
        Self(e.to_string())
    }
}

impl From<Error> for String {
    fn from(e: Error) -> String {
        e.0
    }
}

pub trait Context<T> {
    fn with_context<F, D>(self, f: F) -> Result<T>
    where
        F: FnOnce() -> D,
        D: std::fmt::Display;
}

impl<T, E: std::fmt::Display> Context<T> for std::result::Result<T, E> {
    fn with_context<F, D>(self, f: F) -> Result<T>
    where
        F: FnOnce() -> D,
        D: std::fmt::Display,
    {
        self.map_err(|e| Error(format!("{}: {}", f(), e)))
    }
}
