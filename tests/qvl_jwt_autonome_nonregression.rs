use ch_api_drive::services::jwt::{Claims, JwtService};
use ch_api_drive::services::storage::is_object_id;
use std::time::Duration;

const SECRET: &str = "secret-de-test-suffisamment-long-pour-hs256";
const AUTRE_SECRET: &str = "un-autre-secret-tout-aussi-long-pour-hs256";
const ISSUER: &str = "ch-api-authenticator";
const AUDIENCE: &str = "ch-api-drive";
const SUB_OBJECT_ID: &str = "0123456789abcdef01234567";
const SUB_NON_OBJECT_ID: &str = "alice@example.com";

fn service() -> JwtService {
    JwtService::from_secret(SECRET, ISSUER, AUDIENCE)
}

#[test]
fn token_valide_est_decode_et_restitue_les_claims() {
    let service = service();
    let token = service
        .issue(SUB_OBJECT_ID, vec!["drive_user".to_string()], None, Duration::from_secs(3600))
        .expect("emission du token");

    let claims = service.decode(&token).expect("decodage du token valide");

    assert_eq!(claims.sub, SUB_OBJECT_ID);
    assert!(claims.has_role("drive_user"));
}

#[test]
fn token_expire_est_rejete() {
    let service = service();
    let now = ch_api_drive::services::jwt::unix_now();
    let claims = Claims::new(
        SUB_OBJECT_ID,
        vec!["drive_user".to_string()],
        None,
        Some(ISSUER.to_string()),
        vec![AUDIENCE.to_string()],
        now - 7200,
        now - 3600,
    );
    let token = service.encode(&claims).expect("emission du token expire");

    assert!(service.decode(&token).is_err());
}

#[test]
fn token_signe_avec_mauvaise_cle_est_rejete() {
    let emetteur = JwtService::from_secret(AUTRE_SECRET, ISSUER, AUDIENCE);
    let token = emetteur
        .issue(SUB_OBJECT_ID, vec!["drive_user".to_string()], None, Duration::from_secs(3600))
        .expect("emission du token");

    let verificateur = service();

    assert!(verificateur.decode(&token).is_err());
}

#[test]
fn token_altere_est_rejete() {
    let service = service();
    let token = service
        .issue(SUB_OBJECT_ID, vec!["drive_user".to_string()], None, Duration::from_secs(3600))
        .expect("emission du token");
    let mut altere = token.clone();
    altere.push('x');

    assert!(service.decode(&altere).is_err());
}

#[test]
fn drive_user_avec_sub_object_id_est_accepte() {
    let service = service();
    let token = service
        .issue(SUB_OBJECT_ID, vec!["drive_user".to_string()], None, Duration::from_secs(3600))
        .expect("emission du token");

    let claims = service.decode(&token).expect("decodage");

    assert!(is_object_id(&claims.sub));
}

#[test]
fn drive_user_avec_sub_non_object_id_est_rejete_meme_signe() {
    let service = service();
    let token = service
        .issue(SUB_NON_OBJECT_ID, vec!["drive_user".to_string()], None, Duration::from_secs(3600))
        .expect("emission du token");

    let claims = service.decode(&token).expect("signature valide donc decode ok");

    assert!(!is_object_id(&claims.sub));
}

#[test]
fn drive_admin_avec_sub_object_id_et_role_admin_est_accepte() {
    let service = service();
    let token = service
        .issue(SUB_OBJECT_ID, vec!["drive_admin".to_string()], None, Duration::from_secs(3600))
        .expect("emission du token");

    let claims = service.decode(&token).expect("decodage");

    assert!(is_object_id(&claims.sub));
    assert!(claims.has_role("drive_admin"));
}

#[test]
fn drive_admin_avec_sub_non_object_id_est_rejete_meme_signe_et_role_valide() {
    let service = service();
    let token = service
        .issue(SUB_NON_OBJECT_ID, vec!["drive_admin".to_string()], None, Duration::from_secs(3600))
        .expect("emission du token");

    let claims = service.decode(&token).expect("signature valide donc decode ok");

    assert!(claims.has_role("drive_admin"));
    assert!(!is_object_id(&claims.sub));
}
