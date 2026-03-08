use thiserror::Error;

#[derive(Error, Debug)]
pub enum GraphyError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Storage error: {0}")]
    Storage(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_display() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: GraphyError = io_err.into();
        let msg = format!("{}", err);
        assert!(msg.contains("IO error"));
        assert!(msg.contains("file not found"));
    }

    #[test]
    fn storage_error_display() {
        let err = GraphyError::Storage("corrupt database".to_string());
        let msg = format!("{}", err);
        assert!(msg.contains("Storage error"));
        assert!(msg.contains("corrupt database"));
    }

    #[test]
    fn io_error_from_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
        let err = GraphyError::from(io_err);
        assert!(matches!(err, GraphyError::Io(_)));
    }

    #[test]
    fn storage_error_empty_message() {
        let err = GraphyError::Storage(String::new());
        assert_eq!(format!("{}", err), "Storage error: ");
    }
}
