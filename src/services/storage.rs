use std::future::Future;
use std::io::{Error, ErrorKind, Result, SeekFrom};
use std::path::{Component, Path, PathBuf};
use tokio::fs;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

pub trait Storage: Clone + Send + Sync {
    fn write_bytes(&self, key: &str, bytes: &[u8]) -> impl Future<Output = Result<()>> + Send;

    fn write_at(
        &self,
        key: &str,
        offset: u64,
        bytes: &[u8],
    ) -> impl Future<Output = Result<()>> + Send;

    fn create_writer(&self, key: &str) -> impl Future<Output = Result<fs::File>> + Send;

    fn open(&self, key: &str) -> impl Future<Output = Result<fs::File>> + Send;

    fn finalize(&self, tmp_key: &str, storage_key: &str)
    -> impl Future<Output = Result<()>> + Send;

    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send;

    fn metadata(&self, key: &str) -> impl Future<Output = Result<std::fs::Metadata>> + Send;
}

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

    pub async fn write_at(&self, key: &str, offset: u64, bytes: &[u8]) -> Result<()> {
        let path = self.resolve(key)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .read(false)
            .truncate(false)
            .open(path)
            .await?;
        file.seek(SeekFrom::Start(offset)).await?;
        file.write_all(bytes).await?;
        file.flush().await?;
        file.sync_data().await
    }

    pub async fn finalize(&self, tmp_key: &str, storage_key: &str) -> Result<()> {
        let source = self.resolve(tmp_key)?;
        let destination = self.resolve(storage_key)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).await?;
        }
        match fs::rename(&source, &destination).await {
            Ok(()) => Ok(()),
            Err(e) if is_cross_device(&e) => {
                finalize_across_devices(&source, &destination).await
            }
            Err(e) => Err(e),
        }
    }
}

const EXDEV: i32 = 18;

fn is_cross_device(error: &Error) -> bool {
    error.raw_os_error() == Some(EXDEV)
}

async fn finalize_across_devices(source: &Path, destination: &Path) -> Result<()> {
    fs::copy(source, destination).await?;
    let destination_file = fs::File::open(destination).await?;
    destination_file.sync_all().await?;
    fs::remove_file(source).await
}

impl Storage for FsStorage {
    async fn write_bytes(&self, key: &str, bytes: &[u8]) -> Result<()> {
        FsStorage::write_bytes(self, key, bytes).await
    }

    async fn write_at(&self, key: &str, offset: u64, bytes: &[u8]) -> Result<()> {
        FsStorage::write_at(self, key, offset, bytes).await
    }

    async fn create_writer(&self, key: &str) -> Result<fs::File> {
        FsStorage::create_writer(self, key).await
    }

    async fn open(&self, key: &str) -> Result<fs::File> {
        FsStorage::open(self, key).await
    }

    async fn finalize(&self, tmp_key: &str, storage_key: &str) -> Result<()> {
        FsStorage::finalize(self, tmp_key, storage_key).await
    }

    async fn delete(&self, key: &str) -> Result<()> {
        FsStorage::delete(self, key).await
    }

    async fn metadata(&self, key: &str) -> Result<std::fs::Metadata> {
        FsStorage::metadata(self, key).await
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
    path.components().all(|component| match component {
        Component::Normal(segment) => match segment.to_str() {
            Some(name) => !is_reserved_windows_name(name),
            None => false,
        },
        _ => false,
    })
}

const RESERVED_WINDOWS_DEVICE_STEMS: [&str; 25] = [
    "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
    "COM8", "COM9", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    "CLOCK$",
];

fn is_reserved_windows_name(segment: &str) -> bool {
    let stem = match segment.split_once('.') {
        Some((before_extension, _)) => before_extension,
        None => segment,
    };
    RESERVED_WINDOWS_DEVICE_STEMS
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
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

    #[test]
    fn is_safe_relative_key_rejects_reserved_nul() {
        assert!(!is_safe_relative_key("NUL"));
    }

    #[test]
    fn is_safe_relative_key_rejects_reserved_con() {
        assert!(!is_safe_relative_key("CON"));
    }

    #[test]
    fn is_safe_relative_key_rejects_reserved_com1() {
        assert!(!is_safe_relative_key("COM1"));
    }

    #[test]
    fn is_safe_relative_key_rejects_reserved_name_case_insensitive() {
        assert!(!is_safe_relative_key("nul"));
        assert!(!is_safe_relative_key("Com1"));
    }

    #[test]
    fn is_safe_relative_key_rejects_reserved_name_with_extension() {
        assert!(!is_safe_relative_key("NUL.txt"));
        assert!(!is_safe_relative_key("con.log"));
    }

    #[test]
    fn is_safe_relative_key_rejects_reserved_name_in_any_component() {
        assert!(!is_safe_relative_key("0123456789abcdef01234567/NUL"));
        assert!(!is_safe_relative_key("NUL/0123456789abcdef01234567"));
    }

    #[test]
    fn is_safe_relative_key_rejects_reserved_clock_device() {
        assert!(!is_safe_relative_key("CLOCK$"));
    }

    #[test]
    fn is_safe_relative_key_accepts_name_containing_reserved_as_substring() {
        assert!(is_safe_relative_key("CONTRACT"));
        assert!(is_safe_relative_key("COM10"));
        assert!(is_safe_relative_key("NULLABLE"));
    }

    #[test]
    fn is_reserved_windows_name_matches_bare_device() {
        assert!(is_reserved_windows_name("NUL"));
        assert!(is_reserved_windows_name("lpt9"));
    }

    #[test]
    fn is_reserved_windows_name_ignores_unrelated_segment() {
        assert!(!is_reserved_windows_name("vacances.jpg"));
        assert!(!is_reserved_windows_name("nuls"));
    }
}
