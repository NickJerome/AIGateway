//! Stable identity inputs and public data. Credential material has separate private types.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Auth {
    #[default]
    Public,
    Key,
    User,
    Admin,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub user_id: Option<String>,
    pub key_id: Option<String>,
    pub kind: &'static str,
    /// API keys never convey their owner's administration privileges.
    pub administrator: bool,
}

#[derive(Debug, Serialize)]
pub struct User {
    pub id: String,
    pub login: String,
    pub enabled: bool,
    pub administrator: bool,
    pub created_at: i64,
}

#[derive(Debug, Serialize)]
pub struct Key {
    pub id: String,
    pub user_id: Option<String>,
    pub name: String,
    pub revoked: bool,
    pub expires_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Login {
    pub login: String,
    pub password: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewUser {
    pub login: String,
    pub password: String,
    #[serde(default)]
    pub administrator: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateUser {
    pub enabled: Option<bool>,
    pub administrator: Option<bool>,
    pub password: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewKey {
    pub name: String,
    pub user_id: Option<String>,
    pub expires_at: Option<i64>,
}

#[derive(Serialize)]
pub struct Session {
    pub token: String,
    pub expires_at: i64,
}

#[derive(Serialize)]
pub struct IssuedKey {
    #[serde(flatten)]
    pub metadata: Key,
    pub key: String,
}

// Never Serialize or Debug: these are only used between the service and database.
pub(super) struct Password {
    pub hash: String,
    pub salt: String,
}
pub(super) struct Credentials {
    pub user_id: String,
    pub password: Password,
}

/// An explicit administrative scope or one concrete owner. A missing personal
/// user ID cannot accidentally turn into an unfiltered database query.
pub enum KeyScope {
    All,
    User(String),
}
impl KeyScope {
    pub(super) fn owner(self) -> Option<String> {
        match self {
            Self::All => None,
            Self::User(id) => Some(id),
        }
    }
}
