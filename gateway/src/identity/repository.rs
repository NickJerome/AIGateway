//! Identity persistence. Race-sensitive checks and their writes share a transaction.
use super::{
    Auth, Identity, Key, User,
    error::{ErrorKind, fail, invalid},
    model::{Credentials, Password},
};
use crate::storage::Database;
use anyhow::Result;
use rusqlite::{OptionalExtension, params};

#[derive(Clone)]
pub(super) struct Repository(pub Database);

fn user_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    Ok(User {
        id: row.get(0)?,
        login: row.get(1)?,
        enabled: row.get(2)?,
        administrator: row.get(3)?,
        created_at: row.get(4)?,
    })
}
fn key_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Key> {
    Ok(Key {
        id: row.get(0)?,
        user_id: row.get(1)?,
        name: row.get(2)?,
        revoked: row.get(3)?,
        expires_at: row.get(4)?,
        created_at: row.get(5)?,
    })
}
impl Repository {
    pub async fn authenticate(&self, hash: String, mode: Auth, now: i64) -> Result<Identity> {
        self.0.run(move |c| match mode {
            Auth::Key => c.query_row(
                "SELECT k.user_id,k.id FROM identity_keys k LEFT JOIN identity_users u ON u.id=k.user_id
                 WHERE k.token_hash=?1 AND k.revoked=0 AND (k.expires_at IS NULL OR k.expires_at>?2)
                 AND (k.user_id IS NULL OR u.enabled=1)", params![hash, now],
                |r| Ok(Identity { user_id:r.get(0)?, key_id:Some(r.get(1)?), kind:"key", administrator:false }),
            ).optional()?.ok_or_else(invalid),
            Auth::User | Auth::Admin => c.query_row(
                "SELECT u.id,u.administrator FROM identity_sessions s JOIN identity_users u ON u.id=s.user_id
                 WHERE s.token_hash=?1 AND s.expires_at>?2 AND u.enabled=1", params![hash, now],
                |r| Ok(Identity { user_id:Some(r.get(0)?), key_id:None, kind:"session", administrator:r.get(1)? }),
            ).optional()?.ok_or_else(invalid),
            Auth::Public => Err(invalid()),
        }).await
    }

    pub async fn credentials(&self, login: String) -> Result<Option<Credentials>> {
        self.0.run(move |c| Ok(c.query_row(
            "SELECT id,password_hash,password_salt FROM identity_users WHERE login=?1 AND enabled=1", [login],
            |r| Ok(Credentials { user_id:r.get(0)?, password:Password { hash:r.get(1)?, salt:r.get(2)? } }),
        ).optional()?)).await
    }

