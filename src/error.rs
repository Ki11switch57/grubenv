use std::error::Error as StdError;
use std::fmt;
use std::io;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    InvalidSignature,
    InvalidName(String),
    InvalidValue(String),
    InvalidUtf8,
    Malformed { offset: usize },
    NoSpace { required: usize, capacity: usize },
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(err) => write!(f, "i/o error: {err}"),
            Error::InvalidSignature => f.write_str("missing GRUB environment block signature"),
            Error::InvalidName(name) => write!(f, "invalid variable name: {name:?}"),
            Error::InvalidValue(name) => write!(f, "invalid value for variable {name:?}"),
            Error::InvalidUtf8 => f.write_str("environment block contains invalid UTF-8"),
            Error::Malformed { offset } => {
                write!(f, "malformed environment block at byte {offset}")
            }
            Error::NoSpace { required, capacity } => write!(
                f,
                "environment block is full: {required} bytes required, {capacity} available"
            ),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Error::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::Io(err)
    }
}
