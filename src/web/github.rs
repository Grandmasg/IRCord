use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub struct GitHubWebhookValidator;

impl GitHubWebhookValidator {
    /// Valideert de X-Hub-Signature-256 header tegen het geconfigureerde geheim
    pub fn verify_signature(secret: &str, payload: &[u8], signature_header: &str) -> bool {
        let expected_prefix = "sha256=";
        if !signature_header.starts_with(expected_prefix) {
            return false;
        }

        let hex_signature = &signature_header[expected_prefix.len()..];
        let Ok(signature_bytes) = hex::decode(hex_signature) else {
            return false;
        };

        let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
            return false;
        };

        mac.update(payload);
        mac.verify_slice(&signature_bytes).is_ok()
    }
}
