//! Error reporting for application boundaries that receive errors from several subsystems.

#![no_std]

extern crate alloc;

use alloc::{borrow::Cow, boxed::Box, string::ToString};
use core::{error::Error as StdError, fmt};

type BoxedError = Box<dyn StdError + Send + Sync + 'static>;

pub type Result<T, E = Error> = core::result::Result<T, E>;

pub enum Error {
    Message(Message),
    Source(BoxedError),
}

impl Error {
    pub fn message(message: fmt::Arguments<'_>) -> Self {
        Self::Message(Message(match message.as_str() {
            Some(text) => Cow::Borrowed(text),
            None => Cow::Owned(message.to_string()),
        }))
    }

    pub fn context(self, message: impl Into<Cow<'static, str>>) -> Self {
        Self::Source(Box::new(ContextError {
            message: Message(message.into()),
            cause: self,
        }))
    }

    pub fn as_error(&self) -> &(dyn StdError + Send + Sync + 'static) {
        match self {
            Self::Message(message) => message,
            Self::Source(source) => source.as_ref(),
        }
    }

    pub fn into_boxed(self) -> BoxedError {
        match self {
            Self::Message(message) => Box::new(message),
            Self::Source(source) => source,
        }
    }

    pub fn find<E: StdError + 'static>(&self) -> Option<&E> {
        self.causes().find_map(|error| error.downcast_ref::<E>())
    }

    fn causes(&self) -> impl Iterator<Item = &(dyn StdError + 'static)> {
        let first: &(dyn StdError + 'static) = self.as_error();
        core::iter::successors(Some(first), |&error| error.source())
    }
}

impl<E: Into<BoxedError>> From<E> for Error {
    fn from(error: E) -> Self {
        Self::Source(error.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.as_error())?;
        if formatter.alternate() {
            for cause in self.causes().skip(1) {
                write!(formatter, ": {cause}")?;
            }
        }
        Ok(())
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:#}")
    }
}

#[derive(Debug)]
pub struct Message(Cow<'static, str>);

impl fmt::Display for Message {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl StdError for Message {}

#[derive(Debug)]
struct ContextError {
    message: Message,
    cause: Error,
}

impl fmt::Display for ContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.message, formatter)
    }
}

impl StdError for ContextError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(self.cause.as_error())
    }
}

pub trait Context<T>: Sized {
    fn context(self, message: impl Into<Cow<'static, str>>) -> Result<T> {
        self.with_context(|| message)
    }

    fn with_context<M: Into<Cow<'static, str>>>(self, message: impl FnOnce() -> M) -> Result<T>;
}

impl<T, E: Into<Error>> Context<T> for Result<T, E> {
    fn with_context<M: Into<Cow<'static, str>>>(self, message: impl FnOnce() -> M) -> Result<T> {
        self.map_err(|error| error.into().context(message()))
    }
}

impl<T> Context<T> for Option<T> {
    fn with_context<M: Into<Cow<'static, str>>>(self, message: impl FnOnce() -> M) -> Result<T> {
        self.ok_or_else(|| Error::Message(Message(message().into())))
    }
}

#[macro_export]
macro_rules! report {
    ($message:literal $(,)?) => {
        $crate::Error::message(format_args!($message))
    };
    ($format:literal, $($args:tt)*) => {
        $crate::Error::message(format_args!($format, $($args)*))
    };
    ($error:expr $(,)?) => {
        $crate::Error::from($error)
    };
}

#[macro_export]
macro_rules! bail {
    ($($args:tt)*) => {
        return Err($crate::report!($($args)*))
    };
}

#[macro_export]
macro_rules! ensure {
    ($condition:expr, $($args:tt)*) => {
        if !$condition {
            $crate::bail!($($args)*);
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Offline;

    impl fmt::Display for Offline {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("Offline")
        }
    }

    impl StdError for Offline {}

    #[test]
    fn literal_reports_keep_the_message_inline() {
        let error = report!("Invalid request");
        assert!(matches!(error, Error::Message(Message(Cow::Borrowed(_)))));
    }

    #[test]
    fn context_keeps_the_typed_cause_and_formats_the_chain() {
        let error = Err::<(), _>(Offline)
            .context("Connect")
            .context("Send request")
            .unwrap_err();
        assert!(error.find::<Offline>().is_some());
        assert_eq!(error.to_string(), "Send request");
        assert_eq!(
            alloc::format!("{error:#}"),
            "Send request: Connect: Offline"
        );
    }
}
