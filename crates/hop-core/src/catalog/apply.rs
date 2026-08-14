use std::collections::{BTreeMap, BTreeSet};

use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Sqlite, Transaction};

use super::{
    manifest::{
        validate_references, AssetType, CatalogError, CatalogErrorCode, CatalogResult,
        CredentialType, ResolvedAccess, ResolvedAsset, ResolvedCredential, ResolvedManifest,
    },
    Catalog, Manifest,
};
use crate::{encrypt_envelope, new_id, MasterKey};

#[derive(Debug, Clone, Copy, Default)]
pub struct ApplyOptions {
    pub base_revision: Option<i64>,
    pub prune: bool,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyAction {
    Created,
    Updated,
    Deleted,
    Orphaned,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApplyChange {
    pub resource_type: String,
    pub name: String,
    pub action: ApplyAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApplySummary {
    pub source_id: String,
    pub base_revision: i64,
    pub new_revision: i64,
    pub generation: i64,
    pub dry_run: bool,
    pub created: usize,
    pub updated: usize,
    pub deleted: usize,
    pub orphaned: usize,
    pub unchanged: usize,
    pub changes: Vec<ApplyChange>,
}

impl Catalog {
    pub async fn validate_manifest(&self, manifest: &Manifest) -> CatalogResult<()> {
        let resolved = manifest.resolve_material()?;
        let credentials = load_credentials(self).await?;
        let assets = load_assets(self).await?;
        let credential_names = credentials.keys().cloned().collect();
        let asset_names = assets.keys().cloned().collect();
        validate_references(&resolved, true, &credential_names, &asset_names)
    }

    pub async fn diff(
        &self,
        manifest: &Manifest,
        source_id: &str,
        master_key: &MasterKey,
        prune: bool,
    ) -> CatalogResult<ApplySummary> {
        self.apply(
            manifest,
            source_id,
            master_key,
            ApplyOptions {
                prune,
                dry_run: true,
                ..ApplyOptions::default()
            },
        )
        .await
    }

    pub async fn apply(
        &self,
        manifest: &Manifest,
        source_id: &str,
        master_key: &MasterKey,
        options: ApplyOptions,
    ) -> CatalogResult<ApplySummary> {
        validate_source_id(source_id)?;
        let base_revision = self.revision().await.map_err(database_error)?;
        if let Some(expected) = options.base_revision {
            if expected != base_revision {
                return Err(revision_conflict(expected, base_revision));
            }
        }
        let current_generation = source_generation(self, source_id).await?;
        let resolved = manifest.resolve_material()?;
        let plan = build_plan(self, resolved, source_id, master_key, options.prune).await?;
        let changed = plan.changed();
        let generation = current_generation + i64::from(changed);
        let new_revision = base_revision + i64::from(changed);
        let summary = plan.summary(
            source_id,
            base_revision,
            new_revision,
            generation,
            options.dry_run,
        );
        if options.dry_run || !changed {
            return Ok(summary);
        }

        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let locked = sqlx::query(
            "UPDATE catalog_meta SET revision = revision WHERE singleton_id = 1 AND revision = ?1",
        )
        .bind(base_revision)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if locked.rows_affected() != 1 {
            let actual = sqlx::query_scalar::<_, i64>(
                "SELECT revision FROM catalog_meta WHERE singleton_id = 1",
            )
            .fetch_one(&mut *transaction)
            .await
            .map_err(database_error)?;
            transaction.rollback().await.map_err(database_error)?;
            return Err(revision_conflict(base_revision, actual));
        }

        execute_plan(
            &mut transaction,
            &plan,
            source_id,
            generation,
            new_revision,
            master_key,
            &summary,
        )
        .await?;
        transaction.commit().await.map_err(database_error)?;
        Ok(summary)
    }
}

struct Plan {
    credential_upserts: Vec<CredentialUpsert>,
    asset_upserts: Vec<AssetUpsert>,
    access_upserts: Vec<AccessUpsert>,
    orphans: Vec<ResourceRef>,
    deletes: Vec<ResourceRef>,
    changes: Vec<ApplyChange>,
}

impl Plan {
    fn changed(&self) -> bool {
        self.changes
            .iter()
            .any(|change| change.action != ApplyAction::Unchanged)
    }

    fn summary(
        &self,
        source_id: &str,
        base_revision: i64,
        new_revision: i64,
        generation: i64,
        dry_run: bool,
    ) -> ApplySummary {
        let count = |action| {
            self.changes
                .iter()
                .filter(|change| change.action == action)
                .count()
        };
        ApplySummary {
            source_id: source_id.to_string(),
            base_revision,
            new_revision,
            generation,
            dry_run,
            created: count(ApplyAction::Created),
            updated: count(ApplyAction::Updated),
            deleted: count(ApplyAction::Deleted),
            orphaned: count(ApplyAction::Orphaned),
            unchanged: count(ApplyAction::Unchanged),
            changes: self.changes.clone(),
        }
    }
}

struct CredentialUpsert {
    id: String,
    name: String,
    credential_type: CredentialType,
    username: String,
    password: Option<String>,
    private_key: Option<String>,
    passphrase: Option<String>,
    hash: String,
}

struct AssetUpsert {
    id: String,
    name: String,
    asset_type: AssetType,
    host: String,
    port: u16,
    display_name: Option<String>,
    description: Option<String>,
    credential_id: Option<String>,
    preset: Option<String>,
    hash: String,
}

struct AccessUpsert {
    id: String,
    name: String,
    public_key: String,
    fingerprint: String,
    enabled: bool,
    asset_ids: Option<Vec<String>>,
    hash: String,
}

#[derive(Clone)]
struct ResourceRef {
    resource_type: &'static str,
    id: String,
    name: String,
}

#[derive(Clone, sqlx::FromRow)]
struct ExistingResource {
    id: String,
    name: String,
    management_mode: Option<String>,
    source_id: Option<String>,
    last_applied_hash: Option<String>,
    orphaned_at: Option<String>,
}

#[derive(Clone, sqlx::FromRow)]
struct ExistingAsset {
    id: String,
    name: String,
    credential_id: Option<String>,
    management_mode: Option<String>,
    source_id: Option<String>,
    last_applied_hash: Option<String>,
    orphaned_at: Option<String>,
}

impl ExistingAsset {
    fn resource(&self) -> ExistingResource {
        ExistingResource {
            id: self.id.clone(),
            name: self.name.clone(),
            management_mode: self.management_mode.clone(),
            source_id: self.source_id.clone(),
            last_applied_hash: self.last_applied_hash.clone(),
            orphaned_at: self.orphaned_at.clone(),
        }
    }
}

async fn build_plan(
    catalog: &Catalog,
    manifest: ResolvedManifest,
    source_id: &str,
    master_key: &MasterKey,
    prune: bool,
) -> CatalogResult<Plan> {
    let existing_credentials = load_credentials(catalog).await?;
    let existing_assets = load_assets(catalog).await?;
    let existing_access = load_access(catalog).await?;

    let mut final_credential_names: BTreeSet<String> =
        existing_credentials.keys().cloned().collect();
    let mut final_asset_names: BTreeSet<String> = existing_assets.keys().cloned().collect();
    apply_final_names(
        &mut final_credential_names,
        &manifest.credentials,
        |resource| matches!(resource, ResolvedCredential::Present { .. }),
    );
    apply_final_names(&mut final_asset_names, &manifest.assets, |resource| {
        matches!(resource, ResolvedAsset::Present { .. })
    });
    if prune {
        remove_pruned_names(
            &mut final_credential_names,
            &existing_credentials,
            manifest.credentials.keys().cloned().collect(),
            source_id,
        );
        let asset_resources: BTreeMap<String, ExistingResource> = existing_assets
            .iter()
            .map(|(name, asset)| (name.clone(), asset.resource()))
            .collect();
        remove_pruned_names(
            &mut final_asset_names,
            &asset_resources,
            manifest.assets.keys().cloned().collect(),
            source_id,
        );
    }
    validate_references(&manifest, true, &final_credential_names, &final_asset_names)?;

    let mut plan = Plan {
        credential_upserts: Vec::new(),
        asset_upserts: Vec::new(),
        access_upserts: Vec::new(),
        orphans: Vec::new(),
        deletes: Vec::new(),
        changes: Vec::new(),
    };

    let mut credential_ids: BTreeMap<String, String> = existing_credentials
        .iter()
        .map(|(name, resource)| (name.clone(), resource.id.clone()))
        .collect();
    for (name, resource) in &manifest.credentials {
        let path = format!("credentials.{name}");
        match resource {
            ResolvedCredential::Absent => {
                plan_absent(
                    &mut plan,
                    "credential",
                    name,
                    existing_credentials.get(name),
                    source_id,
                    &path,
                )?;
                credential_ids.remove(name);
            }
            ResolvedCredential::Present {
                credential_type,
                username,
                password,
                private_key,
                passphrase,
            } => {
                let hash = credential_hash(
                    name,
                    *credential_type,
                    username,
                    password.as_deref(),
                    private_key.as_deref(),
                    passphrase.as_deref(),
                    master_key,
                );
                let (id, action) =
                    classify_present(existing_credentials.get(name), source_id, &hash, &path)?;
                credential_ids.insert(name.clone(), id.clone());
                push_change(&mut plan, "credential", name, action);
                if action != ApplyAction::Unchanged {
                    plan.credential_upserts.push(CredentialUpsert {
                        id,
                        name: name.clone(),
                        credential_type: *credential_type,
                        username: username.clone(),
                        password: password.clone(),
                        private_key: private_key.clone(),
                        passphrase: passphrase.clone(),
                        hash,
                    });
                }
            }
        }
    }
    plan_missing(
        &mut plan,
        "credential",
        &existing_credentials,
        manifest.credentials.keys().cloned().collect(),
        source_id,
        prune,
    );
    for deletion in plan
        .deletes
        .iter()
        .filter(|resource| resource.resource_type == "credential")
    {
        credential_ids.retain(|_, id| id != &deletion.id);
    }

    let mut asset_ids: BTreeMap<String, String> = existing_assets
        .iter()
        .map(|(name, resource)| (name.clone(), resource.id.clone()))
        .collect();
    for (name, resource) in &manifest.assets {
        let path = format!("assets.{name}");
        match resource {
            ResolvedAsset::Absent => {
                let existing = existing_assets.get(name).map(ExistingAsset::resource);
                plan_absent(
                    &mut plan,
                    "asset",
                    name,
                    existing.as_ref(),
                    source_id,
                    &path,
                )?;
                asset_ids.remove(name);
            }
            ResolvedAsset::Present {
                asset_type,
                host,
                port,
                display_name,
                description,
                credential,
                preset,
            } => {
                let credential_id = credential
                    .as_ref()
                    .and_then(|credential| credential_ids.get(credential))
                    .cloned();
                if credential.is_some() && credential_id.is_none() {
                    return Err(CatalogError::new(
                        CatalogErrorCode::UnknownReference,
                        Some(format!("{path}.credential")),
                        "credential is not present in the final catalog",
                    ));
                }
                let hash = asset_hash(
                    *asset_type,
                    host,
                    *port,
                    display_name.as_deref(),
                    description.as_deref(),
                    credential.as_deref(),
                    preset.as_deref(),
                );
                let existing = existing_assets.get(name).map(ExistingAsset::resource);
                let (id, action) = classify_present(existing.as_ref(), source_id, &hash, &path)?;
                asset_ids.insert(name.clone(), id.clone());
                push_change(&mut plan, "asset", name, action);
                if action != ApplyAction::Unchanged {
                    plan.asset_upserts.push(AssetUpsert {
                        id,
                        name: name.clone(),
                        asset_type: *asset_type,
                        host: host.clone(),
                        port: *port,
                        display_name: display_name.clone(),
                        description: description.clone(),
                        credential_id,
                        preset: preset.clone(),
                        hash,
                    });
                }
            }
        }
    }
    let asset_resources: BTreeMap<String, ExistingResource> = existing_assets
        .iter()
        .map(|(name, asset)| (name.clone(), asset.resource()))
        .collect();
    plan_missing(
        &mut plan,
        "asset",
        &asset_resources,
        manifest.assets.keys().cloned().collect(),
        source_id,
        prune,
    );
    for deletion in plan
        .deletes
        .iter()
        .filter(|resource| resource.resource_type == "asset")
    {
        asset_ids.retain(|_, id| id != &deletion.id);
    }

    for (name, resource) in &manifest.access {
        let path = format!("access.{name}");
        match resource {
            ResolvedAccess::Absent => plan_absent(
                &mut plan,
                "access_key",
                name,
                existing_access.get(name),
                source_id,
                &path,
            )?,
            ResolvedAccess::Present {
                public_key,
                fingerprint,
                enabled,
                assets,
            } => {
                let mut normalized_assets = assets.clone();
                if let Some(assets) = &mut normalized_assets {
                    assets.sort();
                }
                let asset_assignment_ids = normalized_assets
                    .as_ref()
                    .map(|assets| {
                        assets
                            .iter()
                            .map(|asset| {
                                asset_ids.get(asset).cloned().ok_or_else(|| {
                                    CatalogError::new(
                                        CatalogErrorCode::UnknownReference,
                                        Some(format!("{path}.assets")),
                                        "asset is not present in the final catalog",
                                    )
                                })
                            })
                            .collect::<CatalogResult<Vec<_>>>()
                    })
                    .transpose()?;
                let hash = access_hash(public_key, *enabled, normalized_assets.as_deref());
                let (id, action) =
                    classify_present(existing_access.get(name), source_id, &hash, &path)?;
                push_change(&mut plan, "access_key", name, action);
                if action != ApplyAction::Unchanged {
                    plan.access_upserts.push(AccessUpsert {
                        id,
                        name: name.clone(),
                        public_key: public_key.clone(),
                        fingerprint: fingerprint.clone(),
                        enabled: *enabled,
                        asset_ids: asset_assignment_ids,
                        hash,
                    });
                }
            }
        }
    }
    plan_missing(
        &mut plan,
        "access_key",
        &existing_access,
        manifest.access.keys().cloned().collect(),
        source_id,
        prune,
    );

    validate_credential_deletes(&plan, &existing_assets)?;
    plan.changes.sort_by(|left, right| {
        (&left.resource_type, &left.name).cmp(&(&right.resource_type, &right.name))
    });
    Ok(plan)
}

fn apply_final_names<T>(
    names: &mut BTreeSet<String>,
    resources: &BTreeMap<String, T>,
    is_present: impl Fn(&T) -> bool,
) {
    for (name, resource) in resources {
        if is_present(resource) {
            names.insert(name.clone());
        } else {
            names.remove(name);
        }
    }
}

fn remove_pruned_names(
    names: &mut BTreeSet<String>,
    existing: &BTreeMap<String, ExistingResource>,
    mentioned: BTreeSet<String>,
    source_id: &str,
) {
    for (name, resource) in existing {
        if resource.source_id.as_deref() == Some(source_id) && !mentioned.contains(name) {
            names.remove(name);
        }
    }
}

fn plan_absent(
    plan: &mut Plan,
    resource_type: &'static str,
    name: &str,
    existing: Option<&ExistingResource>,
    source_id: &str,
    path: &str,
) -> CatalogResult<()> {
    if let Some(existing) = existing {
        ensure_owned(existing, source_id, path)?;
        plan.deletes.push(ResourceRef {
            resource_type,
            id: existing.id.clone(),
            name: name.to_string(),
        });
        push_change(plan, resource_type, name, ApplyAction::Deleted);
    } else {
        push_change(plan, resource_type, name, ApplyAction::Unchanged);
    }
    Ok(())
}

fn plan_missing(
    plan: &mut Plan,
    resource_type: &'static str,
    existing: &BTreeMap<String, ExistingResource>,
    mentioned: BTreeSet<String>,
    source_id: &str,
    prune: bool,
) {
    for (name, resource) in existing {
        if resource.source_id.as_deref() != Some(source_id) || mentioned.contains(name) {
            continue;
        }
        if prune {
            plan.deletes.push(ResourceRef {
                resource_type,
                id: resource.id.clone(),
                name: name.clone(),
            });
            push_change(plan, resource_type, name, ApplyAction::Deleted);
        } else if resource.orphaned_at.is_none() {
            plan.orphans.push(ResourceRef {
                resource_type,
                id: resource.id.clone(),
                name: name.clone(),
            });
            push_change(plan, resource_type, name, ApplyAction::Orphaned);
        } else {
            push_change(plan, resource_type, name, ApplyAction::Unchanged);
        }
    }
}

fn classify_present(
    existing: Option<&ExistingResource>,
    source_id: &str,
    hash: &str,
    path: &str,
) -> CatalogResult<(String, ApplyAction)> {
    let Some(existing) = existing else {
        return Ok((new_id(), ApplyAction::Created));
    };
    ensure_owned(existing, source_id, path)?;
    let action =
        if existing.last_applied_hash.as_deref() == Some(hash) && existing.orphaned_at.is_none() {
            ApplyAction::Unchanged
        } else {
            ApplyAction::Updated
        };
    Ok((existing.id.clone(), action))
}

fn ensure_owned(existing: &ExistingResource, source_id: &str, path: &str) -> CatalogResult<()> {
    if existing.management_mode.as_deref() == Some("declarative")
        && existing.source_id.as_deref() == Some(source_id)
    {
        return Ok(());
    }
    let owner = match (
        existing.management_mode.as_deref(),
        existing.source_id.as_deref(),
    ) {
        (Some("declarative"), Some(owner)) => format!("declarative source {owner}"),
        _ => "local management".to_string(),
    };
    Err(CatalogError::new(
        CatalogErrorCode::OwnershipConflict,
        Some(path.to_string()),
        format!("resource is already owned by {owner}"),
    ))
}

fn validate_credential_deletes(
    plan: &Plan,
    existing_assets: &BTreeMap<String, ExistingAsset>,
) -> CatalogResult<()> {
    let deleted_asset_ids: BTreeSet<&str> = plan
        .deletes
        .iter()
        .filter(|resource| resource.resource_type == "asset")
        .map(|resource| resource.id.as_str())
        .collect();
    let updated_assets: BTreeMap<&str, Option<&str>> = plan
        .asset_upserts
        .iter()
        .map(|asset| (asset.id.as_str(), asset.credential_id.as_deref()))
        .collect();
    for credential in plan
        .deletes
        .iter()
        .filter(|resource| resource.resource_type == "credential")
    {
        for asset in existing_assets.values() {
            if deleted_asset_ids.contains(asset.id.as_str()) {
                continue;
            }
            let final_credential = updated_assets
                .get(asset.id.as_str())
                .copied()
                .unwrap_or(asset.credential_id.as_deref());
            if final_credential == Some(credential.id.as_str()) {
                return Err(CatalogError::new(
                    CatalogErrorCode::ResourceInUse,
                    Some(format!("credentials.{}", credential.name)),
                    format!("credential is still referenced by asset {}", asset.name),
                ));
            }
        }
    }
    Ok(())
}

fn push_change(plan: &mut Plan, resource_type: &str, name: &str, action: ApplyAction) {
    plan.changes.push(ApplyChange {
        resource_type: resource_type.to_string(),
        name: name.to_string(),
        action,
    });
}

async fn execute_plan(
    transaction: &mut Transaction<'_, Sqlite>,
    plan: &Plan,
    source_id: &str,
    generation: i64,
    new_revision: i64,
    master_key: &MasterKey,
    summary: &ApplySummary,
) -> CatalogResult<()> {
    for credential in &plan.credential_upserts {
        let password_enc = encrypt_optional(
            master_key,
            &credential.id,
            "password",
            credential.password.as_deref(),
        )?;
        let private_key_enc = encrypt_optional(
            master_key,
            &credential.id,
            "private_key",
            credential.private_key.as_deref(),
        )?;
        let passphrase_enc = encrypt_optional(
            master_key,
            &credential.id,
            "passphrase",
            credential.passphrase.as_deref(),
        )?;
        sqlx::query(
            r#"
            INSERT INTO credentials
                (id, name, username, auth_type, password_enc, private_key_enc, passphrase_enc, secret_hmac)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                username = excluded.username,
                auth_type = excluded.auth_type,
                password_enc = excluded.password_enc,
                private_key_enc = excluded.private_key_enc,
                passphrase_enc = excluded.passphrase_enc,
                secret_hmac = excluded.secret_hmac,
                updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(&credential.id)
        .bind(&credential.name)
        .bind(&credential.username)
        .bind(credential.credential_type.as_str())
        .bind(password_enc)
        .bind(private_key_enc)
        .bind(passphrase_enc)
        .bind(&credential.hash)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
        upsert_ownership(
            transaction,
            "credential",
            &credential.id,
            source_id,
            &credential.name,
            generation,
            &credential.hash,
        )
        .await?;
    }

    for asset in &plan.asset_upserts {
        sqlx::query(
            r#"
            INSERT INTO assets
                (id, name, asset_type, host, port, display_name, description, preset, credential_id)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                asset_type = excluded.asset_type,
                host = excluded.host,
                port = excluded.port,
                display_name = excluded.display_name,
                description = excluded.description,
                preset = excluded.preset,
                credential_id = excluded.credential_id,
                updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(&asset.id)
        .bind(&asset.name)
        .bind(asset.asset_type.as_str())
        .bind(&asset.host)
        .bind(i64::from(asset.port))
        .bind(&asset.display_name)
        .bind(&asset.description)
        .bind(&asset.preset)
        .bind(&asset.credential_id)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
        upsert_ownership(
            transaction,
            "asset",
            &asset.id,
            source_id,
            &asset.name,
            generation,
            &asset.hash,
        )
        .await?;
    }

    for access in &plan.access_upserts {
        let access_mode = if access.asset_ids.is_some() {
            "restricted"
        } else {
            "all"
        };
        sqlx::query(
            r#"
            INSERT INTO access_keys
                (id, name, public_key, fingerprint, enabled, access_mode)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                public_key = excluded.public_key,
                fingerprint = excluded.fingerprint,
                enabled = excluded.enabled,
                access_mode = excluded.access_mode,
                updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(&access.id)
        .bind(&access.name)
        .bind(&access.public_key)
        .bind(&access.fingerprint)
        .bind(access.enabled)
        .bind(access_mode)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
        sqlx::query("DELETE FROM access_key_assets WHERE key_id = ?1")
            .bind(&access.id)
            .execute(&mut **transaction)
            .await
            .map_err(database_error)?;
        if let Some(asset_ids) = &access.asset_ids {
            for asset_id in asset_ids {
                sqlx::query("INSERT INTO access_key_assets (key_id, asset_id) VALUES (?1, ?2)")
                    .bind(&access.id)
                    .bind(asset_id)
                    .execute(&mut **transaction)
                    .await
                    .map_err(database_error)?;
            }
        }
        upsert_ownership(
            transaction,
            "access_key",
            &access.id,
            source_id,
            &access.name,
            generation,
            &access.hash,
        )
        .await?;
    }

    for orphan in &plan.orphans {
        sqlx::query(
            "UPDATE resource_ownership SET orphaned_at = CURRENT_TIMESTAMP WHERE resource_type = ?1 AND resource_id = ?2",
        )
        .bind(orphan.resource_type)
        .bind(&orphan.id)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
    }

    for resource_type in ["access_key", "asset", "credential"] {
        for deletion in plan
            .deletes
            .iter()
            .filter(|resource| resource.resource_type == resource_type)
        {
            delete_resource(transaction, deletion).await?;
        }
    }

    sqlx::query("UPDATE catalog_meta SET revision = ?1 WHERE singleton_id = 1")
        .bind(new_revision)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
    sqlx::query(
        r#"
        INSERT INTO config_sources
            (source_id, generation, last_success_at, last_success_revision, last_error_at, last_error_code, last_error_message)
        VALUES (?1, ?2, CURRENT_TIMESTAMP, ?3, NULL, NULL, NULL)
        ON CONFLICT(source_id) DO UPDATE SET
            generation = excluded.generation,
            last_success_at = excluded.last_success_at,
            last_success_revision = excluded.last_success_revision,
            last_error_at = NULL,
            last_error_code = NULL,
            last_error_message = NULL
        "#,
    )
    .bind(source_id)
    .bind(generation)
    .bind(new_revision)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    let details = serde_json::json!({
        "source_id": source_id,
        "base_revision": summary.base_revision,
        "new_revision": summary.new_revision,
        "created": summary.created,
        "updated": summary.updated,
        "deleted": summary.deleted,
        "orphaned": summary.orphaned,
    });
    sqlx::query(
        r#"
        INSERT INTO audit_events
            (id, actor_label, action, target_type, target_id, target_label, result, details_json)
        VALUES (?1, 'catalog-apply', 'config.apply', 'config_source', ?2, ?2, 'success', ?3)
        "#,
    )
    .bind(new_id())
    .bind(source_id)
    .bind(details.to_string())
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    Ok(())
}

async fn upsert_ownership(
    transaction: &mut Transaction<'_, Sqlite>,
    resource_type: &str,
    resource_id: &str,
    source_id: &str,
    name: &str,
    generation: i64,
    hash: &str,
) -> CatalogResult<()> {
    let source_key = format!("{resource_type}/{name}");
    sqlx::query(
        r#"
        INSERT INTO resource_ownership
            (resource_type, resource_id, management_mode, source_id, source_key, source_generation, last_applied_hash, last_applied_at, orphaned_at)
        VALUES (?1, ?2, 'declarative', ?3, ?4, ?5, ?6, CURRENT_TIMESTAMP, NULL)
        ON CONFLICT(resource_type, resource_id) DO UPDATE SET
            management_mode = 'declarative',
            source_id = excluded.source_id,
            source_key = excluded.source_key,
            source_generation = excluded.source_generation,
            last_applied_hash = excluded.last_applied_hash,
            last_applied_at = excluded.last_applied_at,
            orphaned_at = NULL
        "#,
    )
    .bind(resource_type)
    .bind(resource_id)
    .bind(source_id)
    .bind(source_key)
    .bind(generation)
    .bind(hash)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    Ok(())
}

async fn delete_resource(
    transaction: &mut Transaction<'_, Sqlite>,
    resource: &ResourceRef,
) -> CatalogResult<()> {
    sqlx::query("DELETE FROM resource_ownership WHERE resource_type = ?1 AND resource_id = ?2")
        .bind(resource.resource_type)
        .bind(&resource.id)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
    let query = match resource.resource_type {
        "credential" => "DELETE FROM credentials WHERE id = ?1",
        "asset" => "DELETE FROM assets WHERE id = ?1",
        "access_key" => "DELETE FROM access_keys WHERE id = ?1",
        _ => {
            return Err(CatalogError::new(
                CatalogErrorCode::ApplyFailed,
                None,
                "unsupported resource type in apply plan",
            ))
        }
    };
    sqlx::query(query)
        .bind(&resource.id)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
    Ok(())
}

fn encrypt_optional(
    master_key: &MasterKey,
    id: &str,
    field: &str,
    value: Option<&str>,
) -> CatalogResult<Option<String>> {
    value
        .map(|value| {
            encrypt_envelope(master_key, &format!("{id}:{field}"), value.as_bytes()).map_err(|_| {
                CatalogError::new(
                    CatalogErrorCode::ApplyFailed,
                    None,
                    "failed to encrypt credential material",
                )
            })
        })
        .transpose()
}

fn credential_hash(
    name: &str,
    credential_type: CredentialType,
    username: &str,
    password: Option<&str>,
    private_key: Option<&str>,
    passphrase: Option<&str>,
    master_key: &MasterKey,
) -> String {
    hash_value(serde_json::json!({
        "type": credential_type.as_str(),
        "username": username,
        "password": password.map(|value| secret_hmac(master_key, name, "password", value)),
        "private_key": private_key.map(|value| secret_hmac(master_key, name, "private_key", value)),
        "passphrase": passphrase.map(|value| secret_hmac(master_key, name, "passphrase", value)),
    }))
}

fn asset_hash(
    asset_type: AssetType,
    host: &str,
    port: u16,
    display_name: Option<&str>,
    description: Option<&str>,
    credential: Option<&str>,
    preset: Option<&str>,
) -> String {
    hash_value(serde_json::json!({
        "type": asset_type.as_str(),
        "host": host,
        "port": port,
        "display_name": display_name,
        "description": description,
        "credential": credential,
        "preset": preset,
    }))
}

fn access_hash(public_key: &str, enabled: bool, assets: Option<&[String]>) -> String {
    hash_value(serde_json::json!({
        "public_key": public_key,
        "enabled": enabled,
        "assets": assets,
    }))
}

fn secret_hmac(master_key: &MasterKey, name: &str, field: &str, value: &str) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(master_key.expose())
        .expect("master key has a valid HMAC length");
    mac.update(name.as_bytes());
    mac.update(&[0]);
    mac.update(field.as_bytes());
    mac.update(&[0]);
    mac.update(value.as_bytes());
    STANDARD_NO_PAD.encode(mac.finalize().into_bytes())
}

fn hash_value(value: serde_json::Value) -> String {
    let bytes = serde_json::to_vec(&value).expect("JSON values are serializable");
    STANDARD_NO_PAD.encode(Sha256::digest(bytes))
}

async fn load_credentials(catalog: &Catalog) -> CatalogResult<BTreeMap<String, ExistingResource>> {
    let resources = sqlx::query_as::<_, ExistingResource>(
        r#"
        SELECT c.id, c.name, o.management_mode, o.source_id, o.last_applied_hash, o.orphaned_at
        FROM credentials c
        LEFT JOIN resource_ownership o ON o.resource_type = 'credential' AND o.resource_id = c.id
        "#,
    )
    .fetch_all(&catalog.pool)
    .await
    .map_err(database_error)?;
    Ok(resources
        .into_iter()
        .map(|resource| (resource.name.clone(), resource))
        .collect())
}

async fn load_assets(catalog: &Catalog) -> CatalogResult<BTreeMap<String, ExistingAsset>> {
    let resources = sqlx::query_as::<_, ExistingAsset>(
        r#"
        SELECT a.id, a.name, a.credential_id,
               o.management_mode, o.source_id, o.last_applied_hash, o.orphaned_at
        FROM assets a
        LEFT JOIN resource_ownership o ON o.resource_type = 'asset' AND o.resource_id = a.id
        "#,
    )
    .fetch_all(&catalog.pool)
    .await
    .map_err(database_error)?;
    Ok(resources
        .into_iter()
        .map(|resource| (resource.name.clone(), resource))
        .collect())
}

async fn load_access(catalog: &Catalog) -> CatalogResult<BTreeMap<String, ExistingResource>> {
    let resources = sqlx::query_as::<_, ExistingResource>(
        r#"
        SELECT k.id, k.name, o.management_mode, o.source_id, o.last_applied_hash, o.orphaned_at
        FROM access_keys k
        LEFT JOIN resource_ownership o ON o.resource_type = 'access_key' AND o.resource_id = k.id
        "#,
    )
    .fetch_all(&catalog.pool)
    .await
    .map_err(database_error)?;
    Ok(resources
        .into_iter()
        .map(|resource| (resource.name.clone(), resource))
        .collect())
}

async fn source_generation(catalog: &Catalog, source_id: &str) -> CatalogResult<i64> {
    sqlx::query_scalar::<_, i64>("SELECT generation FROM config_sources WHERE source_id = ?1")
        .bind(source_id)
        .fetch_optional(&catalog.pool)
        .await
        .map_err(database_error)
        .map(|generation| generation.unwrap_or(0))
}

fn validate_source_id(source_id: &str) -> CatalogResult<()> {
    let valid = !source_id.is_empty()
        && source_id.len() <= 128
        && source_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        });
    if valid {
        Ok(())
    } else {
        Err(CatalogError::new(
            CatalogErrorCode::InvalidResourceName,
            Some("source_id".to_string()),
            "source_id must contain only ASCII letters, digits, '.', '-' or '_'",
        ))
    }
}

fn revision_conflict(expected: i64, actual: i64) -> CatalogError {
    CatalogError::new(
        CatalogErrorCode::RevisionConflict,
        Some("catalog_revision".to_string()),
        format!("base revision {expected} does not match current revision {actual}"),
    )
}

fn database_error(_: impl std::fmt::Display) -> CatalogError {
    CatalogError::new(
        CatalogErrorCode::ApplyFailed,
        None,
        "catalog database operation failed",
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    const PUBLIC_KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ test";

    struct Fixture {
        _directory: tempfile::TempDir,
        password_path: std::path::PathBuf,
        public_key_path: std::path::PathBuf,
    }

    impl Fixture {
        fn new(password: &str) -> Self {
            let directory = tempfile::tempdir().unwrap();
            let password_path = directory.path().join("password");
            let public_key_path = directory.path().join("id.pub");
            fs::write(&password_path, password).unwrap();
            fs::write(&public_key_path, PUBLIC_KEY).unwrap();
            Self {
                _directory: directory,
                password_path,
                public_key_path,
            }
        }

        fn manifest(&self, assets: &str) -> Manifest {
            Manifest::from_yaml(&format!(
                r#"api_version: hop/v1alpha1
credentials:
  root:
    type: password
    username: root
    password:
      file: {}
assets:
{assets}
access:
  laptop:
    public_key:
      file: {}
    assets: [server]
"#,
                self.password_path.display(),
                self.public_key_path.display(),
            ))
            .unwrap()
        }
    }

    fn server_assets() -> &'static str {
        "  server:\n    type: ssh\n    host: 192.0.2.10\n    port: 22\n    credential: root"
    }

    #[tokio::test]
    async fn apply_is_atomic_encrypted_and_idempotent() {
        let catalog = Catalog::in_memory().await.unwrap();
        let master_key = MasterKey::generate();
        let fixture = Fixture::new("do-not-leak-this-password");
        let manifest = fixture.manifest(server_assets());

        let dry_run = catalog
            .diff(&manifest, "home", &master_key, false)
            .await
            .unwrap();
        assert_eq!(dry_run.created, 3);
        assert_eq!(dry_run.base_revision, 0);
        assert_eq!(dry_run.new_revision, 1);
        assert_eq!(catalog.revision().await.unwrap(), 0);

        let applied = catalog
            .apply(&manifest, "home", &master_key, ApplyOptions::default())
            .await
            .unwrap();
        assert_eq!(applied.created, 3);
        assert_eq!(catalog.revision().await.unwrap(), 1);
        let encrypted: String =
            sqlx::query_scalar("SELECT password_enc FROM credentials WHERE name = 'root'")
                .fetch_one(catalog.pool())
                .await
                .unwrap();
        assert!(!encrypted.contains("do-not-leak-this-password"));
        let serialized = serde_json::to_string(&applied).unwrap();
        assert!(!serialized.contains("do-not-leak-this-password"));

        let repeated = catalog
            .apply(&manifest, "home", &master_key, ApplyOptions::default())
            .await
            .unwrap();
        assert_eq!(repeated.unchanged, 3);
        assert_eq!(repeated.new_revision, 1);
        assert_eq!(repeated.generation, 1);
        assert_eq!(catalog.revision().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn resource_in_use_and_revision_conflicts_make_no_partial_changes() {
        let catalog = Catalog::in_memory().await.unwrap();
        let master_key = MasterKey::generate();
        let fixture = Fixture::new("password");
        let manifest = fixture.manifest(server_assets());
        catalog
            .apply(&manifest, "home", &master_key, ApplyOptions::default())
            .await
            .unwrap();

        let invalid = Manifest::from_yaml(
            "api_version: hop/v1alpha1\ncredentials:\n  root:\n    state: absent\n",
        )
        .unwrap();
        let error = catalog
            .apply(&invalid, "home", &master_key, ApplyOptions::default())
            .await
            .unwrap_err();
        assert_eq!(error.code, CatalogErrorCode::ResourceInUse);
        assert_eq!(catalog.revision().await.unwrap(), 1);
        let credential_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM credentials")
            .fetch_one(catalog.pool())
            .await
            .unwrap();
        assert_eq!(credential_count, 1);

        let error = catalog
            .apply(
                &manifest,
                "home",
                &master_key,
                ApplyOptions {
                    base_revision: Some(0),
                    ..ApplyOptions::default()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, CatalogErrorCode::RevisionConflict);
        assert_eq!(catalog.revision().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn missing_resources_orphan_by_default_and_prune_only_the_source() {
        let catalog = Catalog::in_memory().await.unwrap();
        let master_key = MasterKey::generate();
        let fixture = Fixture::new("password");
        let two_assets = format!(
            "{}\n  spare:\n    type: tcp\n    host: 192.0.2.20\n    port: 80",
            server_assets()
        );
        catalog
            .apply(
                &fixture.manifest(&two_assets),
                "home",
                &master_key,
                ApplyOptions::default(),
            )
            .await
            .unwrap();

        let orphaned = catalog
            .apply(
                &fixture.manifest(server_assets()),
                "home",
                &master_key,
                ApplyOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(orphaned.orphaned, 1);
        let orphaned_at: Option<String> = sqlx::query_scalar(
            "SELECT orphaned_at FROM resource_ownership WHERE source_key = 'asset/spare'",
        )
        .fetch_one(catalog.pool())
        .await
        .unwrap();
        assert!(orphaned_at.is_some());

        let pruned = catalog
            .apply(
                &fixture.manifest(server_assets()),
                "home",
                &master_key,
                ApplyOptions {
                    prune: true,
                    ..ApplyOptions::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(pruned.deleted, 1);
        let spare_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM assets WHERE name = 'spare'")
                .fetch_one(catalog.pool())
                .await
                .unwrap();
        assert_eq!(spare_count, 0);
    }

    #[tokio::test]
    async fn manifest_cannot_silently_take_over_a_local_resource() {
        let catalog = Catalog::in_memory().await.unwrap();
        let master_key = MasterKey::generate();
        let fixture = Fixture::new("password");
        sqlx::query(
            "INSERT INTO assets (id, name, asset_type, host, port) VALUES ('local-id', 'server', 'tcp', '127.0.0.1', 80)",
        )
        .execute(catalog.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO resource_ownership (resource_type, resource_id, management_mode) VALUES ('asset', 'local-id', 'local')",
        )
        .execute(catalog.pool())
        .await
        .unwrap();

        let error = catalog
            .apply(
                &fixture.manifest(server_assets()),
                "home",
                &master_key,
                ApplyOptions::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, CatalogErrorCode::OwnershipConflict);
        assert_eq!(catalog.revision().await.unwrap(), 0);
    }
}
