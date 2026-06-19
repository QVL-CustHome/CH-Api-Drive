use ch_api_drive::services::storage::FsStorage;
use std::path::PathBuf;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().unwrap()
}

fn unique(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("us14ac2_{}_{}_{}", tag, std::process::id(), uuid::Uuid::new_v4()));
    p
}

#[test]
fn ac2_sibling_dir_sharing_textual_prefix_is_not_treated_as_inside_root() {
    let base = unique("prefix");
    let root = base.join("data");
    std::fs::create_dir_all(&root).unwrap();
    let sibling = base.join("data_evil");
    std::fs::create_dir_all(&sibling).unwrap();

    let store = FsStorage::new(root.clone());
    let res = rt().block_on(store.write_bytes("../data_evil/leak.txt", b"x"));

    let leaked = sibling.join("leak.txt").exists();
    let _ = std::fs::remove_dir_all(&base);

    assert!(res.is_err(), "ecriture vers dossier frere a prefixe partage acceptee");
    assert!(!leaked, "fichier ecrit dans un dossier frere partageant le prefixe textuel du root");
}

#[test]
fn ac2_existing_nested_legit_path_stays_inside_root() {
    let root = unique("legit");
    std::fs::create_dir_all(&root).unwrap();
    let owner = "0123456789abcdef01234567";
    let node = uuid::Uuid::new_v4();
    let key = FsStorage::build_key(owner, node).unwrap();
    let store = FsStorage::new(root.clone());
    let res = rt().block_on(store.write_bytes(&key, b"y"));
    let inside = root.join(owner).join(node.to_string()).exists();
    let _ = std::fs::remove_dir_all(&root);
    assert!(res.is_ok());
    assert!(inside);
}
