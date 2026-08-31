use std::fmt;

#[derive(Debug)]
pub enum UiError {
    ParseError(String),
    NoDeclarations,
    Io(String),
    /// BACKLOG item 239 — `onSuccess: navigate(X)` names a view `X` that
    /// isn't declared anywhere in the module.
    UnknownNavigateTarget(String),
}

impl fmt::Display for UiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UiError::ParseError(s)  => write!(f, "parse error: {}", s),
            UiError::NoDeclarations => write!(f, "no view or form declarations found"),
            UiError::Io(s)          => write!(f, "io error: {}", s),
            UiError::UnknownNavigateTarget(name) =>
                write!(f, "onSuccess navigate({}) — no `view {}` is declared in this module", name, name),
        }
    }
}
