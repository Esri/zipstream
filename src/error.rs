use std::{error::Error, fmt::{self, Display}};

/// Helper for displaying errors with their sources
pub struct Report<T>(pub T);
impl<T: Error> Display for Report<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut error: &dyn Error = &self.0;

        write!(f, "{}", error)?;

        while let Some(source) = error.source() {
            write!(f, "\n  : {source}")?;
            error = source;
        }

        Ok(())
    }
}
