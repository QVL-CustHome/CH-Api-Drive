use std::io::{Error, ErrorKind, Result};
use std::path::{Component, Path, PathBuf};
use tokio::fs;

#[derive(Clone)]
pub struct FsStorage {
    root: PathBuf,
}

impl FsStorage {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn build_key(owner_id: &str, node_id: uuid::Uuid) -> Result<String> {
        if !is_object_id(owner_id) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "owner_id invalide",
            ));
        }
        Ok(format!("{owner_id}/{node_id}"))
    }

    pub fn thumb_key(storage_key: &str) -> String {
        format!("{storage_key}.thumb")
    }

    pub async fn write_bytes(&self, key: &str, bytes: &[u8]) -> Result<()> {
        let path = self.resolve(key)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(path, bytes).await
    }

    fn resolve(&self, key: &str) -> Result<PathBuf> {
        if !is_safe_relative_key(key) {
            return Err(Error::new(ErrorKind::PermissionDenied, "clé de stockage invalide"));
        }
        let path = self.root.join(key);
        if !path.starts_with(&self.root) {
            return Err(Error::new(ErrorKind::PermissionDenied, "chemin hors du répertoire racine"));
        }
        Ok(path)
    }

    pub async fn create_writer(&self, key: &str) -> Result<fs::File> {
        let path = self.resolve(key)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::File::create(path).await
    }

    pub async fn open(&self, key: &str) -> Result<fs::File> {
        fs::File::open(self.resolve(key)?).await
    }

    pub async fn delete(&self, key: &str) -> Result<()> {
        match fs::remove_file(self.resolve(key)?).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub async fn metadata(&self, key: &str) -> Result<std::fs::Metadata> {
        fs::metadata(self.resolve(key)?).await
    }
}

