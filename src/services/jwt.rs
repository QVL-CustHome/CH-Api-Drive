use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use jsonwebtoken::errors::{Error as JwtError, ErrorKind as JwtErrorKind};

pub const DEFAULT_ALGORITHM: Algorithm = Algorithm::HS256;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Claims {
    pub sub: String,

    #[serde(default, deserialize_with = "deserialize_roles")]
    pub roles: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,

    pub iat: u64,
    pub exp: u64,
}

impl Claims {
    pub fn new(
        sub: impl Into<String>,
        roles: Vec<String>,
        ip: Option<String>,
        iat: u64,
        exp: u64,
    ) -> Self {
        Self {
            sub: sub.into(),
            roles,
            ip,
            iat,
            exp,
        }
    }

    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|owned| owned == role)
    }
}

fn deserialize_roles<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum RolesFormat {
        Flat(Vec<String>),
        PerPortalMany(HashMap<String, Vec<String>>),
        PerPortalOne(HashMap<String, String>),
    }

    let collected = match RolesFormat::deserialize(deserializer)? {
        RolesFormat::Flat(roles) => roles,
        RolesFormat::PerPortalMany(map) => map.into_values().flatten().collect(),
        RolesFormat::PerPortalOne(map) => map.into_values().collect(),
    };

    let mut seen = HashSet::new();
    Ok(collected
        .into_iter()
        .filter(|role| seen.insert(role.clone()))
        .collect())
}

pub struct JwtService {
    encoding: EncodingKey,
    decoding: DecodingKey,
    algorithm: Algorithm,
}

impl JwtService {
    pub fn from_secret(secret: &str) -> Self {
        Self::with_algorithm(secret, DEFAULT_ALGORITHM)
    }

    pub fn with_algorithm(secret: &str, algorithm: Algorithm) -> Self {
        Self {
            encoding: EncodingKey::from_secret(secret.as_bytes()),
            decoding: DecodingKey::from_secret(secret.as_bytes()),
            algorithm,
        }
    }

    pub fn algorithm(&self) -> Algorithm {
        self.algorithm
    }

    pub fn decode(&self, token: &str) -> Result<Claims, JwtError> {
        let validation = Validation::new(self.algorithm);
        Ok(jsonwebtoken::decode::<Claims>(token, &self.decoding, &validation)?.claims)
    }

    pub fn validate(&self, token: &str) -> Result<Claims, JwtError> {
        self.decode(token)
    }

    pub fn encode(&self, claims: &Claims) -> Result<String, JwtError> {
        jsonwebtoken::encode(&Header::new(self.algorithm), claims, &self.encoding)
    }

    pub fn issue(
        &self,
        sub: impl Into<String>,
        roles: Vec<String>,
        ip: Option<String>,
        ttl: Duration,
    ) -> Result<String, JwtError> {
        let now = unix_now();
        let claims = Claims::new(sub, roles, ip, now, now + ttl.as_secs());
        self.encode(&claims)
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("horloge systeme anterieure a 1970")
        .as_secs()
}
