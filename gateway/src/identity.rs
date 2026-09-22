//! Core identity service: credentials, sessions and key ownership.
//! HTTP routing and JSON decoding live in `http`; SQL lives in `repository`.
use crate::storage::Database;
use anyhow::{Result, anyhow};
use argon2::Argon2;
use std::sync::Arc;
use tokio::sync::Semaphore;
mod credentials;
pub(crate) mod error;
mod model;
mod repository;
pub use credentials::secret_equal;
use credentials::{digest, now, random_hex};
pub use error::{Error, ErrorKind};
use error::{fail, invalid};
use model::Password;
pub use model::{
    Auth, Identity, IssuedKey, Key, KeyScope, Login, NewKey, NewUser, Session, UpdateUser, User,
};
use repository::Repository;

#[derive(Clone)]
pub struct Service {
    repository: Repository,
    passwords: Arc<Semaphore>,
}
impl Service {
    pub fn new(db: Database) -> Self {
        Self {
            repository: Repository(db),
            passwords: Arc::new(Semaphore::new(2)),
        }
    }

    // The permit belongs to the blocking job even if its HTTP caller is cancelled.
    async fn password_hash(&self, password: String, salt: String) -> Result<String> {
        let permit = self
            .passwords
            .clone()
            .try_acquire_owned()
            .map_err(|_| fail(ErrorKind::Busy, "login capacity exhausted; retry later"))?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut out = [0u8; 32];
            Argon2::default()
                .hash_password_into(password.as_bytes(), salt.as_bytes(), &mut out)
                .map_err(|e| anyhow!("password hashing failed: {e}"))?;
            Ok(hex::encode(out))
        })
        .await?
    }

    async fn new_password(&self, password: String) -> Result<Password> {
        if !(12..=1024).contains(&password.len()) {
            return Err(fail(
                ErrorKind::InvalidInput,
                "password must contain 12–1024 bytes",
            ));
        }
        let salt = random_hex(16)?;
        Ok(Password {
            hash: self.password_hash(password, salt.clone()).await?,
            salt,
        })
    }

    pub async fn authenticate(&self, token: &str, mode: Auth) -> Result<Identity> {
        let identity = self
            .repository
            .authenticate(digest(token), mode, now())
            .await?;
        if mode == Auth::Admin && !identity.administrator {
            return Err(fail(
                ErrorKind::Forbidden,
                "administrator authorization required",
            ));
        }
        Ok(identity)
    }

    pub async fn login(&self, input: Login) -> Result<Session> {
        if input.login.len() > 128 || input.password.len() > 1024 {
            return Err(invalid());
        }
        let credentials = self
            .repository
            .credentials(input.login.to_ascii_lowercase())
            .await?;
        // Unknown users perform the same expensive operation.
        let salt = credentials
            .as_ref()
            .map(|c| c.password.salt.clone())
            .unwrap_or_else(|| "00000000000000000000000000000000".into());
        let actual = self.password_hash(input.password, salt).await?;
        let credentials = credentials.ok_or_else(invalid)?;
        if !secret_equal(&actual, &credentials.password.hash) {
            return Err(invalid());
        }
        let token = format!("gw_session_{}", random_hex(32)?);
        let now = now();
        let expires_at = now + 86400;
        self.repository
            .create_session(credentials, digest(&token), expires_at, now)
            .await?;
        Ok(Session { token, expires_at })
    }

    pub async fn logout(&self, token: &str) -> Result<()> {
        self.repository.logout(digest(token)).await
    }

    pub async fn create_user(&self, input: NewUser) -> Result<User> {
        let login = input.login.to_ascii_lowercase();
        if login.is_empty()
            || login.len() > 128
            || !login
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-@+".contains(&b))
        {
            return Err(fail(ErrorKind::InvalidInput, "invalid login name"));
        }
        let password = self.new_password(input.password).await?;
        let user = User {
            id: format!("usr_{}", random_hex(16)?),
            login,
            enabled: true,
            administrator: input.administrator,
            created_at: now(),
        };
        self.repository.create_user(user, password).await
    }

    pub async fn update_user(
        &self,
        id: String,
        input: UpdateUser,
        has_bootstrap: bool,
    ) -> Result<User> {
        if input.enabled.is_none() && input.administrator.is_none() && input.password.is_none() {
            return Err(fail(ErrorKind::InvalidInput, "no user changes supplied"));
        }
        let password = match input.password {
            Some(password) => Some(self.new_password(password).await?),
            None => None,
        };
        self.repository
            .update_user(
                id,
                input.enabled,
                input.administrator,
                password,
                has_bootstrap,
            )
            .await
    }

    pub async fn user(&self, id: String) -> Result<User> {
        self.repository.user(id).await
    }
    pub async fn users(&self) -> Result<Vec<User>> {
        self.repository.users().await
    }

    pub async fn create_key(&self, input: NewKey, scope: KeyScope) -> Result<IssuedKey> {
        if input.name.trim().is_empty()
            || input.name.len() > 128
            || input
                .expires_at
                .is_some_and(|t| t <= now() || t > 253402300799)
        {
            return Err(fail(
                ErrorKind::InvalidInput,
                "invalid key name or expiry (Unix seconds)",
            ));
        }
        let owner = match scope {
            KeyScope::All => input.user_id,
            KeyScope::User(owner) => {
                if input.user_id.as_ref().is_some_and(|id| id != &owner) {
                    return Err(fail(
                        ErrorKind::Forbidden,
                        "cannot create another user's key",
                    ));
                }
                Some(owner)
            }
        };
        let key = Key {
            id: format!("key_{}", random_hex(16)?),
            user_id: owner,
            name: input.name,
            revoked: false,
            expires_at: input.expires_at,
            created_at: now(),
        };
        let token = format!("gw_key_{}", random_hex(32)?);
        let metadata = self.repository.create_key(key, digest(&token)).await?;
        Ok(IssuedKey {
            metadata,
            key: token,
        })
    }

    pub async fn keys(&self, scope: KeyScope) -> Result<Vec<Key>> {
        self.repository.keys(scope.owner()).await
    }
    pub async fn revoke_key(&self, id: String, scope: KeyScope) -> Result<()> {
        self.repository.revoke_key(id, scope.owner()).await
    }
}
