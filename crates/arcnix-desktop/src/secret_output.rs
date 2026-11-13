//! Hide a transient stdin credential even when a remote program echoes it in
//! separate output chunks. Decode UTF-8 only after matching the original bytes.
pub struct SecretOutput {
    secret: Vec<u8>,
    pending: Vec<u8>,
    utf8: Vec<u8>,
}
impl SecretOutput {
    pub fn new(secret: &[u8]) -> Self {
        Self {
            secret: secret.to_vec(),
            pending: Vec::new(),
            utf8: Vec::new(),
        }
    }
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        self.emit(false)
    }
    pub fn finish(&mut self) -> String {
        self.emit(true)
    }
    fn emit(&mut self, finish: bool) -> String {
        let limit = if finish {
            self.pending.len()
        } else {
            self.pending
                .len()
                .saturating_sub(self.secret.len().saturating_sub(1))
        };
        let mut consumed = 0;
        while consumed < limit {
            if !self.secret.is_empty() && self.pending[consumed..].starts_with(&self.secret) {
                self.utf8.extend_from_slice("[凭据已隐藏]".as_bytes());
                consumed += self.secret.len();
            } else {
                self.utf8.push(self.pending[consumed]);
                consumed += 1;
            }
        }
        self.pending.drain(..consumed);
        let mut text = String::new();
        loop {
            match std::str::from_utf8(&self.utf8) {
                Ok(valid) => {
                    text.push_str(valid);
                    self.utf8.clear();
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    text.push_str(
                        std::str::from_utf8(&self.utf8[..valid]).expect("validated UTF-8 prefix"),
                    );
                    self.utf8.drain(..valid);
                    if let Some(length) = error.error_len() {
                        text.push('\u{fffd}');
                        self.utf8.drain(..length);
                    } else {
                        if finish && !self.utf8.is_empty() {
                            text.push('\u{fffd}');
                            self.utf8.clear();
                        }
                        break;
                    }
                }
            }
        }
        text
    }
}
impl Drop for SecretOutput {
    fn drop(&mut self) {
        self.secret.fill(0);
        self.pending.fill(0);
        self.utf8.fill(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secrets_are_hidden_at_every_possible_chunk_boundary() {
        let input = "开始 secret-密码-secret 完成 secret-密码-secret".as_bytes();
        let secret = "secret-密码-secret".as_bytes();
        for size in 1..=input.len() {
            let mut filter = SecretOutput::new(secret);
            let mut output = String::new();
            for chunk in input.chunks(size) {
                output.push_str(&filter.push(chunk));
            }
            output.push_str(&filter.finish());
            assert_eq!(
                output, "开始 [凭据已隐藏] 完成 [凭据已隐藏]",
                "chunk size {size}"
            );
        }
    }
    #[test]
    fn empty_secret_preserves_multibyte_output_and_long_secret_flushes() {
        let mut filter = SecretOutput::new(b"");
        let mut output = String::new();
        for byte in "安装完成".as_bytes() {
            output.push_str(&filter.push(&[*byte]));
        }
        output.push_str(&filter.finish());
        assert_eq!(output, "安装完成");
        let mut filter = SecretOutput::new(b"very-long-password");
        assert!(filter.push(b"ok").is_empty());
        assert_eq!(filter.finish(), "ok");
    }
}
