// @generated — updated to include roles/permissions tables

diesel::table! {
    accounts (id) {
        id -> BigInt,
        username -> Text,
        password_hash -> Text,
        created_at -> Text,
        last_login -> Nullable<Text>,
        is_admin -> Bool,
    }
}

diesel::table! {
    characters (id) {
        id -> BigInt,
        account_id -> BigInt,
        name -> Text,
        created_at -> Text,
        last_played -> Nullable<Text>,
        data -> Text,
    }
}

diesel::table! {
    roles (id) {
        id -> BigInt,
        name -> Text,
        created_at -> Text,
    }
}

diesel::table! {
    permissions (id) {
        id -> BigInt,
        name -> Text,
    }
}

diesel::table! {
    role_permissions (role_id, permission_id) {
        role_id -> BigInt,
        permission_id -> BigInt,
    }
}

diesel::table! {
    character_roles (character_id, role_id) {
        character_id -> BigInt,
        role_id -> BigInt,
    }
}

// The generic document store. Deliberately joins nothing — it has no foreign
// keys and never appears in a query with another table, so it needs neither a
// `joinable!` nor a place in `allow_tables_to_appear_in_same_query!`.
diesel::table! {
    documents (collection, id) {
        collection -> Text,
        id -> Text,
        data -> Text,
        created_at -> Text,
        updated_at -> Text,
    }
}

diesel::joinable!(characters -> accounts (account_id));
diesel::joinable!(role_permissions -> roles (role_id));
diesel::joinable!(role_permissions -> permissions (permission_id));
diesel::joinable!(character_roles -> characters (character_id));
diesel::joinable!(character_roles -> roles (role_id));

diesel::allow_tables_to_appear_in_same_query!(
    accounts,
    characters,
    roles,
    permissions,
    role_permissions,
    character_roles,
);
