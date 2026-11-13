//! Classify a lost SSH transport separately from ordinary file/permission errors.
pub fn is_connection_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        if let Some(error) = cause.downcast_ref::<ssh2::Error>() {
            return matches!(
                error.code(),
                ssh2::ErrorCode::Session(-1 | -2 | -3 | -7 | -9 | -13 | -30 | -43 | -45)
            );
        }
        cause.downcast_ref::<std::io::Error>().is_some_and(|error| {
            matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::NotConnected
                    | std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::TimedOut
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinguishes_disconnect_from_permission_and_missing_files() {
        let broken = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
            .context("SFTP upload");
        assert!(is_connection_error(&broken));
        assert!(is_connection_error(&anyhow::Error::new(
            ssh2::Error::from_errno(ssh2::ErrorCode::Session(-43))
        )));
        assert!(!is_connection_error(&anyhow::Error::new(
            ssh2::Error::from_errno(ssh2::ErrorCode::SFTP(2))
        )));
        assert!(!is_connection_error(&anyhow::Error::new(
            std::io::Error::from(std::io::ErrorKind::PermissionDenied)
        )));
        assert!(!is_connection_error(&anyhow::anyhow!(
            "Destination already exists"
        )));
    }
}
