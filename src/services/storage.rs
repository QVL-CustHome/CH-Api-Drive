use std::path::PathBuf;
use tokio::fs;

#[derive(Clone)]
pub struct FsStorage {
    root: PathBuf,
}

impl FsStorage {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn build_key(owner_id: &str, node_id: uuid::Uuid) -> String {
        format!("{owner_id}/{node_id}")
    }

    pub fn thumb_key(storage_key: &str) -> String {
        format!("{storage_key}.thumb")
    }

    pub async fn write_bytes(&self, key: &str, bytes: &[u8]) -> std::io::Result<()> {
        let path = self.resolve(key);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(path, bytes).await
    }

    fn resolve(&self, key: &str) -> PathBuf {
        self.root.join(key)
    }

    pub async fn create_writer(&self, key: &str) -> std::io::Result<fs::File> {
        let path = self.resolve(key);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::File::create(path).await
    }

    pub async fn open(&self, key: &str) -> std::io::Result<fs::File> {
        fs::File::open(self.resolve(key)).await
    }

    pub async fn delete(&self, key: &str) -> std::io::Result<()> {
        match fs::remove_file(self.resolve(key)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub async fn metadata(&self, key: &str) -> std::io::Result<std::fs::Metadata> {
        fs::metadata(self.resolve(key)).await
    }
}
