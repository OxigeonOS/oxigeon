use diesel::prelude::*;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use argon2::password_hash::SaltString;
use rand_core::OsRng;

use crate::error::{OxigeonError, Result};
use crate::domain::db::connection::AnyPool;
use super::super::db::schema::accounts;

/// Account model — the login identity
#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = accounts)]
pub struct Account {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub created_at: String,
    pub last_login: Option<String>,
    pub is_admin: bool,
}

impl Account {
    /// Convert to a Lua-friendly table representation
    pub fn to_lua_table(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "username": self.username,
            "is_admin": self.is_admin,
            "created_at": self.created_at,
        })
    }
}

#[derive(Insertable)]
#[diesel(table_name = accounts)]
struct NewAccount<'a> {
    username: &'a str,
    password_hash: &'a str,
    created_at: String,
    is_admin: bool,
}

/// Hash a password using argon2
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2.hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| OxigeonError::Internal(format!("Password hash error: {}", e)))
}

/// Verify a password against a hash
pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
    let parsed_hash = PasswordHash::new(hash)
        .map_err(|e| OxigeonError::Internal(format!("Invalid hash: {}", e)))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

/// Diesel-backed account store
pub struct DieselAccountStore {
    pool: AnyPool,
    min_password_length: usize,
}

impl DieselAccountStore {
    pub fn new(pool: AnyPool, min_password_length: usize) -> Self {
        DieselAccountStore { pool, min_password_length }
    }

    pub fn create(&self, username: &str, password: &str) -> Result<Account> {
        if password.len() < self.min_password_length {
            return Err(OxigeonError::Internal(format!(
                "Password must be at least {} characters", self.min_password_length
            )));
        }

        let hash = hash_password(password)?;
        let now = chrono::Utc::now().to_rfc3339();
        {
            let mut conn = self.pool.get_sqlite()?;
            diesel::insert_into(accounts::table)
                .values(NewAccount {
                    username,
                    password_hash: &hash,
                    created_at: now,
                    is_admin: false,
                })
                .execute(&mut conn)?;
        }  // conn is released back to pool here

        let account = self.find_by_name(username)?
            .ok_or_else(|| OxigeonError::Internal("Account not found after insert".into()))?;

        // Auto-promote the first account (id=1) to admin
        if account.id == 1 {
            self.set_admin(account.id, true)?;
            return self.find_by_id(account.id)?
                .ok_or_else(|| OxigeonError::Internal("Account not found after admin promotion".into()));
        }

        Ok(account)
    }

    pub fn authenticate(&self, username: &str, password: &str) -> Result<Account> {
        let account = self.find_by_name(username)?
            .ok_or(OxigeonError::AuthenticationFailed)?;

        if verify_password(password, &account.password_hash)? {
            Ok(account)
        } else {
            Err(OxigeonError::AuthenticationFailed)
        }
    }

    pub fn find_by_id(&self, id: i64) -> Result<Option<Account>> {
        let mut conn = self.pool.get_sqlite()?;
        accounts::table
            .find(id)
            .first::<Account>(&mut conn)
            .optional()
            .map_err(Into::into)
    }

    pub fn find_by_name(&self, name: &str) -> Result<Option<Account>> {
        let mut conn = self.pool.get_sqlite()?;
        accounts::table
            .filter(accounts::username.eq(name))
            .first::<Account>(&mut conn)
            .optional()
            .map_err(Into::into)
    }

    pub fn update_password(&self, id: i64, new_password: &str) -> Result<()> {
        if new_password.len() < self.min_password_length {
            return Err(OxigeonError::Internal(format!(
                "Password must be at least {} characters", self.min_password_length
            )));
        }
        let hash = hash_password(new_password)?;
        let mut conn = self.pool.get_sqlite()?;
        diesel::update(accounts::table.find(id))
            .set(accounts::password_hash.eq(&hash))
            .execute(&mut conn)?;
        Ok(())
    }

    pub fn set_admin(&self, id: i64, is_admin: bool) -> Result<()> {
        let mut conn = self.pool.get_sqlite()?;
        diesel::update(accounts::table.find(id))
            .set(accounts::is_admin.eq(is_admin))
            .execute(&mut conn)?;
        Ok(())
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        let mut conn = self.pool.get_sqlite()?;
        diesel::delete(accounts::table.find(id))
            .execute(&mut conn)?;
        Ok(())
    }
}

impl crate::domain::traits::AccountStore for DieselAccountStore {
    fn create(&self, username: &str, password: &str) -> Result<Account> {
        DieselAccountStore::create(self, username, password)
    }
    fn authenticate(&self, username: &str, password: &str) -> Result<Account> {
        DieselAccountStore::authenticate(self, username, password)
    }
    fn find_by_id(&self, id: i64) -> Result<Option<Account>> {
        DieselAccountStore::find_by_id(self, id)
    }
    fn find_by_name(&self, name: &str) -> Result<Option<Account>> {
        DieselAccountStore::find_by_name(self, name)
    }
    fn update_password(&self, id: i64, new_password: &str) -> Result<()> {
        DieselAccountStore::update_password(self, id, new_password)
    }
    fn set_admin(&self, id: i64, is_admin: bool) -> Result<()> {
        DieselAccountStore::set_admin(self, id, is_admin)
    }
    fn delete(&self, id: i64) -> Result<()> {
        DieselAccountStore::delete(self, id)
    }
}
