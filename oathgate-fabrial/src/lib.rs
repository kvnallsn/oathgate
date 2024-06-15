//! Fabrial library

pub mod tty;

/// A wrapper type for fabrial errors
pub struct Error {
    inner: Box<dyn std::error::Error + Send + Sync>,
    context: Option<String>,
}

pub trait ErrorContext<T> {
    fn context(self, ctx: impl Into<String>) -> Result<T, Error>;
}

impl Error {
    pub fn source(&self) -> &dyn std::error::Error {
        self.inner.as_ref()
    }

    pub fn context(&self) -> Option<&str> {
        self.context.as_ref().map(|s| s.as_str())
    }

    pub fn context_str(&self) -> &str {
        self.context.as_ref().map(|s| s.as_str()).unwrap_or("None")
    }
}

impl<E> From<E> for Error
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn from(error: E) -> Self {
        Self {
            inner: Box::new(error),
            context: None,
        }
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "Err({:?}, context: {:?})", self.inner, self.context)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        match self.context.as_ref() {
            Some(ctx) => write!(f, "Error: {ctx} ({})", self.inner),
            None => write!(f, "Error: ({})", self.inner),
        }
    }
}

impl<T, E> ErrorContext<T> for Result<T, E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn context(self, ctx: impl Into<String>) -> Result<T, Error> {
        match self {
            Ok(t) => Ok(t),
            Err(error) => Err(Error {
                inner: Box::new(error),
                context: Some(ctx.into()),
            }),
        }
    }
}

impl<T> ErrorContext<T> for Result<T, Error> {
    fn context(self, ctx: impl Into<String>) -> Result<T, Error> {
        self.map_err(|error| Error {
            inner: error.inner,
            context: Some(ctx.into()),
        })
    }
}
