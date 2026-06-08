use std::fmt;

#[derive(Debug)]
pub enum UiError {
    ParseError(String),
    NoDeclarations,
    Io(String),
}

impl fmt::Display for UiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UiError::ParseError(s)  => write!(f, "parse error: {}", s),
            UiError::NoDeclarations => write!(f, "no view or form declarations found"),
            UiError::Io(s)          => write!(f, "io error: {}", s),
        }
    }
}
