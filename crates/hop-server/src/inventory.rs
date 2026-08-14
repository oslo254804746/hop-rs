use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};

use hop_core::{
    config::InventorySourceConfig, ApplyOptions, ApplySummary, Catalog, CatalogError, MasterKey,
};
use tracing::{info, warn};

use crate::manifest_io::{self, ScopeSnapshot};

pub async fn apply_source(
    catalog: &Catalog,
    master_key: &MasterKey,
    source: &InventorySourceConfig,
    actor_label: &str,
) -> Result<ApplySummary, CatalogError> {
    let result = async {
        let manifest = manifest_io::load_manifest_scope(&[PathBuf::from(&source.path)])?;
        let base_revision = catalog.revision().await.map_err(|_| {
            CatalogError::new(
                hop_core::CatalogErrorCode::ApplyFailed,
                None,
                "catalog database operation failed",
            )
        })?;
        catalog
            .apply(
                &manifest,
                &source.id,
                master_key,
                ApplyOptions {
                    base_revision: Some(base_revision),
                    prune: source.prune,
                    dry_run: false,
                },
            )
            .await
    }
    .await;
    if let Err(error) = &result {
        if let Err(record_error) = catalog
            .record_apply_failure(&source.id, actor_label, error)
            .await
        {
            warn!(
                source_id = %source.id,
                error = %record_error,
                "failed to record inventory apply error"
            );
        }
    }
    result
}

pub async fn apply_startup_sources(
    catalog: &Catalog,
    master_key: &MasterKey,
    sources: &[InventorySourceConfig],
) {
    for source in sources {
        match apply_source(catalog, master_key, source, "watcher-startup").await {
            Ok(summary) => info!(
                source_id = %source.id,
                revision = summary.new_revision,
                "inventory source applied at startup"
            ),
            Err(error) => warn!(
                source_id = %source.id,
                code = %error.code,
                path = ?error.path,
                "inventory source rejected; retaining the previous catalog"
            ),
        }
    }
}

pub async fn watch_sources(
    catalog: Catalog,
    master_key: Arc<MasterKey>,
    sources: Vec<InventorySourceConfig>,
) -> anyhow::Result<()> {
    let sources: Vec<_> = sources.into_iter().filter(|source| source.watch).collect();
    if sources.is_empty() {
        std::future::pending::<()>().await;
        return Ok(());
    }

    let mut stable_candidates: BTreeMap<String, ScopeSnapshot> = BTreeMap::new();
    let mut applied_snapshots: BTreeMap<String, ScopeSnapshot> = BTreeMap::new();
    for source in &sources {
        if let Ok(snapshot) = manifest_io::scope_snapshot(&[PathBuf::from(&source.path)]) {
            stable_candidates.insert(source.id.clone(), snapshot.clone());
            applied_snapshots.insert(source.id.clone(), snapshot);
        }
    }

    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        for source in &sources {
            let snapshot = match manifest_io::scope_snapshot(&[PathBuf::from(&source.path)]) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    stable_candidates.remove(&source.id);
                    if let Err(record_error) = catalog
                        .record_apply_failure(&source.id, "watcher", &error)
                        .await
                    {
                        warn!(source_id = %source.id, error = %record_error, "failed to record watcher scan error");
                    }
                    continue;
                }
            };
            if stable_candidates.get(&source.id) != Some(&snapshot) {
                stable_candidates.insert(source.id.clone(), snapshot);
                continue;
            }
            if applied_snapshots.get(&source.id) == Some(&snapshot) {
                continue;
            }
            match apply_source(&catalog, &master_key, source, "watcher").await {
                Ok(summary) => info!(
                    source_id = %source.id,
                    revision = summary.new_revision,
                    "stable inventory source change applied"
                ),
                Err(error) => warn!(
                    source_id = %source.id,
                    code = %error.code,
                    path = ?error.path,
                    "inventory source change rejected; retaining the previous catalog"
                ),
            }
            applied_snapshots.insert(source.id.clone(), snapshot);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[tokio::test]
    async fn invalid_reload_records_failure_without_changing_catalog() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("resources.yaml");
        fs::write(&path, "api_version: hop/v1alpha1\nassets: [invalid]").unwrap();
        let catalog = Catalog::in_memory().await.unwrap();
        let source = InventorySourceConfig {
            id: "home".to_string(),
            path: path.display().to_string(),
            watch: true,
            prune: false,
        };

        let error = apply_source(&catalog, &MasterKey::generate(), &source, "test")
            .await
            .unwrap_err();
        assert_eq!(catalog.revision().await.unwrap(), 0);
        let status = catalog.status().await.unwrap();
        assert_eq!(status.sources.len(), 1);
        assert_eq!(
            status.sources[0].last_error_code.as_deref(),
            Some(error.code.to_string().as_str())
        );
        assert!(catalog.list_assets().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn watcher_waits_for_a_stable_scope_and_keeps_last_valid_catalog() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("resources.yaml");
        fs::write(
            &path,
            "api_version: hop/v1alpha1\nassets:\n  one: { type: tcp, host: 192.0.2.1, port: 80 }\n  two: { type: tcp, host: 192.0.2.2, port: 80 }\n",
        )
        .unwrap();
        let catalog = Catalog::in_memory().await.unwrap();
        let master_key = Arc::new(MasterKey::generate());
        let source = InventorySourceConfig {
            id: "home".to_string(),
            path: path.display().to_string(),
            watch: true,
            prune: false,
        };
        apply_source(&catalog, &master_key, &source, "test")
            .await
            .unwrap();
        let watcher = tokio::spawn(watch_sources(catalog.clone(), master_key, vec![source]));
        tokio::time::sleep(Duration::from_millis(100)).await;
        fs::write(
            &path,
            "api_version: hop/v1alpha1\nassets:\n  one: { type: tcp, host: 192.0.2.10, port: 80 }\n",
        )
        .unwrap();
        wait_for_revision(&catalog, 2).await;
        assert_eq!(catalog.list_assets().await.unwrap().len(), 2);
        assert_eq!(catalog.status().await.unwrap().orphans.len(), 1);

        let revision = catalog.revision().await.unwrap();
        fs::write(&path, "api_version: hop/v1alpha1\nassets: [invalid]").unwrap();
        wait_for_source_error(&catalog).await;
        assert_eq!(catalog.revision().await.unwrap(), revision);
        assert_eq!(catalog.list_assets().await.unwrap().len(), 2);
        watcher.abort();
    }

    async fn wait_for_revision(catalog: &Catalog, expected: i64) {
        for _ in 0..60 {
            if catalog.revision().await.unwrap() >= expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("watcher did not reach catalog revision {expected}");
    }

    async fn wait_for_source_error(catalog: &Catalog) {
        for _ in 0..60 {
            if catalog
                .status()
                .await
                .unwrap()
                .sources
                .first()
                .and_then(|source| source.last_error_code.as_ref())
                .is_some()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("watcher did not record invalid source status");
    }
}
