use std::{fs, path::PathBuf};

use hop_core::{CatalogError, CatalogErrorCode, Manifest};

pub fn load_manifest_scope(inputs: &[PathBuf]) -> Result<Manifest, CatalogError> {
    let paths = expand_manifest_scope(inputs)?;
    let manifests = paths
        .into_iter()
        .map(|path| {
            let raw = fs::read_to_string(&path).map_err(|_| {
                CatalogError::new(
                    CatalogErrorCode::SourceScanIncomplete,
                    Some(path.display().to_string()),
                    "unable to read complete manifest scope",
                )
            })?;
            match path.extension().and_then(|extension| extension.to_str()) {
                Some("yaml" | "yml") => Manifest::from_yaml(&raw),
                Some("toml") => Manifest::from_toml(&raw),
                _ => Err(CatalogError::new(
                    CatalogErrorCode::ApplyFailed,
                    Some(path.display().to_string()),
                    "manifest file extension must be .yaml, .yml or .toml",
                )),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    Manifest::merge(manifests)
}

fn expand_manifest_scope(inputs: &[PathBuf]) -> Result<Vec<PathBuf>, CatalogError> {
    let mut paths = Vec::new();
    for input in inputs {
        let pattern = input.to_string_lossy();
        if pattern
            .chars()
            .any(|character| matches!(character, '*' | '?' | '['))
        {
            let matches = glob::glob(&pattern).map_err(|_| {
                CatalogError::new(
                    CatalogErrorCode::SourceScanIncomplete,
                    Some(pattern.to_string()),
                    "invalid manifest glob pattern",
                )
            })?;
            let mut matched = false;
            for entry in matches {
                let path = entry.map_err(|_| {
                    CatalogError::new(
                        CatalogErrorCode::SourceScanIncomplete,
                        Some(pattern.to_string()),
                        "manifest glob scan failed",
                    )
                })?;
                matched = true;
                paths.push(path);
            }
            if !matched {
                return Err(CatalogError::new(
                    CatalogErrorCode::SourceScanIncomplete,
                    Some(pattern.to_string()),
                    "manifest glob matched no files",
                ));
            }
        } else {
            paths.push(input.clone());
        }
    }
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return Err(CatalogError::new(
            CatalogErrorCode::SourceScanIncomplete,
            None,
            "manifest scope is empty",
        ));
    }
    Ok(paths)
}
