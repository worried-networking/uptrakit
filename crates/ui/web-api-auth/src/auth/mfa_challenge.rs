//! MFA challenge lifecycle, email OTP helpers, and recovery code management.
//!
//! # Errors
//!
//! All fallible functions in this module return [`super::Result`] wrapping
//! [`super::AuthError`]. Database operations propagate [`sea_orm::DbErr`] via
//! `context_to`. Argon2 hashing errors surface as [`super::AuthError::PasswordHash`].

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use rand::Rng as _;
use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set, sea_query::Expr,
};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::prelude::*;
use uptrakit_shared_db::entity::{mfa_challenge, user_recovery_code};

use super::token::{generate_secure_token, hash_token};
use super::{AuthError, Result};

/// Charset used for recovery codes. Excludes visually ambiguous characters
/// (0, O, 1, I, l, 8, B).
pub const RECOVERY_CODE_CHARSET: &str = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Challenge TTL in seconds (5 minutes).
const MFA_CHALLENGE_TTL_SECS: i64 = 300;

/// Maximum number of failed verification attempts before a challenge is locked.
const MAX_MFA_ATTEMPTS: i32 = 5;

/// Create a new MFA challenge row for `user_id`.
///
/// Returns the plaintext token to send to the client. The token is stored only
/// as a SHA-256 hash in the database.
///
/// # Errors
///
/// Returns [`AuthError::TokenGeneration`] if the CSPRNG fails, or
/// [`AuthError::Database`] on insert failure.
pub async fn create_mfa_challenge(
    db: &impl sea_orm::ConnectionTrait,
    user_id: uuid::Uuid,
) -> Result<String> {
    let plaintext = generate_secure_token()?;
    let token_hash = hash_token(&plaintext);
    let now = OffsetDateTime::now_utc();

    mfa_challenge::ActiveModel {
        id: Set(uuid::Uuid::now_v7()),
        user_id: Set(user_id),
        token_hash: Set(token_hash),
        email_code_hash: Set(None),
        attempt_count: Set(0),
        expires_at: Set(now + time::Duration::seconds(MFA_CHALLENGE_TTL_SECS)),
        consumed_at: Set(None),
        created_at: Set(now),
    }
    .insert(db)
    .await
    .context_to()?;

    Ok(plaintext)
}

/// Load and validate a challenge by its plaintext token.
///
/// Returns the challenge row when the token resolves to an unconsumed,
/// unexpired challenge that has remaining attempts.
///
/// **Must be called inside a `BEGIN IMMEDIATE` transaction** to avoid
/// TOCTOU races between the load and subsequent
/// [`record_failed_attempt`] / [`consume_challenge`] calls.
///
/// # Errors
///
/// Returns [`AuthError::MfaChallengeNotFound`] when the token is unknown or
/// already consumed, [`AuthError::MfaChallengeExpired`] when the TTL has
/// elapsed, and [`AuthError::MfaChallengeExhausted`] when the attempt counter
/// has reached the limit.
pub async fn load_valid_challenge(
    db: &impl sea_orm::ConnectionTrait,
    plaintext_token: &str,
) -> Result<mfa_challenge::Model> {
    let token_hash = hash_token(plaintext_token);
    let now = OffsetDateTime::now_utc();

    let challenge = MfaChallenge::find()
        .filter(mfa_challenge::Column::TokenHash.eq(&token_hash))
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(AuthError::MfaChallengeNotFound))?;

    if challenge.consumed_at.is_some() {
        return Err(report!(AuthError::MfaChallengeNotFound));
    }
    if now >= challenge.expires_at {
        return Err(report!(AuthError::MfaChallengeExpired));
    }
    if challenge.attempt_count >= MAX_MFA_ATTEMPTS {
        return Err(report!(AuthError::MfaChallengeExhausted));
    }

    Ok(challenge)
}

