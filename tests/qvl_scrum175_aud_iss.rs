use ch_api_drive::services::jwt::{unix_now, Claims, JwtService};
use jsonwebtoken::{Algorithm, EncodingKey, Header};

const SECRET: &str = "secret-de-test-suffisamment-long-pour-hs256";
const ISSUER_ATTENDU: &str = "ch-api-authenticator";
const AUDIENCE_DRIVE: &str = "ch-api-drive";
const AUDIENCE_AUTRE_SERVICE: &str = "ch-api-other";
const SUB: &str = "0123456789abcdef01234567";

fn drive() -> JwtService {
    JwtService::from_secret(SECRET, ISSUER_ATTENDU, AUDIENCE_DRIVE)
}

fn signer_claims(claims: &Claims) -> String {
    jsonwebtoken::encode(
        &Header::new(Algorithm::HS256),
        claims,
        &EncodingKey::from_secret(SECRET.as_bytes()),
    )
    .expect("signature du token forge")
}

fn signer_json(claims: serde_json::Value) -> String {
    jsonwebtoken::encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(SECRET.as_bytes()),
    )
    .expect("signature du token forge")
}

fn claims_avec(iss: Option<&str>, aud: Vec<String>) -> Claims {
    let now = unix_now();
    Claims::new(
        SUB,
        vec!["drive_user".to_string()],
        None,
        iss.map(|value| value.to_string()),
        aud,
        now,
        now + 3600,
    )
}

#[test]
fn token_avec_aud_drive_et_iss_authenticator_est_accepte() {
    let drive = drive();
    let token = signer_claims(&claims_avec(
        Some(ISSUER_ATTENDU),
        vec![AUDIENCE_DRIVE.to_string()],
    ));

    let claims = drive.decode(&token).expect("token destine a drive accepte");

    assert!(claims.aud.iter().any(|value| value == AUDIENCE_DRIVE));
    assert_eq!(claims.iss.as_deref(), Some(ISSUER_ATTENDU));
}

#[test]
fn token_destine_a_un_autre_service_est_rejete() {
    let drive = drive();
    let token = signer_claims(&claims_avec(
        Some(ISSUER_ATTENDU),
        vec![AUDIENCE_AUTRE_SERVICE.to_string()],
    ));

    assert!(drive.decode(&token).is_err());
}

#[test]
fn token_avec_aud_vide_est_rejete() {
    let drive = drive();
    let token = signer_claims(&claims_avec(Some(ISSUER_ATTENDU), Vec::new()));

    assert!(drive.decode(&token).is_err());
}

#[test]
fn token_sans_claim_aud_est_rejete() {
    let drive = drive();
    let now = unix_now();
    let token = signer_json(serde_json::json!({
        "sub": SUB,
        "roles": ["drive_user"],
        "iss": ISSUER_ATTENDU,
        "iat": now,
        "exp": now + 3600,
    }));

    assert!(drive.decode(&token).is_err());
}

#[test]
fn token_avec_iss_incorrect_est_rejete() {
    let drive = drive();
    let token = signer_claims(&claims_avec(
        Some("ch-api-imposteur"),
        vec![AUDIENCE_DRIVE.to_string()],
    ));

    assert!(drive.decode(&token).is_err());
}

#[test]
fn token_sans_claim_iss_est_rejete() {
    let drive = drive();
    let now = unix_now();
    let token = signer_json(serde_json::json!({
        "sub": SUB,
        "roles": ["drive_user"],
        "aud": [AUDIENCE_DRIVE],
        "iat": now,
        "exp": now + 3600,
    }));

    assert!(drive.decode(&token).is_err());
}

#[test]
fn token_avec_aud_drive_parmi_plusieurs_audiences_est_accepte() {
    let drive = drive();
    let token = signer_claims(&claims_avec(
        Some(ISSUER_ATTENDU),
        vec![
            AUDIENCE_AUTRE_SERVICE.to_string(),
            AUDIENCE_DRIVE.to_string(),
        ],
    ));

    let claims = drive.decode(&token).expect("aud multiple contenant drive accepte");

    assert!(claims.aud.iter().any(|value| value == AUDIENCE_DRIVE));
}
