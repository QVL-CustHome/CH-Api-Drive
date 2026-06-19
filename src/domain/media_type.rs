use serde::{Deserialize, Serialize};
use sqlx::Postgres;
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use sqlx::postgres::{PgArgumentBuffer, PgTypeInfo, PgValueRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaType {
    Image,
    Video,
}

impl MediaType {
    pub fn as_str(self) -> &'static str {
        match self {
            MediaType::Image => "image",
            MediaType::Video => "video",
        }
    }

    pub fn from_mime(mime: &str) -> Option<Self> {
        if mime.starts_with("image/") {
            Some(MediaType::Image)
        } else if mime.starts_with("video/") {
            Some(MediaType::Video)
        } else {
            None
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("type de média invalide : {0}")]
pub struct ParseMediaTypeError(String);

impl std::str::FromStr for MediaType {
    type Err = ParseMediaTypeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "image" => Ok(MediaType::Image),
            "video" => Ok(MediaType::Video),
            other => Err(ParseMediaTypeError(other.to_string())),
        }
    }
}

impl sqlx::Type<Postgres> for MediaType {
    fn type_info() -> PgTypeInfo {
        <&str as sqlx::Type<Postgres>>::type_info()
    }

    fn compatible(ty: &PgTypeInfo) -> bool {
        <&str as sqlx::Type<Postgres>>::compatible(ty)
    }
}

impl<'r> sqlx::Decode<'r, Postgres> for MediaType {
    fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
        let raw = <&str as sqlx::Decode<Postgres>>::decode(value)?;
        raw.parse().map_err(Into::into)
    }
}

impl<'q> sqlx::Encode<'q, Postgres> for MediaType {
    fn encode_by_ref(&self, buf: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
        <&str as sqlx::Encode<Postgres>>::encode(self.as_str(), buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_values() {
        assert_eq!("image".parse::<MediaType>().unwrap(), MediaType::Image);
        assert_eq!("video".parse::<MediaType>().unwrap(), MediaType::Video);
    }

    #[test]
    fn parse_unknown_is_rejected() {
        assert!("audio".parse::<MediaType>().is_err());
    }

    #[test]
    fn from_mime_classifies_image_and_video() {
        assert_eq!(MediaType::from_mime("image/png"), Some(MediaType::Image));
        assert_eq!(MediaType::from_mime("video/mp4"), Some(MediaType::Video));
        assert_eq!(MediaType::from_mime("application/pdf"), None);
    }

    #[test]
    fn serialized_as_lowercase() {
        assert_eq!(
            serde_json::to_string(&MediaType::Image).unwrap(),
            "\"image\""
        );
    }

    #[test]
    fn deserialize_invalid_is_rejected() {
        assert!(serde_json::from_str::<MediaType>("\"gif-thing\"").is_err());
    }
}
