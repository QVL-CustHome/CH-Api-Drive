use ch_api_drive::services::storage::FsStorage;
use std::io::ErrorKind;
use std::path::PathBuf;

const VALID_OWNER: &str = "0123456789abcdef01234567";

fn temp_root(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "scrum187_{}_{}_{}",
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

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().unwrap()
}

mod ac1_server_key_never_derived_from_client_name {
    use super::*;

    #[test]
    fn build_key_produces_server_key_from_owner_and_node_uuid() {
        let node_id = uuid::Uuid::new_v4();
        let key = FsStorage::build_key(VALID_OWNER, node_id).unwrap();
        assert_eq!(key, format!("{VALID_OWNER}/{node_id}"));
        assert!(!key.contains("facture"));
    }

    #[test]
    fn write_at_operates_on_server_key_and_ignores_client_filename() {
        let root = temp_root("ac1_write_at_serverkey");
        let store = FsStorage::new(root.clone());
        let node_id = uuid::Uuid::new_v4();
        let server_key = FsStorage::build_key(VALID_OWNER, node_id).unwrap();

        let written = rt().block_on(store.write_at(&server_key, 0, b"data"));
        assert!(written.is_ok());

        let expected_path = root.join(VALID_OWNER).join(node_id.to_string());
        assert!(expected_path.exists());
        let client_named_path = root.join("vacances 2024.jpg");
        assert!(!client_named_path.exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn finalize_operates_on_server_keys_only() {
        let root = temp_root("ac1_finalize_serverkey");
        let store = FsStorage::new(root.clone());
        let node_id = uuid::Uuid::new_v4();
        let tmp_key = format!("{VALID_OWNER}/{node_id}.part");
        let storage_key = FsStorage::build_key(VALID_OWNER, node_id).unwrap();

        rt().block_on(store.write_at(&tmp_key, 0, b"payload")).unwrap();
        rt().block_on(store.finalize(&tmp_key, &storage_key)).unwrap();

        let final_path = root.join(VALID_OWNER).join(node_id.to_string());
        assert!(final_path.exists());
        assert!(!root.join(&tmp_key).exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn client_filename_with_traversal_is_not_usable_as_key() {
        let root = temp_root("ac1_client_traversal_key");
        let store = FsStorage::new(root.clone());
        let malicious_client_name = "../../etc/passwd";
        let err = rt()
            .block_on(store.write_at(malicious_client_name, 0, b"x"))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
        let _ = std::fs::remove_dir_all(&root);
    }
}

mod ac2_write_at_traversal_blocked {
    use super::*;

    fn attempt(tag: &str, key: &str) -> std::io::Result<()> {
        let root = temp_root(tag);
        let store = FsStorage::new(root.clone());
        let res = rt().block_on(store.write_at(key, 0, b"pwned"));
        let _ = std::fs::remove_dir_all(&root);
        res
    }

    fn attempt_with_sentinel(tag: &str, sentinel_name: &str, key: &str) -> bool {
        let root = temp_root(tag);
        let sentinel = outside_target(&root, sentinel_name);
        let _ = std::fs::remove_file(&sentinel);
        let store = FsStorage::new(root.clone());
        let _ = rt().block_on(store.write_at(key, 0, b"pwned"));
        let escaped = sentinel.exists();
        let _ = std::fs::remove_file(&sentinel);
        let _ = std::fs::remove_dir_all(&root);
        escaped
    }

    #[test]
    fn write_at_rejects_parent_traversal_unix() {
        let err = attempt("wa_rel_unix", "../escape.bin").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_at_rejects_parent_traversal_windows() {
        let err = attempt("wa_rel_win", "..\\escape.bin").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_at_rejects_embedded_traversal() {
        let err = attempt("wa_emb", "owner/../../escape.bin").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_at_rejects_unix_absolute() {
        let err = attempt("wa_abs_unix", "/etc/escape.bin").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_at_rejects_windows_drive_absolute() {
        let err = attempt("wa_abs_win", "C:\\Windows\\escape.bin").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_at_rejects_null_byte() {
        let err = attempt("wa_null", "owner/node\0").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_at_does_not_escape_root_unix_style() {
        let escaped = attempt_with_sentinel("wa_s_unix", "scrum187_wa_unix.txt", "../scrum187_wa_unix.txt");
        assert!(!escaped, "write_at a ecrit hors du root via ../");
    }

    #[test]
    fn write_at_does_not_escape_root_windows_style() {
        let escaped = attempt_with_sentinel("wa_s_win", "scrum187_wa_win.txt", "..\\scrum187_wa_win.txt");
        assert!(!escaped, "write_at a ecrit hors du root via ..\\");
    }
}

mod ac2_write_at_reserved_windows_names_blocked {
    use super::*;

    fn attempt(tag: &str, key: &str) -> std::io::Result<()> {
        let root = temp_root(tag);
        let store = FsStorage::new(root.clone());
        let res = rt().block_on(store.write_at(key, 0, b"pwned"));
        let _ = std::fs::remove_dir_all(&root);
        res
    }

    #[test]
    fn write_at_rejects_reserved_con() {
        let err = attempt("wa_con", "CON").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_at_rejects_reserved_prn() {
        let err = attempt("wa_prn", "PRN").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_at_rejects_reserved_aux() {
        let err = attempt("wa_aux", "AUX").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_at_rejects_reserved_nul() {
        let err = attempt("wa_nul", "NUL").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_at_rejects_reserved_com1() {
        let err = attempt("wa_com1", "COM1").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_at_rejects_reserved_lpt1() {
        let err = attempt("wa_lpt1", "LPT1").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_at_rejects_reserved_nested_in_owner_segment() {
        let err = attempt("wa_nested_con", &format!("{VALID_OWNER}/CON")).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }
}

mod ac2_finalize_traversal_blocked {
    use super::*;

    fn prepare_tmp(root: &PathBuf, store: &FsStorage) -> String {
        let node_id = uuid::Uuid::new_v4();
        let tmp_key = format!("{VALID_OWNER}/{node_id}.part");
        let _ = std::fs::create_dir_all(root.join(VALID_OWNER));
        rt().block_on(store.write_at(&tmp_key, 0, b"payload")).unwrap();
        tmp_key
    }

    fn attempt_destination(tag: &str, dest_key: &str) -> std::io::Result<()> {
        let root = temp_root(tag);
        let store = FsStorage::new(root.clone());
        let tmp_key = prepare_tmp(&root, &store);
        let res = rt().block_on(store.finalize(&tmp_key, dest_key));
        let _ = std::fs::remove_dir_all(&root);
        res
    }

    fn attempt_source(tag: &str, src_key: &str) -> std::io::Result<()> {
        let root = temp_root(tag);
        let store = FsStorage::new(root.clone());
        let node_id = uuid::Uuid::new_v4();
        let dest_key = format!("{VALID_OWNER}/{node_id}");
        let res = rt().block_on(store.finalize(src_key, &dest_key));
        let _ = std::fs::remove_dir_all(&root);
        res
    }

    #[test]
    fn finalize_rejects_destination_parent_traversal() {
        let err = attempt_destination("fin_dest_rel", "../escape.bin").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn finalize_rejects_destination_embedded_traversal() {
        let err = attempt_destination("fin_dest_emb", "owner/../../escape.bin").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn finalize_rejects_destination_unix_absolute() {
        let err = attempt_destination("fin_dest_abs", "/etc/escape.bin").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn finalize_rejects_destination_windows_drive_absolute() {
        let err = attempt_destination("fin_dest_win", "C:\\Windows\\escape.bin").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn finalize_rejects_destination_null_byte() {
        let err = attempt_destination("fin_dest_null", "owner/node\0").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn finalize_rejects_destination_reserved_windows_name() {
        let err = attempt_destination("fin_dest_con", "CON").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn finalize_rejects_source_parent_traversal() {
        let err = attempt_source("fin_src_rel", "../../etc/passwd").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn finalize_rejects_source_windows_drive_absolute() {
        let err = attempt_source("fin_src_win", "C:\\Windows\\system.ini").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }
}

mod ac2_functional_write_at_offset {
    use super::*;

    #[test]
    fn write_at_offset_zero_writes_bytes_verbatim() {
        let root = temp_root("fn_off0");
        let store = FsStorage::new(root.clone());
        let node_id = uuid::Uuid::new_v4();
        let key = format!("{VALID_OWNER}/{node_id}");
        rt().block_on(store.write_at(&key, 0, b"hello")).unwrap();
        let content = std::fs::read(root.join(VALID_OWNER).join(node_id.to_string())).unwrap();
        assert_eq!(content, b"hello");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_at_positive_offset_places_bytes_at_offset() {
        let root = temp_root("fn_offpos");
        let store = FsStorage::new(root.clone());
        let node_id = uuid::Uuid::new_v4();
        let key = format!("{VALID_OWNER}/{node_id}");
        rt().block_on(store.write_at(&key, 4, b"WXYZ")).unwrap();
        let content = std::fs::read(root.join(VALID_OWNER).join(node_id.to_string())).unwrap();
        assert_eq!(content.len(), 8);
        assert_eq!(&content[4..8], b"WXYZ");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_at_two_chunks_reassemble_in_order() {
        let root = temp_root("fn_chunks");
        let store = FsStorage::new(root.clone());
        let node_id = uuid::Uuid::new_v4();
        let key = format!("{VALID_OWNER}/{node_id}");
        rt().block_on(store.write_at(&key, 0, b"AAAA")).unwrap();
        rt().block_on(store.write_at(&key, 4, b"BBBB")).unwrap();
        let content = std::fs::read(root.join(VALID_OWNER).join(node_id.to_string())).unwrap();
        assert_eq!(content, b"AAAABBBB");
        let _ = std::fs::remove_dir_all(&root);
    }
}

mod ac2_functional_finalize_atomic {
    use super::*;

    #[test]
    fn finalize_renames_tmp_to_final_and_removes_tmp() {
        let root = temp_root("fin_ok");
        let store = FsStorage::new(root.clone());
        let node_id = uuid::Uuid::new_v4();
        let tmp_key = format!("{VALID_OWNER}/{node_id}.part");
        let storage_key = format!("{VALID_OWNER}/{node_id}");
        rt().block_on(store.write_at(&tmp_key, 0, b"final-content")).unwrap();

        rt().block_on(store.finalize(&tmp_key, &storage_key)).unwrap();

        let final_path = root.join(VALID_OWNER).join(node_id.to_string());
        let tmp_path = root.join(VALID_OWNER).join(format!("{node_id}.part"));
        assert!(final_path.exists());
        assert!(!tmp_path.exists());
        assert_eq!(std::fs::read(&final_path).unwrap(), b"final-content");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn finalize_creates_destination_parent_directory() {
        let root = temp_root("fin_mkparent");
        let store = FsStorage::new(root.clone());
        let node_id = uuid::Uuid::new_v4();
        let tmp_key = format!("{VALID_OWNER}/{node_id}.part");
        let other_owner = "fedcba9876543210fedcba98";
        let storage_key = format!("{other_owner}/{node_id}");
        rt().block_on(store.write_at(&tmp_key, 0, b"x")).unwrap();

        rt().block_on(store.finalize(&tmp_key, &storage_key)).unwrap();

        assert!(root.join(other_owner).join(node_id.to_string()).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn finalize_fails_when_source_missing() {
        let root = temp_root("fin_nosrc");
        let store = FsStorage::new(root.clone());
        let node_id = uuid::Uuid::new_v4();
        let missing_tmp = format!("{VALID_OWNER}/{node_id}.part");
        let storage_key = format!("{VALID_OWNER}/{node_id}");
        let err = rt()
            .block_on(store.finalize(&missing_tmp, &storage_key))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        let _ = std::fs::remove_dir_all(&root);
    }
}

mod c2_technical_robustness {
    use super::*;

    #[test]
    fn write_at_sequential_offsets_preserve_previous_chunks() {
        let root = temp_root("tech_seq_offsets");
        let store = FsStorage::new(root.clone());
        let node_id = uuid::Uuid::new_v4();
        let key = format!("{VALID_OWNER}/{node_id}");

        rt().block_on(store.write_at(&key, 0, b"AAAA")).unwrap();
        rt().block_on(store.write_at(&key, 4, b"BBBB")).unwrap();
        rt().block_on(store.write_at(&key, 8, b"CCCC")).unwrap();
        rt().block_on(store.write_at(&key, 12, b"DDDD")).unwrap();

        let content = std::fs::read(root.join(VALID_OWNER).join(node_id.to_string())).unwrap();
        assert_eq!(content, b"AAAABBBBCCCCDDDD");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_at_later_chunk_does_not_truncate_existing_content() {
        let root = temp_root("tech_no_truncate");
        let store = FsStorage::new(root.clone());
        let node_id = uuid::Uuid::new_v4();
        let key = format!("{VALID_OWNER}/{node_id}");

        rt().block_on(store.write_at(&key, 0, b"0123456789")).unwrap();
        rt().block_on(store.write_at(&key, 10, b"ABCDE")).unwrap();

        let content = std::fs::read(root.join(VALID_OWNER).join(node_id.to_string())).unwrap();
        assert_eq!(content, b"0123456789ABCDE");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_at_reopen_keeps_earlier_offsets_intact() {
        let root = temp_root("tech_reopen");
        let store = FsStorage::new(root.clone());
        let node_id = uuid::Uuid::new_v4();
        let key = format!("{VALID_OWNER}/{node_id}");

        rt().block_on(store.write_at(&key, 0, b"HEAD")).unwrap();
        let store_reopened = FsStorage::new(root.clone());
        rt().block_on(store_reopened.write_at(&key, 4, b"TAIL")).unwrap();

        let content = std::fs::read(root.join(VALID_OWNER).join(node_id.to_string())).unwrap();
        assert_eq!(content, b"HEADTAIL");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn finalize_round_trip_preserves_assembled_payload() {
        let root = temp_root("tech_finalize_roundtrip");
        let store = FsStorage::new(root.clone());
        let node_id = uuid::Uuid::new_v4();
        let tmp_key = format!("{VALID_OWNER}/{node_id}.part");
        let storage_key = format!("{VALID_OWNER}/{node_id}");

        rt().block_on(store.write_at(&tmp_key, 0, b"chunk-one-")).unwrap();
        rt().block_on(store.write_at(&tmp_key, 10, b"chunk-two")).unwrap();
        rt().block_on(store.finalize(&tmp_key, &storage_key)).unwrap();

        let final_path = root.join(VALID_OWNER).join(node_id.to_string());
        let tmp_path = root.join(VALID_OWNER).join(format!("{node_id}.part"));
        assert!(final_path.exists());
        assert!(!tmp_path.exists());
        assert_eq!(std::fs::read(&final_path).unwrap(), b"chunk-one-chunk-two");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn reject_reserved(tag: &str, key: &str) -> ErrorKind {
        let root = temp_root(tag);
        let store = FsStorage::new(root.clone());
        let err = rt().block_on(store.write_at(key, 0, b"x")).unwrap_err();
        let _ = std::fs::remove_dir_all(&root);
        err.kind()
    }

    #[test]
    fn write_at_rejects_reserved_nul() {
        assert_eq!(reject_reserved("tech_nul", "NUL"), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_at_rejects_reserved_con() {
        assert_eq!(reject_reserved("tech_con", "CON"), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_at_rejects_reserved_com1() {
        assert_eq!(reject_reserved("tech_com1", "COM1"), ErrorKind::PermissionDenied);
    }

    #[test]
    fn write_at_rejects_reserved_nul_with_extension() {
        assert_eq!(
            reject_reserved("tech_nul_ext", "NUL.txt"),
            ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn write_at_rejects_reserved_name_in_owner_component() {
        assert_eq!(
            reject_reserved("tech_nested_nul", &format!("{VALID_OWNER}/NUL")),
            ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn write_at_accepts_name_containing_reserved_substring() {
        let root = temp_root("tech_substring_ok");
        let store = FsStorage::new(root.clone());
        let key = format!("{VALID_OWNER}/COM10");
        rt().block_on(store.write_at(&key, 0, b"ok")).unwrap();
        assert!(root.join(VALID_OWNER).join("COM10").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