/// Increment `attempt_count` on a challenge after a failed verification.
///
/// When the new count reaches [`MAX_MFA_ATTEMPTS`] the challenge is also
/// immediately consumed (locked). Returns `true` if the challenge is now
/// exhausted.
///
/// # Errors
///
/// Returns [`AuthError::Database`] on update failure.
pub async fn record_failed_attempt(
    db: &impl sea_orm::ConnectionTrait,
    challenge: &mfa_challenge::Model,
) -> Result<bool> {
    let now = OffsetDateTime::now_utc();
    let new_count = challenge.attempt_count + 1;
    let exhausted = new_count >= MAX_MFA_ATTEMPTS;

    let mut active: mfa_challenge::ActiveModel = challenge.clone().into_active_model();
    active.attempt_count = Set(new_count);
    if exhausted {
        active.consumed_at = Set(Some(now));
    }
    active.update(db).await.context_to()?;

    Ok(exhausted)
}

/// Mark a challenge as consumed after a successful verification.
///
/// # Errors
///
/// Returns [`AuthError::Database`] on update failure.
pub async fn consume_challenge(
    db: &impl sea_orm::ConnectionTrait,
    challenge: &mfa_challenge::Model,
) -> Result<()> {
    let mut active: mfa_challenge::ActiveModel = challenge.clone().into_active_model();
    active.consumed_at = Set(Some(OffsetDateTime::now_utc()));
    active.update(db).await.context_to()?;
    Ok(())
}

/// Generate a zero-padded 6-digit email OTP code.
///
/// The returned string is always exactly 6 ASCII decimal digits.
#[must_use]
pub fn generate_email_otp() -> String {
    let n: u32 = rand::rng().random_range(0..1_000_000);
    format!("{n:06}")
}

/// Hash an email OTP code with Argon2id for at-rest storage.
///
/// # Errors
///
/// Returns [`AuthError::PasswordHash`] if Argon2id hashing fails.
pub fn hash_email_otp(code: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(code.as_bytes(), &salt)
        .map_err(|e| report!(AuthError::PasswordHash(e)))?
        .to_string();
    Ok(hash)
}

/// Verify an email OTP against its stored Argon2id hash.
///
/// Returns `true` when the code matches, `false` on a wrong code, and
/// an error only if the stored hash cannot be parsed.
///
/// # Errors
///
/// Returns [`AuthError::PasswordHash`] if the hash string is malformed.
pub fn verify_email_otp(code: &str, hash: &str) -> Result<bool> {
    let parsed = PasswordHash::new(hash).map_err(|e| report!(AuthError::PasswordHash(e)))?;
    Ok(Argon2::default()
        .verify_password(code.as_bytes(), &parsed)
        .is_ok())
}

/// Persist the email OTP hash in the challenge row.
///
/// # Errors
///
/// Returns [`AuthError::Database`] on update failure.
pub async fn store_email_otp_hash(
    db: &impl sea_orm::ConnectionTrait,
    challenge: &mfa_challenge::Model,
    code_hash: String,
) -> Result<()> {
    let mut active: mfa_challenge::ActiveModel = challenge.clone().into_active_model();
    active.email_code_hash = Set(Some(code_hash));
    active.update(db).await.context_to()?;
    Ok(())
}

/// Generate 8 unique 10-character plaintext recovery codes.
///
/// Characters are sampled from [`RECOVERY_CODE_CHARSET`], which excludes
/// visually ambiguous characters.
#[must_use]
pub fn generate_recovery_codes() -> Vec<String> {
    let charset: Vec<char> = RECOVERY_CODE_CHARSET.chars().collect();
    let mut rng = rand::rng();
    (0..8)
        .map(|_| {
            (0..10)
                .map(|_| {
                    let idx = rng.random_range(0..charset.len());
                    #[expect(
                        clippy::indexing_slicing,
                        reason = "idx is in 0..charset.len() by construction"
                    )]
                    charset[idx]
                })
                .collect()
        })
        .collect()
}

