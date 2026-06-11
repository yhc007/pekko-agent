use jsonwebtoken::{
    decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation,
};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum JwtError {
    #[error("token has expired")]
    Expired,
    #[error("invalid token: {0}")]
    Invalid(String),
    #[error("failed to issue token: {0}")]
    Encode(String),
}

/// Claims embedded in every access token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// user_id (JWT "subject")
    pub sub: String,
    pub tenant_id: String,
    pub roles: Vec<String>,
    /// Issued-at (Unix seconds)
    pub iat: u64,
    /// Expiry (Unix seconds)
    pub exp: u64,
}

/// Issues and validates HS256 JWT access tokens.
///
/// Cheap to clone — inner keys are Arc'd by jsonwebtoken.
#[derive(Clone)]
pub struct JwtManager {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    pub token_ttl_seconds: u64,
}

impl JwtManager {
    pub fn new(secret: impl AsRef<[u8]>) -> Self {
        let s = secret.as_ref();
        Self {
            encoding_key: EncodingKey::from_secret(s),
            decoding_key: DecodingKey::from_secret(s),
            token_ttl_seconds: 3600,
        }
    }

    pub fn with_ttl(mut self, seconds: u64) -> Self {
        self.token_ttl_seconds = seconds;
        self
    }

    /// Sign a new token for `user_id` belonging to `tenant_id` with the given `roles`.
    pub fn issue(
        &self,
        user_id: &str,
        tenant_id: &str,
        roles: Vec<String>,
    ) -> Result<String, JwtError> {
        let now = unix_now();
        let claims = Claims {
            sub:       user_id.to_string(),
            tenant_id: tenant_id.to_string(),
            roles,
            iat: now,
            exp: now + self.token_ttl_seconds,
        };
        encode(&Header::default(), &claims, &self.encoding_key)
            .map_err(|e| JwtError::Encode(e.to_string()))
    }

    /// Decode and verify a Bearer token. Returns the embedded `Claims` on success.
    pub fn validate(&self, token: &str) -> Result<Claims, JwtError> {
        let mut v = Validation::new(Algorithm::HS256);
        v.validate_exp = true;
        decode::<Claims>(token, &self.decoding_key, &v)
            .map(|td| td.claims)
            .map_err(|e| {
                if *e.kind() == jsonwebtoken::errors::ErrorKind::ExpiredSignature {
                    JwtError::Expired
                } else {
                    JwtError::Invalid(e.to_string())
                }
            })
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before epoch")
        .as_secs()
}
