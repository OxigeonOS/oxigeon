use crate::domain::models::account::Account;
use crate::domain::models::character::Character;
use crate::domain::models::role::Role;
use crate::error::Result;

/// Trait abstraction over any account store backend.
pub trait AccountStore: Send + Sync {
    fn create(&self, username: &str, password: &str) -> Result<Account>;
    fn authenticate(&self, username: &str, password: &str) -> Result<Account>;
    fn find_by_id(&self, id: i64) -> Result<Option<Account>>;
    fn find_by_name(&self, name: &str) -> Result<Option<Account>>;
    fn update_password(&self, id: i64, new_password: &str) -> Result<()>;
    fn set_admin(&self, id: i64, is_admin: bool) -> Result<()>;
    fn delete(&self, id: i64) -> Result<()>;
}

/// Trait abstraction over any character store backend.
pub trait CharacterStore: Send + Sync {
    fn create(&self, account_id: i64, name: &str) -> Result<Character>;
    fn find_by_id(&self, id: i64) -> Result<Option<Character>>;
    fn find_by_account(&self, account_id: i64) -> Result<Vec<Character>>;
    fn find_by_name(&self, name: &str) -> Result<Option<Character>>;
    fn delete(&self, id: i64) -> Result<()>;
    fn save_data(&self, id: i64, data: &str) -> Result<()>;
    fn load_data(&self, id: i64) -> Result<Option<String>>;
}

/// Trait abstraction over any role/permission store backend.
pub trait RoleStore: Send + Sync {
    fn create_role(&self, name: &str) -> Result<Role>;
    fn find_role_by_name(&self, name: &str) -> Result<Option<Role>>;
    fn find_role_by_id(&self, id: i64) -> Result<Option<Role>>;
    fn list_roles(&self) -> Result<Vec<Role>>;
    fn delete_role(&self, id: i64) -> Result<()>;
    fn grant_permission(&self, role_id: i64, perm_name: &str) -> Result<()>;
    fn revoke_permission(&self, role_id: i64, perm_name: &str) -> Result<()>;
    fn get_permissions_for_role(&self, role_id: i64) -> Result<Vec<String>>;
    fn assign_role(&self, character_id: i64, role_id: i64) -> Result<()>;
    fn revoke_role(&self, character_id: i64, role_id: i64) -> Result<()>;
    fn get_roles_for_character(&self, character_id: i64) -> Result<Vec<Role>>;
    fn get_permissions_for_character(&self, character_id: i64) -> Result<Vec<String>>;
}
