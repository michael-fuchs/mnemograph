//! Browser sessions of the owner, stored in the database.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::{Knowledge, Result};

/// A logged-in owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// The owner's user ID at the identity provider.
    pub owner_id: String,
    /// How to show the owner, e.g. their email address.
    pub owner_name: String,
}

impl Knowledge {
    /// Starts a session and returns its token, which only the browser keeps.
    pub fn create_session(&self, session: &Session, lifetime: Duration) -> Result<String> {
        let mut bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        let token = URL_SAFE_NO_PAD.encode(bytes);
        let conn = self.conn();
        conn.execute(
            "DELETE FROM session WHERE expires_at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            [],
        )?;
        conn.execute(
            "INSERT INTO session (token_hash, owner_id, owner_name, expires_at) \
             VALUES (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?4))",
            params![
                hash(&token),
                session.owner_id,
                session.owner_name,
                format!("+{} seconds", lifetime.as_secs())
            ],
        )?;
        Ok(token)
    }

    /// Returns the session for a token, if it exists and has not expired.
    pub fn session(&self, token: &str) -> Result<Option<Session>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT owner_id, owner_name FROM session \
                 WHERE token_hash = ?1 AND expires_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                [hash(token)],
                |row| {
                    Ok(Session {
                        owner_id: row.get(0)?,
                        owner_name: row.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// Ends a session.
    pub fn end_session(&self, token: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM session WHERE token_hash = ?1", [hash(token)])?;
        Ok(())
    }
}

fn hash(token: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes()))
}
