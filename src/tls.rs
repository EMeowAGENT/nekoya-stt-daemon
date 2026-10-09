//! rustls 0.23 要求显式选择进程级加密提供者（`CryptoProvider`）。
//!
//! 依赖树里 `tokio-rustls → rustls` 不带 provider 特性，若不在任何 TLS 握手前
//! 安装默认 provider，第一次 WSS 连接就会 panic：
//! `Could not automatically determine the process-level CryptoProvider from Rustls crate features`。
//!
//! 这里统一安装 `ring` provider，且**幂等**（重复调用无害）。

/// 安装默认加密提供者；可重复调用。
pub fn install_crypto_provider() {
    // 已安装过则返回 Err（Arc<CryptoProvider>），忽略即可。
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_is_idempotent() {
        install_crypto_provider();
        install_crypto_provider(); // 第二次不应 panic
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }

    /// 构造 `ClientConfig` 会使用进程默认 provider；未安装时会 panic。
    /// 本测试等价于「第一次 WSS 握手不再 panic」的离线护栏。
    #[test]
    fn client_config_builds_with_default_provider() {
        install_crypto_provider();
        let _ = rustls::ClientConfig::builder()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
    }
}
