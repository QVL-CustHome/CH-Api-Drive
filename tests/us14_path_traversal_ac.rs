use ch_api_drive::services::storage::{is_object_id, FsStorage};
use std::io::ErrorKind;
use std::path::PathBuf;

const VALID_OWNER: &str = "0123456789abcdef01234567";

fn temp_root(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "us14_{}_{}_{}",
        tag,
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn outside_target(root: &PathBuf, name: &str) -> PathBuf {
    let parent = root.parent().unwrap();
    parent.join(name)
}

mod ac1_owner_id_validation {
    use super::*;

    #[test]
    fn accepts_legitimate_object_id() {
        assert!(FsStorage::build_key(VALID_OWNER, uuid::Uuid::nil()).is_ok());
    }

    #[test]
    fn rejects_dotdot_only() {
        let err = FsStorage::build_key("..", uuid::Uuid::nil()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn rejects_relative_traversal_unix() {
        let err = FsStorage::build_key("../../etc", uuid::Uuid::nil()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn rejects_relative_traversal_windows() {
        let err = FsStorage::build_key("..\\..\\windows", uuid::Uuid::nil()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn rejects_forward_slash_segment() {
        let err = FsStorage::build_key("owner/sub", uuid::Uuid::nil()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn rejects_backslash_segment() {
        let err = FsStorage::build_key("owner\\sub", uuid::Uuid::nil()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn rejects_unix_absolute_segment() {
        let err = FsStorage::build_key("/abs/path", uuid::Uuid::nil()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn rejects_windows_drive_absolute_segment() {
        let err = FsStorage::build_key("C:\\Windows", uuid::Uuid::nil()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn rejects_unc_path_segment() {
        let err = FsStorage::build_key("\\\\server\\share", uuid::Uuid::nil()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn rejects_empty_segment() {
        let err = FsStorage::build_key("", uuid::Uuid::nil()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn rejects_null_byte_segment() {
        let err = FsStorage::build_key("0123456789abcdef0123456\0", uuid::Uuid::nil()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn rejects_mixed_separators_in_segment() {
        let err = FsStorage::build_key("..\\../etc", uuid::Uuid::nil()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn object_id_rejects_backslash() {
        assert!(!is_object_id("0123456789abcdef0123456\\"));
    }

    #[test]
    fn object_id_rejects_forward_slash() {
        assert!(!is_object_id("0123456789abcdef0123456/"));
    }
}

mod ac3_write_traversal_blocked {
    use super::*;

    fn attempt_write(root_tag: &str, key: &str) -> std::io::Result<()> {
        let root = temp_root(root_tag);
        let store = FsStorage::new(root.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let res = rt.block_on(store.write_bytes(key, b"pwned"));
        let _ = std::fs::remove_dir_all(&root);
        res
    }

    fn attempt_write_with_sentinel(root_tag: &str, sentinel_name: &str, key: &str) -> bool {
        let root = temp_root(root_tag);
        let sentinel = outside_target(&root, sentinel_name);
        let _ = std::fs::remove_file(&sentinel);
        let store = FsStorage::new(root.clone());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _ = rt.block_on(store.write_bytes(key, b"pwned"));
        let escaped = sentinel.exists();
        let _ = std::fs::remove_file(&sentinel);
        let _ = std::fs::remove_dir_all(&root);
        escaped
    }

    #[test]
    fn write_rejects_relative_traversal_unix() {
        let err = attempt_write("w_rel_unix", "../escape.txt").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_rejects_relative_traversal_windows() {
        let err = attempt_write("w_rel_win", "..\\escape.txt").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_rejects_embedded_traversal() {
        let err = attempt_write("w_emb", "owner/../../escape.txt").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_rejects_embedded_traversal_backslash() {
        let err = attempt_write("w_emb_bs", "owner\\..\\..\\escape.txt").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_rejects_unix_absolute() {
        let err = attempt_write("w_abs_unix", "/etc/escape.txt").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_rejects_windows_drive_absolute() {
        let err = attempt_write("w_abs_win", "C:\\Windows\\escape.txt").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_rejects_null_byte() {
        let err = attempt_write("w_null", "owner/node\0").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_does_not_create_file_outside_root_unix_style() {
        let escaped = attempt_write_with_sentinel("s_unix", "sentinel_unix.txt", "../sentinel_unix.txt");
        assert!(!escaped, "fichier ecrit hors du root via ../");
    }

    #[test]
    fn write_does_not_create_file_outside_root_windows_style() {
        let escaped = attempt_write_with_sentinel("s_win", "sentinel_win.txt", "..\\sentinel_win.txt");
        assert!(!escaped, "fichier ecrit hors du root via ..\\");
    }
}

mod ac3_read_delete_metadata_blocked {
    use super::*;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Runtime::new().unwrap()
    }

    #[test]
    fn open_rejects_traversal_without_touching_fs() {
        let root = temp_root("o_trav");
        let outside = outside_target(&root, "secret_open.txt");
        std::fs::write(&outside, b"top-secret").unwrap();
        let store = FsStorage::new(root.clone());
        let res = rt().block_on(store.open("../secret_open.txt"));
        let leaked = res.is_ok();
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&root);
        assert!(!leaked, "lecture hors root reussie via ../");
    }

    #[test]
    fn open_rejects_windows_traversal() {
        let root = temp_root("o_trav_win");
        let outside = outside_target(&root, "secret_open_win.txt");
        std::fs::write(&outside, b"top-secret").unwrap();
        let store = FsStorage::new(root.clone());
        let res = rt().block_on(store.open("..\\secret_open_win.txt"));
        let leaked = res.is_ok();
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&root);
        assert!(!leaked, "lecture hors root reussie via ..\\");
    }

    #[test]
    fn delete_rejects_traversal_without_touching_fs() {
        let root = temp_root("d_trav");
        let outside = outside_target(&root, "victim_delete.txt");
        std::fs::write(&outside, b"do-not-delete").unwrap();
        let store = FsStorage::new(root.clone());
        let _ = rt().block_on(store.delete("../victim_delete.txt"));
        let survived = outside.exists();
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&root);
        assert!(survived, "fichier hors root supprime via ../");
    }

    #[test]
    fn metadata_rejects_traversal() {
        let root = temp_root("m_trav");
        let outside = outside_target(&root, "probe_meta.txt");
        std::fs::write(&outside, b"probe").unwrap();
        let store = FsStorage::new(root.clone());
        let res = rt().block_on(store.metadata("../probe_meta.txt"));
        let leaked = res.is_ok();
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&root);
        assert!(!leaked, "metadata hors root obtenue via ../");
    }
}

mod ac4_nominal_not_broken {
    use super::*;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Runtime::new().unwrap()
    }

    #[test]
    fn nominal_write_read_roundtrip() {
        let root = temp_root("nom_rw");
        let store = FsStorage::new(root.clone());
        let node = uuid::Uuid::new_v4();
        let key = FsStorage::build_key(VALID_OWNER, node).unwrap();
        let payload = b"hello-drive";
        rt().block_on(store.write_bytes(&key, payload)).unwrap();
        let meta = rt().block_on(store.metadata(&key)).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(meta.len(), payload.len() as u64);
    }

    #[test]
    fn nominal_thumb_key_resolves_and_writes() {
        let root = temp_root("nom_thumb");
        let store = FsStorage::new(root.clone());
        let node = uuid::Uuid::new_v4();
        let key = FsStorage::build_key(VALID_OWNER, node).unwrap();
        let thumb = FsStorage::thumb_key(&key);
        let res = rt().block_on(store.write_bytes(&thumb, b"thumbdata"));
        let _ = std::fs::remove_dir_all(&root);
        assert!(res.is_ok());
    }

    #[test]
    fn nominal_written_file_is_inside_root() {
        let root = temp_root("nom_inside");
        let store = FsStorage::new(root.clone());
        let node = uuid::Uuid::new_v4();
        let key = FsStorage::build_key(VALID_OWNER, node).unwrap();
        rt().block_on(store.write_bytes(&key, b"x")).unwrap();
        let expected = root.join(VALID_OWNER).join(node.to_string());
        let exists = expected.exists();
        let _ = std::fs::remove_dir_all(&root);
        assert!(exists, "le fichier nominal n'est pas a l'emplacement attendu sous root");
    }

    #[test]
    fn nominal_delete_is_idempotent_on_missing() {
        let root = temp_root("nom_del");
        let store = FsStorage::new(root.clone());
        let node = uuid::Uuid::new_v4();
        let key = FsStorage::build_key(VALID_OWNER, node).unwrap();
        let res = rt().block_on(store.delete(&key));
        let _ = std::fs::remove_dir_all(&root);
        assert!(res.is_ok());
    }
}