    pub async fn create_session(
        &self,
        credentials: Credentials,
        hash: String,
        expires: i64,
        now: i64,
    ) -> Result<()> {
        self.0.run(move |c| {
            let tx = c.transaction()?;
            // Password reset or disable during hashing must win over this login.
            let valid: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM identity_users WHERE id=?1 AND enabled=1 AND password_hash=?2)",
                params![credentials.user_id, credentials.password.hash], |r| r.get(0),
            )?;
            if !valid { return Err(invalid()); }
            tx.execute("DELETE FROM identity_sessions WHERE expires_at<=?1", [now])?;
            tx.execute(
                "DELETE FROM identity_sessions WHERE user_id=?1 AND token_hash NOT IN
                 (SELECT token_hash FROM identity_sessions WHERE user_id=?1 ORDER BY created_at DESC,rowid DESC LIMIT 7)",
                [&credentials.user_id],
            )?;
            tx.execute("INSERT INTO identity_sessions(token_hash,user_id,expires_at,created_at) VALUES (?1,?2,?3,?4)",
                params![hash, credentials.user_id, expires, now])?;
            tx.commit()?;
            Ok(())
        }).await
    }

    pub async fn logout(&self, hash: String) -> Result<()> {
        self.0
            .run(move |c| {
                c.execute("DELETE FROM identity_sessions WHERE token_hash=?1", [hash])?;
                Ok(())
            })
            .await
    }

    pub async fn create_user(&self, user: User, password: Password) -> Result<User> {
        self.0.run(move |c| {
            let inserted = c.execute(
                "INSERT INTO identity_users(id,login,password_hash,password_salt,administrator,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(login) DO NOTHING",
                params![user.id,user.login,password.hash,password.salt,user.administrator,user.created_at],
            )?;
            if inserted == 0 { return Err(fail(ErrorKind::Conflict, "login name already exists")); }
            Ok(user)
        }).await
    }

    pub async fn update_user(
        &self,
        id: String,
        enabled: Option<bool>,
        administrator: Option<bool>,
        password: Option<Password>,
        has_bootstrap: bool,
    ) -> Result<User> {
        self.0.run(move |c| {
            let tx = c.transaction()?;
            let mut user = tx.query_row(
                "SELECT id,login,enabled,administrator,created_at FROM identity_users WHERE id=?1", [&id], user_row,
            ).optional()?.ok_or_else(|| fail(ErrorKind::NotFound, "user not found"))?;
            let next_enabled = enabled.unwrap_or(user.enabled);
            let next_admin = administrator.unwrap_or(user.administrator);
            if !has_bootstrap && user.enabled && user.administrator && (!next_enabled || !next_admin) {
                let count: i64 = tx.query_row("SELECT COUNT(*) FROM identity_users WHERE enabled=1 AND administrator=1", [], |r| r.get(0))?;
                if count <= 1 { return Err(fail(ErrorKind::Conflict, "cannot disable or demote the last administrator")); }
            }
            tx.execute("UPDATE identity_users SET enabled=?2,administrator=?3 WHERE id=?1", params![id,next_enabled,next_admin])?;
            if let Some(password) = &password {
                tx.execute("UPDATE identity_users SET password_hash=?2,password_salt=?3 WHERE id=?1", params![id,password.hash,password.salt])?;
            }
            if !next_enabled || administrator.is_some() || password.is_some() {
                tx.execute("DELETE FROM identity_sessions WHERE user_id=?1", [&id])?;
            }
            tx.commit()?;
            user.enabled = next_enabled;
            user.administrator = next_admin;
            Ok(user)
        }).await
    }

    pub async fn user(&self, id: String) -> Result<User> {
        self.0.run(move |c| c.query_row(
            "SELECT id,login,enabled,administrator,created_at FROM identity_users WHERE id=?1", [id], user_row,
        ).optional()?.ok_or_else(|| fail(ErrorKind::NotFound, "user not found"))).await
    }

    pub async fn users(&self) -> Result<Vec<User>> {
        self.0.run(|c| {
            let mut query = c.prepare("SELECT id,login,enabled,administrator,created_at FROM identity_users ORDER BY created_at,id LIMIT 1000")?;
            Ok(query.query_map([], user_row)?.collect::<rusqlite::Result<Vec<_>>>()?)
        }).await
    }

    pub async fn create_key(&self, key: Key, hash: String) -> Result<Key> {
        self.0.run(move |c| {
            let tx = c.transaction()?;
            if let Some(user) = &key.user_id {
                let valid: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM identity_users WHERE id=?1 AND enabled=1)", [user], |r| r.get(0))?;
                if !valid { return Err(fail(ErrorKind::NotFound, "active user not found")); }
            }
            tx.execute(
                "INSERT INTO identity_keys(id,user_id,name,token_hash,expires_at,created_at) VALUES (?1,?2,?3,?4,?5,?6)",
                params![key.id,key.user_id,key.name,hash,key.expires_at,key.created_at],
            )?;
            tx.commit()?;
            Ok(key)
        }).await
    }

    pub async fn keys(&self, owner: Option<String>) -> Result<Vec<Key>> {
        self.0
            .run(move |c| {
                let mut query = c.prepare(
                    "SELECT id,user_id,name,revoked,expires_at,created_at FROM identity_keys
                 WHERE (?1 IS NULL OR user_id=?1) ORDER BY created_at,id LIMIT 1000",
                )?;
                Ok(query
                    .query_map([owner], key_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .await
    }

    pub async fn revoke_key(&self, id: String, owner: Option<String>) -> Result<()> {
        self.0
            .run(move |c| {
                if c.execute(
                    "UPDATE identity_keys SET revoked=1 WHERE id=?1 AND (?2 IS NULL OR user_id=?2)",
                    params![id, owner],
                )? == 0
                {
                    return Err(fail(ErrorKind::NotFound, "key not found"));
                }
                Ok(())
            })
            .await
    }
}
