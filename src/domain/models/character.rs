use diesel::prelude::*;
use crate::error::{OxigeonError, Result};
use crate::domain::db::connection::AnyPool;
use super::super::db::schema::characters;

/// Character model — the in-game presence
#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = characters)]
pub struct Character {
    pub id: i64,
    pub account_id: i64,
    pub name: String,
    pub created_at: String,
    pub last_played: Option<String>,
    pub data: String,
}

impl Character {
    pub fn to_lua_table(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "account_id": self.account_id,
            "name": self.name,
            "created_at": self.created_at,
        })
    }
}

#[derive(Insertable)]
#[diesel(table_name = characters)]
struct NewCharacter<'a> {
    account_id: i64,
    name: &'a str,
    created_at: String,
}

/// Diesel-backed character store
pub struct DieselCharacterStore {
    pool: AnyPool,
    max_per_account: usize,
}

impl DieselCharacterStore {
    pub fn new(pool: AnyPool, max_per_account: usize) -> Self {
        DieselCharacterStore { pool, max_per_account }
    }

    pub fn create(&self, account_id: i64, name: &str) -> Result<Character> {
        // Check limit
        let existing = self.find_by_account(account_id)?;
        if existing.len() >= self.max_per_account {
            return Err(OxigeonError::CharacterLimitReached);
        }

        let now = chrono::Utc::now().to_rfc3339();
        {
            let mut conn = self.pool.get_sqlite()?;
            diesel::insert_into(characters::table)
                .values(NewCharacter {
                    account_id,
                    name,
                    created_at: now,
                })
                .execute(&mut conn)?;
        }  // conn released here

        self.find_by_name(name)?
            .ok_or_else(|| OxigeonError::Internal("Character not found after insert".into()))
    }

    pub fn find_by_id(&self, id: i64) -> Result<Option<Character>> {
        let mut conn = self.pool.get_sqlite()?;
        characters::table
            .find(id)
            .first::<Character>(&mut conn)
            .optional()
            .map_err(Into::into)
    }

    pub fn find_by_account(&self, account_id: i64) -> Result<Vec<Character>> {
        let mut conn = self.pool.get_sqlite()?;
        characters::table
            .filter(characters::account_id.eq(account_id))
            .load::<Character>(&mut conn)
            .map_err(Into::into)
    }

    pub fn find_by_name(&self, name: &str) -> Result<Option<Character>> {
        let mut conn = self.pool.get_sqlite()?;
        characters::table
            .filter(characters::name.eq(name))
            .first::<Character>(&mut conn)
            .optional()
            .map_err(Into::into)
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        let mut conn = self.pool.get_sqlite()?;
        diesel::delete(characters::table.find(id))
            .execute(&mut conn)?;
        Ok(())
    }

    pub fn save_data(&self, id: i64, data: &str) -> Result<()> {
        let mut conn = self.pool.get_sqlite()?;
        diesel::update(characters::table.find(id))
            .set(characters::data.eq(data))
            .execute(&mut conn)?;
        Ok(())
    }

    pub fn load_data(&self, id: i64) -> Result<Option<String>> {
        let mut conn = self.pool.get_sqlite()?;
        let result: Option<String> = characters::table
            .find(id)
            .select(characters::data)
            .first::<String>(&mut conn)
            .optional()?;
        Ok(result)
    }
}

impl crate::domain::traits::CharacterStore for DieselCharacterStore {
    fn create(&self, account_id: i64, name: &str) -> Result<Character> {
        DieselCharacterStore::create(self, account_id, name)
    }
    fn find_by_id(&self, id: i64) -> Result<Option<Character>> {
        DieselCharacterStore::find_by_id(self, id)
    }
    fn find_by_account(&self, account_id: i64) -> Result<Vec<Character>> {
        DieselCharacterStore::find_by_account(self, account_id)
    }
    fn find_by_name(&self, name: &str) -> Result<Option<Character>> {
        DieselCharacterStore::find_by_name(self, name)
    }
    fn delete(&self, id: i64) -> Result<()> {
        DieselCharacterStore::delete(self, id)
    }
    fn save_data(&self, id: i64, data: &str) -> Result<()> {
        DieselCharacterStore::save_data(self, id, data)
    }
    fn load_data(&self, id: i64) -> Result<Option<String>> {
        DieselCharacterStore::load_data(self, id)
    }
}