pub fn is_object_id(value: &str) -> bool {
    value.len() == 24 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_safe_relative_key(key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    if key.contains('\0') {
        return false;
    }
    if key.contains('\\') {
        return false;
    }
    let path = Path::new(key);
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_OWNER_ID: &str = "0123456789abcdef01234567";

    fn storage() -> FsStorage {
        FsStorage::new(PathBuf::from("/srv/drive/data"))
    }

    #[test]
    fn is_object_id_accepts_24_lowercase_hex() {
        assert!(is_object_id(VALID_OWNER_ID));
    }

    #[test]
    fn is_object_id_accepts_uppercase_hex() {
        assert!(is_object_id("0123456789ABCDEF01234567"));
    }

    #[test]
    fn is_object_id_accepts_mixed_case_hex() {
        assert!(is_object_id("0123456789AbCdEf01234567"));
    }

    #[test]
    fn is_object_id_rejects_too_short() {
        assert!(!is_object_id("0123456789abcdef0123456"));
    }

    #[test]
    fn is_object_id_rejects_too_long() {
        assert!(!is_object_id("0123456789abcdef012345678"));
    }

    #[test]
    fn is_object_id_rejects_empty() {
        assert!(!is_object_id(""));
    }

    #[test]
    fn is_object_id_rejects_non_hex_char() {
        assert!(!is_object_id("0123456789abcdef0123456g"));
    }

    #[test]
    fn is_object_id_rejects_forward_slash() {
        assert!(!is_object_id("01234567/9abcdef012345678"));
    }

    #[test]
    fn is_object_id_rejects_dot_segment() {
        assert!(!is_object_id("..0123456789abcdef0123456"));
    }

    #[test]
    fn is_object_id_rejects_null_byte() {
        assert!(!is_object_id("0123456789abcdef0123456\0"));
    }

    #[test]
    fn is_safe_relative_key_rejects_empty() {
        assert!(!is_safe_relative_key(""));
    }

    #[test]
    fn is_safe_relative_key_rejects_single_dot() {
        assert!(!is_safe_relative_key("."));
    }

    #[test]
    fn is_safe_relative_key_rejects_parent_dir() {
        assert!(!is_safe_relative_key(".."));
    }

    #[test]
    fn is_safe_relative_key_rejects_leading_parent_dir() {
        assert!(!is_safe_relative_key("../etc/passwd"));
    }

    #[test]
    fn is_safe_relative_key_rejects_embedded_parent_dir() {
        assert!(!is_safe_relative_key("owner/../../etc/passwd"));
    }

    #[test]
    fn is_safe_relative_key_rejects_unix_absolute() {
        assert!(!is_safe_relative_key("/etc/passwd"));
    }

    #[test]
    fn is_safe_relative_key_rejects_trailing_parent_dir() {
        assert!(!is_safe_relative_key("owner/node/.."));
    }

    #[test]
    fn is_safe_relative_key_accepts_simple_segment() {
        assert!(is_safe_relative_key("file"));
    }

    #[test]
    fn is_safe_relative_key_accepts_owner_node_pair() {
        assert!(is_safe_relative_key(
            "0123456789abcdef01234567/11111111-1111-1111-1111-111111111111"
        ));
    }

    #[test]
    fn is_safe_relative_key_accepts_thumb_suffix() {
        assert!(is_safe_relative_key(
            "0123456789abcdef01234567/11111111-1111-1111-1111-111111111111.thumb"
        ));
    }

    #[test]
    fn build_key_accepts_valid_owner_id() {
        let node_id = uuid::Uuid::nil();
        let key = FsStorage::build_key(VALID_OWNER_ID, node_id).unwrap();
        assert_eq!(key, format!("{VALID_OWNER_ID}/{node_id}"));
    }

    #[test]
    fn build_key_rejects_parent_traversal_owner_id() {
        let err = FsStorage::build_key("../../etc", uuid::Uuid::nil()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn build_key_rejects_absolute_owner_id() {
        let err = FsStorage::build_key("/abs/path", uuid::Uuid::nil()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn build_key_rejects_dot_dot_owner_id() {
        let err = FsStorage::build_key("..", uuid::Uuid::nil()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn build_key_rejects_empty_owner_id() {
        let err = FsStorage::build_key("", uuid::Uuid::nil()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn build_key_rejects_owner_id_with_slash() {
        let err = FsStorage::build_key("owner/sub", uuid::Uuid::nil()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn build_key_rejects_owner_id_with_null_byte() {
        let err = FsStorage::build_key("0123456789abcdef0123456\0", uuid::Uuid::nil()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn resolve_accepts_valid_key_under_root() {
        let store = storage();
        let key = format!("{VALID_OWNER_ID}/{}", uuid::Uuid::nil());
        let path = store.resolve(&key).unwrap();
        assert!(path.starts_with("/srv/drive/data"));
    }

    #[test]
    fn resolve_rejects_empty_key() {
        let err = storage().resolve("").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn resolve_rejects_single_dot_key() {
        let err = storage().resolve(".").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn resolve_rejects_parent_traversal_key() {
        let err = storage().resolve("../../etc/passwd").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn resolve_rejects_embedded_parent_traversal_key() {
        let err = storage().resolve("owner/../../etc/passwd").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn resolve_rejects_unix_absolute_key() {
        let err = storage().resolve("/etc/passwd").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn resolve_rejects_trailing_parent_key() {
        let err = storage().resolve("owner/node/..").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn resolve_rejects_null_byte_key() {
        let err = storage().resolve("owner/node\0").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn resolve_rejects_traversal_escaping_root() {
        let err = storage().resolve("owner/../../../etc/passwd").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn resolve_valid_key_stays_strictly_under_root() {
        let store = storage();
        let root = PathBuf::from("/srv/drive/data");
        let key = format!("{VALID_OWNER_ID}/{}", uuid::Uuid::nil());
        let path = store.resolve(&key).unwrap();
        assert!(path.starts_with(&root));
        assert_ne!(path, root);
        assert_eq!(
            path,
            root.join(VALID_OWNER_ID).join(uuid::Uuid::nil().to_string())
        );
    }

    #[test]
    fn is_safe_relative_key_rejects_windows_separator() {
        assert!(!is_safe_relative_key("owner\\sub"));
    }

    #[test]
    fn is_safe_relative_key_rejects_windows_parent_traversal() {
        assert!(!is_safe_relative_key("..\\..\\etc"));
    }

    #[test]
    fn is_safe_relative_key_rejects_windows_drive_absolute() {
        assert!(!is_safe_relative_key("C:\\Windows"));
    }

    #[test]
    fn is_safe_relative_key_rejects_mixed_separators() {
        assert!(!is_safe_relative_key("..\\../etc"));
    }

    #[test]
    fn build_key_rejects_owner_id_with_backslash() {
        let err = FsStorage::build_key("owner\\sub", uuid::Uuid::nil()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn resolve_rejects_windows_separator_key() {
        let err = storage().resolve("owner\\sub").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn resolve_rejects_windows_parent_traversal_key() {
        let err = storage().resolve("..\\..\\etc").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn resolve_rejects_windows_drive_absolute_key() {
        let err = storage().resolve("C:\\Windows").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn resolve_rejects_mixed_separators_key() {
        let err = storage().resolve("..\\../etc").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }
}
