use diesel::prelude::*;
use crate::error::Result;
use crate::domain::db::connection::AnyPool;
use super::super::db::schema::{roles, permissions, role_permissions, character_roles};

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = roles)]
pub struct Role {
    pub id: i64,
    pub name: String,
    pub created_at: String,
}

impl Role {
    pub fn to_lua_table(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "name": self.name,
            "created_at": self.created_at,
        })
    }
}

#[derive(Insertable)]
#[diesel(table_name = roles)]
struct NewRole<'a> {
    name: &'a str,
    created_at: String,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = permissions)]
pub struct Permission {
    pub id: i64,
    pub name: String,
}

#[derive(Insertable)]
#[diesel(table_name = permissions)]
struct NewPermission<'a> {
    name: &'a str,
}

#[derive(Insertable)]
#[diesel(table_name = role_permissions)]
struct NewRolePermission {
    role_id: i64,
    permission_id: i64,
}

#[derive(Insertable)]
#[diesel(table_name = character_roles)]
struct NewCharacterRole {
    character_id: i64,
    role_id: i64,
}

pub struct DieselRoleStore {
    pool: AnyPool,
}

impl DieselRoleStore {
    pub fn new(pool: AnyPool) -> Self {
        DieselRoleStore { pool }
    }

    /// Create a new role. Returns error if name already exists.
    pub fn create_role(&self, name: &str) -> Result<Role> {
        let mut conn = self.pool.get_sqlite()?;
        let now = chrono::Utc::now().to_rfc3339();
        let new_role = NewRole { name, created_at: now };
        diesel::insert_into(roles::table)
            .values(&new_role)
            .execute(&mut conn)?;
        roles::table
            .filter(roles::name.eq(name))
            .first::<Role>(&mut conn)
            .map_err(Into::into)
    }

    pub fn find_role_by_name(&self, name: &str) -> Result<Option<Role>> {
        let mut conn = self.pool.get_sqlite()?;
        roles::table
            .filter(roles::name.eq(name))
            .first::<Role>(&mut conn)
            .optional()
            .map_err(Into::into)
    }

    pub fn find_role_by_id(&self, id: i64) -> Result<Option<Role>> {
        let mut conn = self.pool.get_sqlite()?;
        roles::table
            .filter(roles::id.eq(id))
            .first::<Role>(&mut conn)
            .optional()
            .map_err(Into::into)
    }

    pub fn list_roles(&self) -> Result<Vec<Role>> {
        let mut conn = self.pool.get_sqlite()?;
        roles::table
            .order(roles::name.asc())
            .load::<Role>(&mut conn)
            .map_err(Into::into)
    }

    pub fn delete_role(&self, id: i64) -> Result<()> {
        let mut conn = self.pool.get_sqlite()?;
        diesel::delete(roles::table.filter(roles::id.eq(id)))
            .execute(&mut conn)?;
        Ok(())
    }

    /// Ensure a permission name exists in the permissions table, returning its id.
    fn ensure_permission(&self, conn: &mut diesel::SqliteConnection, perm_name: &str) -> Result<i64> {
        // Try to find existing
        let existing: Option<Permission> = permissions::table
            .filter(permissions::name.eq(perm_name))
            .first::<Permission>(conn)
            .optional()?;
        if let Some(p) = existing {
            return Ok(p.id);
        }
        // Insert new
        let new_perm = NewPermission { name: perm_name };
        diesel::insert_into(permissions::table)
            .values(&new_perm)
            .execute(conn)?;
        let p: Permission = permissions::table
            .filter(permissions::name.eq(perm_name))
            .first::<Permission>(conn)?;
        Ok(p.id)
    }

    pub fn grant_permission(&self, role_id: i64, perm_name: &str) -> Result<()> {
        let mut conn = self.pool.get_sqlite()?;
        let perm_id = self.ensure_permission(&mut conn, perm_name)?;
        // Ignore duplicate (already granted) — SQLite UNIQUE constraint will produce an error
        // which we suppress here
        let _ = diesel::insert_into(role_permissions::table)
            .values(NewRolePermission { role_id, permission_id: perm_id })
            .execute(&mut conn);
        Ok(())
    }

    pub fn revoke_permission(&self, role_id: i64, perm_name: &str) -> Result<()> {
        let mut conn = self.pool.get_sqlite()?;
        // Find permission id
        let perm: Option<Permission> = permissions::table
            .filter(permissions::name.eq(perm_name))
            .first::<Permission>(&mut conn)
            .optional()?;
        if let Some(p) = perm {
            diesel::delete(
                role_permissions::table
                    .filter(role_permissions::role_id.eq(role_id))
                    .filter(role_permissions::permission_id.eq(p.id))
            )
            .execute(&mut conn)?;
        }
        Ok(())
    }