/// Hash a single recovery code with Argon2id.
///
/// Uses the same parameters as [`hash_email_otp`].
///
/// # Errors
///
/// Returns [`AuthError::PasswordHash`] if Argon2id hashing fails.
pub fn hash_recovery_code(code: &str) -> Result<String> {
    hash_email_otp(code)
}

/// Find the first unused recovery code matching `plaintext`.
///
/// Returns the row `id` when a match is found, `None` otherwise.
///
/// **This function is CPU-intensive** (each Argon2id verification takes
/// approximately 100 ms). Callers must invoke it from
/// `tokio::task::spawn_blocking` to avoid blocking the async executor.
#[must_use]
pub fn find_matching_recovery_code(
    codes: &[user_recovery_code::Model],
    plaintext: &str,
) -> Option<uuid::Uuid> {
    for code in codes.iter().filter(|c| c.used_at.is_none()) {
        let Ok(hash) = PasswordHash::new(&code.code_hash) else {
            continue;
        };
        if Argon2::default()
            .verify_password(plaintext.as_bytes(), &hash)
            .is_ok()
        {
            return Some(code.id);
        }
    }
    None
}

/// Mark a recovery code row as used.
///
/// **Must be called inside a `BEGIN IMMEDIATE` transaction** to prevent
/// concurrent use of the same code.
///
/// # Errors
///
/// Returns [`AuthError::Database`] on update failure.
pub async fn consume_recovery_code(
    db: &impl sea_orm::ConnectionTrait,
    code_id: uuid::Uuid,
) -> Result<()> {
    UserRecoveryCode::update_many()
        .col_expr(
            user_recovery_code::Column::UsedAt,
            Expr::value(OffsetDateTime::now_utc()),
        )
        .filter(user_recovery_code::Column::Id.eq(code_id))
        .exec(db)
        .await
        .context_to()?;
    Ok(())
}

/// Delete all existing recovery codes for `user_id` and insert new ones.
///
/// **Must be called inside a `BEGIN IMMEDIATE` transaction.**
///
/// `code_hashes` must be pre-computed by the caller (via `spawn_blocking` +
/// `hash_recovery_code`) before opening the transaction, to avoid running
/// Argon2id while holding the write lock.
///
/// # Errors
///
/// Returns [`AuthError::Database`] on any DB failure.
pub async fn replace_recovery_codes(
    db: &impl sea_orm::ConnectionTrait,
    user_id: uuid::Uuid,
    code_hashes: &[String],
) -> Result<()> {
    UserRecoveryCode::delete_many()
        .filter(user_recovery_code::Column::UserId.eq(user_id))
        .exec(db)
        .await
        .context_to()?;

    let now = OffsetDateTime::now_utc();
    for hash in code_hashes {
        user_recovery_code::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            user_id: Set(user_id),
            code_hash: Set(hash.clone()),
            created_at: Set(now),
            used_at: Set(None),
        }
        .insert(db)
        .await
        .context_to()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_recovery_codes_produces_eight_unique_codes() {
        let codes = generate_recovery_codes();
        assert_eq!(codes.len(), 8);
        let unique: std::collections::HashSet<_> = codes.iter().collect();
        assert_eq!(unique.len(), 8, "all codes must be unique");
    }

    #[test]
    fn recovery_code_has_correct_length_and_charset() {
        let codes = generate_recovery_codes();
        for code in &codes {
            assert_eq!(code.len(), 10);
            assert!(
                code.chars().all(|c| RECOVERY_CODE_CHARSET.contains(c)),
                "code {code} contains invalid chars"
            );
        }
    }

    #[test]
    fn generate_email_otp_is_six_digits() {
        for _ in 0..20 {
            let code = generate_email_otp();
            assert_eq!(code.len(), 6);
            assert!(code.chars().all(|c| c.is_ascii_digit()));
        }
    }
}
