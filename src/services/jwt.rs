use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub ip: Option<String>,
    pub iat: u64,
    pub exp: u64,
}

pub struct JwtService {
    decoding: DecodingKey,
}

impl JwtService {
    pub fn new(secret: &str) -> Self {
        Self {
            decoding: DecodingKey::from_secret(secret.as_bytes()),
        }
    }

    pub fn validate(&self, token: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
        let validation = Validation::new(Algorithm::HS256);
        Ok(jsonwebtoken::decode::<Claims>(token, &self.decoding, &validation)?.claims)
    }
}
