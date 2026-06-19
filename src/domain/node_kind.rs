use serde::{Deserialize, Serialize};
use sqlx::Postgres;
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use sqlx::postgres::{PgArgumentBuffer, PgTypeInfo, PgValueRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Folder,
    File,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::Folder => "folder",
            NodeKind::File => "file",
        }
    }

    pub fn is_folder(self) -> bool {
        matches!(self, NodeKind::Folder)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("type de nœud invalide : {0}")]
pub struct ParseNodeKindError(String);

impl std::str::FromStr for NodeKind {
    type Err = ParseNodeKindError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "folder" => Ok(NodeKind::Folder),
            "file" => Ok(NodeKind::File),
            other => Err(ParseNodeKindError(other.to_string())),
        }
    }
}

impl sqlx::Type<Postgres> for NodeKind {
    fn type_info() -> PgTypeInfo {
        <&str as sqlx::Type<Postgres>>::type_info()
    }

    fn compatible(ty: &PgTypeInfo) -> bool {
        <&str as sqlx::Type<Postgres>>::compatible(ty)
    }
}

impl<'r> sqlx::Decode<'r, Postgres> for NodeKind {
    fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
        let raw = <&str as sqlx::Decode<Postgres>>::decode(value)?;
        raw.parse().map_err(Into::into)
    }
}

impl<'q> sqlx::Encode<'q, Postgres> for NodeKind {
    fn encode_by_ref(&self, buf: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
        <&str as sqlx::Encode<Postgres>>::encode(self.as_str(), buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_values() {
        assert_eq!("folder".parse::<NodeKind>().unwrap(), NodeKind::Folder);
        assert_eq!("file".parse::<NodeKind>().unwrap(), NodeKind::File);
    }

    #[test]
    fn parse_unknown_is_rejected() {
        assert!("symlink".parse::<NodeKind>().is_err());
    }

    #[test]
    fn as_str_roundtrip() {
        for kind in [NodeKind::Folder, NodeKind::File] {
            assert_eq!(kind.as_str().parse::<NodeKind>().unwrap(), kind);
        }
    }

    #[test]
    fn serialized_as_lowercase() {
        assert_eq!(
            serde_json::to_string(&NodeKind::Folder).unwrap(),
            "\"folder\""
        );
    }

    #[test]
    fn deserialize_invalid_is_rejected() {
        assert!(serde_json::from_str::<NodeKind>("\"directory\"").is_err());
    }
}