    pub fn get_permissions_for_role(&self, role_id: i64) -> Result<Vec<String>> {
        let mut conn = self.pool.get_sqlite()?;
        let perm_ids: Vec<i64> = role_permissions::table
            .filter(role_permissions::role_id.eq(role_id))
            .select(role_permissions::permission_id)
            .load::<i64>(&mut conn)?;
        if perm_ids.is_empty() {
            return Ok(vec![]);
        }
        permissions::table
            .filter(permissions::id.eq_any(&perm_ids))
            .select(permissions::name)
            .load::<String>(&mut conn)
            .map_err(Into::into)
    }

    pub fn assign_role(&self, character_id: i64, role_id: i64) -> Result<()> {
        let mut conn = self.pool.get_sqlite()?;
        // Ignore duplicate
        let _ = diesel::insert_into(character_roles::table)
            .values(NewCharacterRole { character_id, role_id })
            .execute(&mut conn);
        Ok(())
    }

    pub fn revoke_role(&self, character_id: i64, role_id: i64) -> Result<()> {
        let mut conn = self.pool.get_sqlite()?;
        diesel::delete(
            character_roles::table
                .filter(character_roles::character_id.eq(character_id))
                .filter(character_roles::role_id.eq(role_id))
        )
        .execute(&mut conn)?;
        Ok(())
    }

    pub fn get_roles_for_character(&self, character_id: i64) -> Result<Vec<Role>> {
        let mut conn = self.pool.get_sqlite()?;
        let role_ids: Vec<i64> = character_roles::table
            .filter(character_roles::character_id.eq(character_id))
            .select(character_roles::role_id)
            .load::<i64>(&mut conn)?;
        if role_ids.is_empty() {
            return Ok(vec![]);
        }
        roles::table
            .filter(roles::id.eq_any(&role_ids))
            .order(roles::name.asc())
            .load::<Role>(&mut conn)
            .map_err(Into::into)
    }

    /// Get the union of all permissions across all roles assigned to a character.
    pub fn get_permissions_for_character(&self, character_id: i64) -> Result<Vec<String>> {
        let mut conn = self.pool.get_sqlite()?;
        // Get all role_ids for this character
        let role_ids: Vec<i64> = character_roles::table
            .filter(character_roles::character_id.eq(character_id))
            .select(character_roles::role_id)
            .load::<i64>(&mut conn)?;
        if role_ids.is_empty() {
            return Ok(vec![]);
        }
        // Get all permission_ids for those roles
        let perm_ids: Vec<i64> = role_permissions::table
            .filter(role_permissions::role_id.eq_any(&role_ids))
            .select(role_permissions::permission_id)
            .load::<i64>(&mut conn)?;
        if perm_ids.is_empty() {
            return Ok(vec![]);
        }
        // Deduplicate and fetch permission names
        let mut unique_ids = perm_ids;
        unique_ids.sort();
        unique_ids.dedup();
        permissions::table
            .filter(permissions::id.eq_any(&unique_ids))
            .select(permissions::name)
            .load::<String>(&mut conn)
            .map_err(Into::into)
    }
}

impl crate::domain::traits::RoleStore for DieselRoleStore {
    fn create_role(&self, name: &str) -> Result<Role> {
        DieselRoleStore::create_role(self, name)
    }
    fn find_role_by_name(&self, name: &str) -> Result<Option<Role>> {
        DieselRoleStore::find_role_by_name(self, name)
    }
    fn find_role_by_id(&self, id: i64) -> Result<Option<Role>> {
        DieselRoleStore::find_role_by_id(self, id)
    }
    fn list_roles(&self) -> Result<Vec<Role>> {
        DieselRoleStore::list_roles(self)
    }
    fn delete_role(&self, id: i64) -> Result<()> {
        DieselRoleStore::delete_role(self, id)
    }
    fn grant_permission(&self, role_id: i64, perm_name: &str) -> Result<()> {
        DieselRoleStore::grant_permission(self, role_id, perm_name)
    }
    fn revoke_permission(&self, role_id: i64, perm_name: &str) -> Result<()> {
        DieselRoleStore::revoke_permission(self, role_id, perm_name)
    }
    fn get_permissions_for_role(&self, role_id: i64) -> Result<Vec<String>> {
        DieselRoleStore::get_permissions_for_role(self, role_id)
    }
    fn assign_role(&self, character_id: i64, role_id: i64) -> Result<()> {
        DieselRoleStore::assign_role(self, character_id, role_id)
    }
    fn revoke_role(&self, character_id: i64, role_id: i64) -> Result<()> {
        DieselRoleStore::revoke_role(self, character_id, role_id)
    }
    fn get_roles_for_character(&self, character_id: i64) -> Result<Vec<Role>> {
        DieselRoleStore::get_roles_for_character(self, character_id)
    }
    fn get_permissions_for_character(&self, character_id: i64) -> Result<Vec<String>> {
        DieselRoleStore::get_permissions_for_character(self, character_id)
    }
}
