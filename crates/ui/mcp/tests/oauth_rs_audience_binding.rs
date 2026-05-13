//! Tests that the JWT verifier correctly rejects tokens with mismatched audience.
//!
//! Per spec §6 + §9.1 RFC 8707 audience binding.

use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use uptrakit_web_api_types::oauth::{McpAccessTokenClaims, McpOAuthJwtVerifier};

const SECRET: &[u8] = b"audience-binding-test-secret-32b";
const ISSUER: &str = "https://controller.example.com";
const CORRECT_AUD: &str = "https://controller.example.com/mcp";
const WRONG_AUD: &str = "https://other.example.com/mcp";

/// Minimal signer helper — mints a token with a controlled `aud` claim.
fn mint_token_with_aud(aud: &str) -> String {
    let claims = McpAccessTokenClaims::new(
        ISSUER.into(),                                 // iss
        "00000000-0000-0000-0000-000000000001".into(), // sub
        aud.into(),                                    // aud
        "test-client-id".into(),                       // client_id
        "mcp:read".into(),                             // scope
        "00000000-0000-0000-0000-000000000003".into(), // jti
        1,                                             // iat
        1,                                             // nbf
        9_999_999_999,                                 // exp
        "00000000-0000-0000-0000-000000000002".into(), // tenant_id
    );

    let mut header = Header::new(Algorithm::HS256);
    header.typ = Some("at+jwt".to_string());
    encode(&header, &claims, &EncodingKey::from_secret(SECRET))
        .expect("token encoding should not fail in tests")
}

#[test]
fn verifier_rejects_token_with_wrong_audience() {
    let verifier = McpOAuthJwtVerifier::new(SECRET, ISSUER.into(), vec![CORRECT_AUD.into()]);
    let token = mint_token_with_aud(WRONG_AUD);
    let result = verifier.verify(&token);
    assert!(
        result.is_err(),
        "token with wrong audience must be rejected"
    );
}

#[test]
fn verifier_accepts_token_with_correct_audience() {
    let verifier = McpOAuthJwtVerifier::new(SECRET, ISSUER.into(), vec![CORRECT_AUD.into()]);
    let token = mint_token_with_aud(CORRECT_AUD);
    let result = verifier.verify(&token);
    assert!(
        result.is_ok(),
        "token with correct audience must be accepted"
    );
}
